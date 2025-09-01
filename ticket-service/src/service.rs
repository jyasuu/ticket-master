use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaProducer, KafkaConsumer,
    CreateEvent, CreateReservation, Reservation, AreaStatus, Area, Seat,
    ReservationType, Topics, Stores, event_area_key, ProcessingContext
};
use std::sync::Arc;
use crate::{CreateEventRequest, CreateReservationRequest};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tracing::{info, error, warn, debug, span, Level, Instrument};
use std::time::Instant;

pub struct TicketService {
    producer: KafkaProducer,
    consumer: Arc<KafkaConsumer>,
    context: Arc<ProcessingContext>,
}

impl Clone for TicketService {
    fn clone(&self) -> Self {
        Self {
            producer: self.producer.clone(),
            consumer: Arc::clone(&self.consumer),
            context: Arc::clone(&self.context),
        }
    }
}

impl TicketService {
    pub async fn new(config: ServiceConfig) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        let producer = KafkaProducer::new(kafka_config.clone())?;
        let consumer = KafkaConsumer::new(kafka_config)?;

        // Subscribe to state topics to populate local stores (matching Java implementation)
        consumer.subscribe(&[
            Topics::STATE_USER_RESERVATION,  // To populate reservation store
        ])?;

        // Initialize state stores for querying
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        
        // Add RocksDB stores for reading state
        context.add_rocksdb_store(Stores::AREA_STATUS.to_string(), "area-status")?;
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;

        Ok(Self { 
            producer,
            consumer: Arc::new(consumer),
            context: Arc::new(context),
        })
    }

    pub async fn run_consumer(&self) -> Result<()> {
        info!("Starting Ticket Service consumer for state synchronization...");
        
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    info!("Received shutdown signal for consumer");
                    break;
                }
                
                // Process state messages
                message_result = self.consumer.recv_message(std::time::Duration::from_millis(100)) => {
                    match message_result? {
                        Some(message) => {
                            if let Err(e) = self.process_state_message(&message).await {
                                tracing::error!("Error processing state message: {}", e);
                            } else {
                                // Commit the message after successful processing
                                if let Err(e) = self.consumer.commit_message(&message) {
                                    tracing::error!("Error committing message: {}", e);
                                }
                            }
                        }
                        None => {
                            // No message received (timeout)
                            continue;
                        }
                    }
                }
            }
        }

        info!("Ticket Service consumer shutting down...");
        Ok(())
    }

    async fn process_state_message(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        info!(
            topic = %message.topic,
            message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
            partition = message.partition,
            offset = message.offset,
            message_size_bytes = message.payload.as_ref().map(|p| p.len()).unwrap_or(0),
            "📥 KAFKA_CONSUME: Received state message from Kafka (Ticket Service)"
        );

        let process_start = Instant::now();
        let result = match message.topic.as_str() {
            Topics::STATE_USER_RESERVATION => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_reservation_state_update (Local Store Sync)"
                );
                self.handle_reservation_state_update(message).await
            }
            _ => {
                warn!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "❌ KAFKA_CONSUME_ERROR: Unknown state topic"
                );
                Ok(())
            }
        };

        match &result {
            Ok(_) => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    processing_duration_ms = process_start.elapsed().as_millis(),
                    "✅ KAFKA_CONSUME_SUCCESS: State message processed successfully"
                );
            }
            Err(e) => {
                error!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    processing_duration_ms = process_start.elapsed().as_millis(),
                    error = %e,
                    "❌ KAFKA_CONSUME_ERROR: State message processing failed"
                );
            }
        }

        result
    }

    async fn handle_reservation_state_update(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "reservation_state_sync",
            message_key = %message.key.as_ref().unwrap_or(&"unknown".to_string()),
            topic = %message.topic,
            partition = message.partition,
            offset = message.offset
        );
        
        async move {
            let reservation_id = message.key.as_ref()
                .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
            
            let reservation: Reservation = message.deserialize_value()?;
            
            info!(
                reservation_id = %reservation_id,
                user_id = %reservation.user_id,
                event_id = %reservation.event_id,
                area_id = %reservation.area_id,
                reservation_state = ?reservation.state,
                seats_count = reservation.seats.len(),
                "🔄 FLOW_START: Updating local reservation store (State Sync)"
            );

            // Get reservation store and update it
            if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
                let store_start = Instant::now();
                store.put(reservation_id, &reservation)?;
                
                info!(
                    reservation_id = %reservation_id,
                    reservation_state = ?reservation.state,
                    store_duration_ms = store_start.elapsed().as_millis(),
                    total_duration_ms = start_time.elapsed().as_millis(),
                    "💾 FLOW_END: Successfully updated reservation in local store"
                );
            } else {
                warn!(
                    reservation_id = %reservation_id,
                    "❌ FLOW_ERROR: Reservation store not available for update"
                );
            }

            Ok(())
        }.instrument(span).await
    }

    pub async fn create_event(&self, request: CreateEventRequest) -> Result<String> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "api_create_event",
            event_name = %request.event_name,
            artist = %request.artist,
            areas_count = request.areas.len()
        );
        
        async move {
            info!(
                event_name = %request.event_name,
                artist = %request.artist,
                areas_count = request.areas.len(),
                "🎫 API_START: Creating event via REST API"
            );

            // Parse timestamps
            let parse_start = Instant::now();
            let reservation_opening_time = parse_timestamp(&request.reservation_opening_time)?;
            let reservation_closing_time = parse_timestamp(&request.reservation_closing_time)?;
            let event_start_time = parse_timestamp(&request.event_start_time)?;
            let event_end_time = parse_timestamp(&request.event_end_time)?;
            
            debug!(
                parse_duration_ms = parse_start.elapsed().as_millis(),
                "📅 API_STEP: Timestamps parsed successfully"
            );

            // Convert areas
            let convert_start = Instant::now();
            let total_seats: i32 = request.areas.iter().map(|a| a.row_count * a.col_count).sum();
            let areas: Vec<Area> = request.areas.into_iter().map(|area_req| {
                Area {
                    area_id: area_req.area_id,
                    price: area_req.price,
                    row_count: area_req.row_count,
                    col_count: area_req.col_count,
                }
            }).collect();
            
            debug!(
                areas_count = areas.len(),
                total_seats = total_seats,
                convert_duration_ms = convert_start.elapsed().as_millis(),
                "🏟️ API_STEP: Areas converted successfully"
            );

            let create_event = CreateEvent {
                artist: request.artist,
                event_name: request.event_name.clone(),
                reservation_opening_time,
                reservation_closing_time,
                event_start_time,
                event_end_time,
                areas,
            };

            // Send create event command
            let publish_start = Instant::now();
            info!(
                topic = Topics::COMMAND_EVENT_CREATE_EVENT,
                message_key = %request.event_name,
                event_name = %request.event_name,
                areas_count = create_event.areas.len(),
                total_seats = total_seats,
                "📤 KAFKA_PRODUCE_START: Publishing create event command to COMMAND_EVENT_CREATE_EVENT"
            );
            
            self.producer.send(
                Topics::COMMAND_EVENT_CREATE_EVENT,
                &request.event_name,
                &create_event,
            ).await?;

            info!(
                topic = Topics::COMMAND_EVENT_CREATE_EVENT,
                message_key = %request.event_name,
                duration_ms = publish_start.elapsed().as_millis(),
                message_size_bytes = std::mem::size_of_val(&create_event),
                "📤 KAFKA_PRODUCE_SUCCESS: Event creation command published to COMMAND_EVENT_CREATE_EVENT"
            );

            info!(
                event_name = %request.event_name,
                total_duration_ms = start_time.elapsed().as_millis(),
                "✅ API_END: Event creation command sent successfully"
            );
            
            Ok(request.event_name)
        }.instrument(span).await
    }

    pub async fn create_reservation(&self, request: CreateReservationRequest) -> Result<String> {
        let reservation_id = Uuid::new_v4().to_string();
        let start_time = Instant::now();
        let span = span!(Level::INFO, "api_create_reservation",
            reservation_id = %reservation_id,
            user_id = %request.user_id,
            event_id = %request.event_id,
            area_id = %request.area_id,
            num_seats = request.num_of_seats,
            reservation_type = %request.reservation_type
        );
        
        async move {
            info!(
                reservation_id = %reservation_id,
                user_id = %request.user_id,
                event_id = %request.event_id,
                area_id = %request.area_id,
                num_seats = request.num_of_seats,
                reservation_type = %request.reservation_type,
                "🎟️ API_START: Creating reservation via REST API"
            );

            // Parse reservation type
            let parse_start = Instant::now();
            let reservation_type = match request.reservation_type.to_lowercase().as_str() {
                "self_pick" | "selfpick" => ReservationType::SelfPick,
                "random" => ReservationType::Random,
                _ => return Err(TicketMasterError::InvalidArgument(
                    format!("Invalid reservation type: {}", request.reservation_type)
                )),
            };
            
            debug!(
                reservation_type = ?reservation_type,
                parse_duration_ms = parse_start.elapsed().as_millis(),
                "🎯 API_STEP: Reservation type parsed successfully"
            );

            // Convert seats if provided
            let convert_start = Instant::now();
            let seats: Vec<Seat> = request.seats.unwrap_or_default().into_iter().map(|seat_req| {
                Seat {
                    row: seat_req.row,
                    col: seat_req.col,
                }
            }).collect();
            
            debug!(
                seats_provided = seats.len(),
                convert_duration_ms = convert_start.elapsed().as_millis(),
                "💺 API_STEP: Seats converted successfully"
            );

            let create_reservation = CreateReservation {
                reservation_id: reservation_id.clone(),
                user_id: request.user_id.clone(),
                event_id: request.event_id.clone(),
                area_id: request.area_id.clone(),
                num_of_seats: request.num_of_seats,
                num_of_seat: 0, // This seems to be used for numbering, defaulting to 0
                reservation_type,
                seats,
            };

            // Send create reservation command
            let publish_start = Instant::now();
            info!(
                topic = Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
                message_key = %reservation_id,
                reservation_id = %reservation_id,
                user_id = %request.user_id,
                event_id = %request.event_id,
                area_id = %request.area_id,
                num_seats = request.num_of_seats,
                reservation_type = ?create_reservation.reservation_type,
                "📤 KAFKA_PRODUCE_START: Publishing create reservation command to COMMAND_RESERVATION_CREATE_RESERVATION"
            );
            
            self.producer.send(
                Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
                &reservation_id,
                &create_reservation,
            ).await?;

            info!(
                topic = Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
                message_key = %reservation_id,
                duration_ms = publish_start.elapsed().as_millis(),
                message_size_bytes = std::mem::size_of_val(&create_reservation),
                "📤 KAFKA_PRODUCE_SUCCESS: Reservation creation command published to COMMAND_RESERVATION_CREATE_RESERVATION"
            );

            info!(
                reservation_id = %reservation_id,
                total_duration_ms = start_time.elapsed().as_millis(),
                "✅ API_END: Reservation creation command sent successfully"
            );
            
            Ok(reservation_id)
        }.instrument(span).await
    }

    pub async fn get_area_status(&self, event_name: &str, area_id: &str) -> Result<Option<AreaStatus>> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "api_get_area_status",
            event_name = %event_name,
            area_id = %area_id
        );
        
        async move {
            info!(
                event_name = %event_name,
                area_id = %area_id,
                "🔍 API_START: Getting area status via REST API"
            );
            
            let key = event_area_key(event_name, area_id);
            
            if let Some(store) = self.context.get_rocksdb_store(Stores::AREA_STATUS) {
                let lookup_start = Instant::now();
                match store.get::<AreaStatus>(&key)? {
                    Some(area_status) => {
                        info!(
                            event_area_key = %key,
                            available_seats = area_status.available_seats,
                            total_seats = area_status.row_count * area_status.col_count,
                            lookup_duration_ms = lookup_start.elapsed().as_millis(),
                            total_duration_ms = start_time.elapsed().as_millis(),
                            "✅ API_END: Area status found in local store"
                        );
                        Ok(Some(area_status))
                    }
                    None => {
                        info!(
                            event_area_key = %key,
                            lookup_duration_ms = lookup_start.elapsed().as_millis(),
                            total_duration_ms = start_time.elapsed().as_millis(),
                            "❌ API_END: No area status found for key"
                        );
                        Ok(None)
                    }
                }
            } else {
                warn!(
                    event_name = %event_name,
                    area_id = %area_id,
                    total_duration_ms = start_time.elapsed().as_millis(),
                    "❌ API_ERROR: Area status store not available"
                );
                Ok(None)
            }
        }.instrument(span).await
    }

    pub async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "api_get_reservation",
            reservation_id = %reservation_id
        );
        
        async move {
            info!(
                reservation_id = %reservation_id,
                "🔍 API_START: Getting reservation via REST API"
            );
            
            if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
                let lookup_start = Instant::now();
                match store.get::<Reservation>(reservation_id)? {
                    Some(reservation) => {
                        info!(
                            reservation_id = %reservation_id,
                            user_id = %reservation.user_id,
                            event_id = %reservation.event_id,
                            area_id = %reservation.area_id,
                            reservation_state = ?reservation.state,
                            seats_count = reservation.seats.len(),
                            lookup_duration_ms = lookup_start.elapsed().as_millis(),
                            total_duration_ms = start_time.elapsed().as_millis(),
                            "✅ API_END: Reservation found in local store"
                        );
                        Ok(Some(reservation))
                    }
                    None => {
                        info!(
                            reservation_id = %reservation_id,
                            lookup_duration_ms = lookup_start.elapsed().as_millis(),
                            total_duration_ms = start_time.elapsed().as_millis(),
                            "❌ API_END: No reservation found for id"
                        );
                        Ok(None)
                    }
                }
            } else {
                warn!(
                    reservation_id = %reservation_id,
                    total_duration_ms = start_time.elapsed().as_millis(),
                    "❌ API_ERROR: Reservation store not available"
                );
                Ok(None)
            }
        }.instrument(span).await
    }
}

fn parse_timestamp(timestamp_str: &str) -> Result<DateTime<Utc>> {
    // Try parsing as ISO 8601 format first
    if let Ok(dt) = DateTime::parse_from_rfc3339(timestamp_str) {
        return Ok(dt.with_timezone(&Utc));
    }
    
    // Try parsing as timestamp millis
    if let Ok(millis) = timestamp_str.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp_millis(millis) {
            return Ok(dt);
        }
    }
    
    Err(TicketMasterError::InvalidArgument(
        format!("Invalid timestamp format: {}", timestamp_str)
    ))
}
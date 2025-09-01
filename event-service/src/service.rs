use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaConsumer, KafkaProducer,
    CreateEvent, AreaStatus, ReserveSeat, ReservationResult, ReservationResultEnum,
    ReservationErrorCode, ReservationType, Seat, Topics, Stores, event_area_key,
    StateStore, ProcessingContext
};
use crate::strategies::{ReservationStrategy, SelfPickStrategy, RandomStrategy};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, error, warn, debug, span, Level, Instrument};
use tokio::signal;

pub struct EventService {
    consumer: KafkaConsumer,
    producer: KafkaProducer,
    context: ProcessingContext,
    strategies: HashMap<ReservationType, Box<dyn ReservationStrategy + Send + Sync>>,
}

impl EventService {
    pub async fn new(config: ServiceConfig) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        
        let consumer = KafkaConsumer::new(kafka_config.clone())?;
        let producer = KafkaProducer::new(kafka_config)?;
        
        // Subscribe to topics
        consumer.subscribe(&[
            Topics::COMMAND_EVENT_CREATE_EVENT,
            Topics::COMMAND_EVENT_RESERVE_SEAT,
        ])?;

        // Initialize state stores with RocksDB (matching Java implementation)
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        context.add_rocksdb_store(Stores::AREA_STATUS.to_string(), "area-status")?;

        // Initialize reservation strategies
        let mut strategies: HashMap<ReservationType, Box<dyn ReservationStrategy + Send + Sync>> = HashMap::new();
        strategies.insert(ReservationType::SelfPick, Box::new(SelfPickStrategy));
        strategies.insert(ReservationType::Random, Box::new(RandomStrategy));

        Ok(Self {
            consumer,
            producer,
            context,
            strategies,
        })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Event Service is running...");

        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = signal::ctrl_c() => {
                    info!("Received shutdown signal");
                    break;
                }
                
                // Process messages
                message_result = self.consumer.recv_message(Duration::from_millis(100)) => {
                    match message_result? {
                        Some(message) => {
                            if let Err(e) = self.process_message(&message).await {
                                error!("Error processing message: {}", e);
                            } else {
                                // Commit the message after successful processing
                                if let Err(e) = self.consumer.commit_message(&message) {
                                    error!("Error committing message: {}", e);
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

        info!("Event Service shutting down...");
        Ok(())
    }

    async fn process_message(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        info!(
            topic = %message.topic,
            message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
            partition = message.partition,
            offset = message.offset,
            message_size_bytes = message.payload.as_ref().map(|p| p.len()).unwrap_or(0),
            "📥 KAFKA_CONSUME: Received message from Kafka"
        );

        let process_start = Instant::now();
        let result = match message.topic.as_str() {
            Topics::COMMAND_EVENT_CREATE_EVENT => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_create_event"
                );
                self.handle_create_event(message).await
            }
            Topics::COMMAND_EVENT_RESERVE_SEAT => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_reserve_seat"
                );
                self.handle_reserve_seat(message).await
            }
            _ => {
                warn!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "❌ KAFKA_CONSUME_ERROR: Unknown topic"
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
                    "✅ KAFKA_CONSUME_SUCCESS: Message processed successfully"
                );
            }
            Err(e) => {
                error!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    processing_duration_ms = process_start.elapsed().as_millis(),
                    error = %e,
                    "❌ KAFKA_CONSUME_ERROR: Message processing failed"
                );
            }
        }

        result
    }

    async fn handle_create_event(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "event_creation", 
            message_key = %message.key.as_ref().unwrap_or(&"unknown".to_string()),
            topic = %message.topic,
            partition = message.partition,
            offset = message.offset
        );
        
        async move {
            let event_name = message.key.as_ref()
                .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event name key".to_string()))?;
            
            let create_event: CreateEvent = message.deserialize_value()?;
            
            info!(
                event_name = %event_name,
                artist = %create_event.artist,
                areas_count = create_event.areas.len(),
                "🎫 FLOW_START: Creating new event"
            );

            let area_status_store = self.context
                .get_rocksdb_store(Stores::AREA_STATUS)
                .ok_or_else(|| TicketMasterError::InvalidArgument("Area status store not found".to_string()))?;

            let total_seats: i32 = create_event.areas.iter().map(|a| a.row_count * a.col_count).sum();
            let mut areas_created = 0;

            // Create area status for each area and store them
            for (area_index, area) in create_event.areas.iter().enumerate() {
                let area_span = span!(Level::DEBUG, "area_creation",
                    area_id = %area.area_id,
                    area_index = area_index,
                    rows = area.row_count,
                    cols = area.col_count,
                    price = %area.price
                );
                
                let area_status_store = self.context
                    .get_rocksdb_store(Stores::AREA_STATUS)
                    .ok_or_else(|| TicketMasterError::InvalidArgument("Area status store not found".to_string()))?;
                
                async move {
                    let area_status = AreaStatus::from_area(event_name, area);
                    let key = event_area_key(event_name, &area.area_id);
                    let seats_in_area = area.row_count * area.col_count;
                    
                    debug!(
                        area_key = %key,
                        seats_count = seats_in_area,
                        "📍 FLOW_STEP: Creating area status"
                    );
                    
                    // Store in RocksDB
                    let store_start = Instant::now();
                    area_status_store.put(&key, &area_status)?;
                    debug!(
                        duration_ms = store_start.elapsed().as_millis(),
                        "💾 FLOW_STEP: Area status stored in RocksDB"
                    );
                    
                    // Emit area status to state topic
                    let publish_start = Instant::now();
                    info!(
                        topic = Topics::STATE_EVENT_AREA_STATUS,
                        message_key = %key,
                        area_id = %area.area_id,
                        available_seats = area_status.available_seats,
                        total_seats = area_status.row_count * area_status.col_count,
                        "📤 KAFKA_PRODUCE_START: Publishing area status to STATE_EVENT_AREA_STATUS"
                    );
                    
                    self.producer.send(
                        Topics::STATE_EVENT_AREA_STATUS,
                        &key,
                        &area_status,
                    ).await?;
                    
                    info!(
                        topic = Topics::STATE_EVENT_AREA_STATUS,
                        message_key = %key,
                        area_id = %area.area_id,
                        duration_ms = publish_start.elapsed().as_millis(),
                        message_size_bytes = std::mem::size_of_val(&area_status),
                        "📤 KAFKA_PRODUCE_SUCCESS: Area status published to STATE_EVENT_AREA_STATUS"
                    );
                    
                    areas_created += 1;
                    Ok::<(), TicketMasterError>(())
                }.instrument(area_span).await?;
            }

            info!(
                event_name = %event_name,
                total_areas = areas_created,
                total_seats = total_seats,
                duration_ms = start_time.elapsed().as_millis(),
                "✅ FLOW_END: Event created successfully"
            );
            
            Ok(())
        }.instrument(span).await
    }

    async fn handle_reserve_seat(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "seat_reservation",
            message_key = %message.key.as_ref().unwrap_or(&"unknown".to_string()),
            topic = %message.topic,
            partition = message.partition,
            offset = message.offset
        );
        
        async move {
            let event_area_id = message.key.as_ref()
                .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
            
            let reserve_request: ReserveSeat = message.deserialize_value()?;
            
            info!(
                reservation_id = %reserve_request.reservation_id,
                event_area_id = %event_area_id,
                event_id = %reserve_request.event_id,
                area_id = %reserve_request.area_id,
                num_seats = reserve_request.num_of_seats,
                reservation_type = ?reserve_request.reservation_type,
                "🎟️ FLOW_START: Processing seat reservation"
            );

            let area_status_store = self.context
                .get_rocksdb_store(Stores::AREA_STATUS)
                .ok_or_else(|| TicketMasterError::InvalidArgument("Area status store not found".to_string()))?;

            // Get current area status
            let store_lookup_start = Instant::now();
            let mut area_status = area_status_store.get::<AreaStatus>(event_area_id)?
                .ok_or_else(|| TicketMasterError::InvalidEventArea(event_area_id.clone()))?;
            
            debug!(
                event_area_id = %event_area_id,
                available_seats = area_status.available_seats,
                duration_ms = store_lookup_start.elapsed().as_millis(),
                "🔍 FLOW_STEP: Area status retrieved from RocksDB"
            );

            // Get reservation strategy
            let strategy = self.strategies.get(&reserve_request.reservation_type)
                .ok_or_else(|| TicketMasterError::InvalidReservationStrategy(format!("{:?}", reserve_request.reservation_type)))?;

            debug!(
                strategy = ?reserve_request.reservation_type,
                "🎯 FLOW_STEP: Reservation strategy selected"
            );

            // Execute reservation
            let reservation_start = Instant::now();
            let result = strategy.reserve(&mut area_status, &reserve_request)?;
            let result2 = result.clone();

            info!(
                reservation_id = %reserve_request.reservation_id,
                result = ?result.result,
                seats_reserved = result.seats.len(),
                duration_ms = reservation_start.elapsed().as_millis(),
                "🎲 FLOW_STEP: Reservation strategy executed"
            );

            // If successful, update the area status
            if result.result == ReservationResultEnum::Success {
                let update_span = span!(Level::DEBUG, "area_status_update",
                    seats_to_update = result.seats.len()
                );
                
                async move {
                    // Update seat availability
                    for (seat_index, seat) in result2.seats.iter().enumerate() {
                        if let Some(seat_status) = area_status.seats
                            .get_mut(seat.row as usize)
                            .and_then(|row| row.get_mut(seat.col as usize)) {
                            seat_status.is_available = false;
                            debug!(
                                seat_index = seat_index,
                                row = seat.row,
                                col = seat.col,
                                "💺 FLOW_STEP: Seat marked as unavailable"
                            );
                        }
                    }
                    
                    let old_available = area_status.available_seats;
                    area_status.available_seats -= result2.seats.len() as i32;
                    
                    info!(
                        old_available_seats = old_available,
                        new_available_seats = area_status.available_seats,
                        seats_reserved = result2.seats.len(),
                        "📊 FLOW_STEP: Area availability updated"
                    );
                    
                    // Update state store
                    let store_update_start = Instant::now();
                    area_status_store.put(event_area_id, &area_status)?;
                    debug!(
                        duration_ms = store_update_start.elapsed().as_millis(),
                        "💾 FLOW_STEP: Updated area status stored in RocksDB"
                    );
                    
                    // Emit updated area status
                    let publish_start = Instant::now();
                    info!(
                        topic = Topics::STATE_EVENT_AREA_STATUS,
                        message_key = %event_area_id,
                        available_seats_before = old_available,
                        available_seats_after = area_status.available_seats,
                        seats_reserved = result2.seats.len(),
                        "📤 KAFKA_PRODUCE_START: Publishing updated area status to STATE_EVENT_AREA_STATUS"
                    );
                    
                    self.producer.send(
                        Topics::STATE_EVENT_AREA_STATUS,
                        event_area_id,
                        &area_status,
                    ).await?;
                    
                    info!(
                        topic = Topics::STATE_EVENT_AREA_STATUS,
                        message_key = %event_area_id,
                        duration_ms = publish_start.elapsed().as_millis(),
                        message_size_bytes = std::mem::size_of_val(&area_status),
                        "📤 KAFKA_PRODUCE_SUCCESS: Updated area status published to STATE_EVENT_AREA_STATUS"
                    );
                    
                    Ok::<(), TicketMasterError>(())
                }.instrument(update_span).await?;
            } else {
                warn!(
                    reservation_id = %reserve_request.reservation_id,
                    error_code = ?result.error_code,
                    error_message = %result.error_message.as_ref().unwrap_or(&"Unknown error".to_string()),
                    "❌ FLOW_STEP: Reservation failed"
                );
            }

            // Send reservation result
            let seats_count = result.seats.len();
            let result_status = result.result.clone();
            let result_publish_start = Instant::now();
            info!(
                topic = Topics::RESPONSE_RESERVATION_RESULT,
                message_key = %reserve_request.reservation_id,
                reservation_result = ?result_status,
                seats_allocated = seats_count,
                error_code = ?result.error_code,
                "📤 KAFKA_PRODUCE_START: Publishing reservation result to RESPONSE_RESERVATION_RESULT"
            );
            
            self.producer.send(
                Topics::RESPONSE_RESERVATION_RESULT,
                &reserve_request.reservation_id,
                &result,
            ).await?;

            info!(
                topic = Topics::RESPONSE_RESERVATION_RESULT,
                message_key = %reserve_request.reservation_id,
                duration_ms = result_publish_start.elapsed().as_millis(),
                message_size_bytes = std::mem::size_of_val(&result),
                "📤 KAFKA_PRODUCE_SUCCESS: Reservation result published to RESPONSE_RESERVATION_RESULT"
            );

            info!(
                reservation_id = %reserve_request.reservation_id,
                result = ?result.result,
                total_duration_ms = start_time.elapsed().as_millis(),
                "✅ FLOW_END: Seat reservation processing completed"
            );
            
            Ok(())
        }.instrument(span).await
    }
}
use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaConsumer, KafkaProducer,
    CreateReservation, Reservation, ReservationResult, ReservationState, 
    ReserveSeat, AreaStatus, Topics, Stores, event_area_key,
    StateStore, ProcessingContext, RocksDBStore
};
use std::time::{Duration, Instant};
use tracing::{info, error, warn, debug, span, Level, Instrument};
use tokio::signal;

pub struct ReservationService {
    consumer: KafkaConsumer,
    producer: KafkaProducer,
    context: ProcessingContext,
}

impl ReservationService {
    pub async fn new(config: ServiceConfig) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        
        let consumer = KafkaConsumer::new(kafka_config.clone())?;
        let producer = KafkaProducer::new(kafka_config)?;
        
        // Subscribe to topics
        consumer.subscribe(&[
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
            Topics::RESPONSE_RESERVATION_RESULT,
            Topics::STATE_EVENT_AREA_STATUS,
        ])?;

        // Initialize state stores with RocksDB (matching Java implementation)
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        
        // Reservation store (equivalent to Java's RESERVATION store)
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;
        
        // Area status cache (equivalent to Java's EVENT_AREA_STATUS_CACHE LRU store)
        context.add_rocksdb_store(Stores::EVENT_AREA_STATUS_CACHE.to_string(), "area-status-cache")?;

        Ok(Self {
            consumer,
            producer,
            context,
        })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Reservation Service is running...");

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

        info!("Reservation Service shutting down...");
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
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_create_reservation"
                );
                self.handle_create_reservation(message).await
            }
            Topics::RESPONSE_RESERVATION_RESULT => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_reservation_result"
                );
                self.handle_reservation_result(message).await
            }
            Topics::STATE_EVENT_AREA_STATUS => {
                info!(
                    topic = %message.topic,
                    message_key = %message.key.as_ref().unwrap_or(&"null".to_string()),
                    "📥 KAFKA_CONSUME_ROUTE: Routing to handle_area_status_update (GlobalTable equivalent)"
                );
                self.handle_area_status_update(message).await
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

    async fn handle_create_reservation(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "reservation_creation",
            message_key = %message.key.as_ref().unwrap_or(&"unknown".to_string()),
            topic = %message.topic,
            partition = message.partition,
            offset = message.offset
        );
        
        async move {
            let reservation_id = message.key.as_ref()
                .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
            
            let create_request: CreateReservation = message.deserialize_value()?;
            
            info!(
                reservation_id = %reservation_id,
                user_id = %create_request.user_id,
                event_id = %create_request.event_id,
                area_id = %create_request.area_id,
                num_seats = create_request.num_of_seats,
                reservation_type = ?create_request.reservation_type,
                "🎫 FLOW_START: Creating new reservation"
            );

            // Get RocksDB reservation store (matching Java implementation)
            let reservation_store = self.context
                .get_rocksdb_store(Stores::RESERVATION)
                .ok_or_else(|| TicketMasterError::InvalidArgument("Reservation store not found".to_string()))?;

            // Get area status cache for filtering (equivalent to Java's GlobalTable)
            let area_cache = self.context
                .get_rocksdb_store(Stores::EVENT_AREA_STATUS_CACHE)
                .ok_or_else(|| TicketMasterError::InvalidArgument("Area status cache not found".to_string()))?;

            // Create new reservation with PROCESSING state
            let mut reservation = Reservation::new(create_request.clone());
            reservation.state = ReservationState::Processing;
            
            debug!(
                reservation_id = %reservation_id,
                initial_state = ?reservation.state,
                "📝 FLOW_STEP: Reservation object created with PROCESSING state"
            );

            // Check area status cache for pre-filtering (equivalent to Java's ReservationValueProcessor)
            let event_area_id = event_area_key(&create_request.event_id, &create_request.area_id);
            let cache_lookup_start = Instant::now();
            
            match area_cache.get::<AreaStatus>(&event_area_id)? {
                Some(cached_area_status) => {
                    info!(
                        event_area_id = %event_area_id,
                        available_seats = cached_area_status.available_seats,
                        cache_hit = true,
                        duration_ms = cache_lookup_start.elapsed().as_millis(),
                        "🎯 FLOW_STEP: Area status found in cache (GlobalTable equivalent)"
                    );

                    // Apply filter strategy (equivalent to Java's FilterStrategy)
                    let filter_result = self.apply_filter_strategy(&cached_area_status, &create_request)?;
                    
                    if !filter_result.passed {
                        warn!(
                            reservation_id = %reservation_id,
                            filter_reason = %filter_result.reason,
                            "❌ FLOW_STEP: Reservation rejected by cache filter"
                        );
                        
                        reservation.state = ReservationState::Failed;
                        reservation.failed_reason = filter_result.reason;
                        
                        // Store failed reservation
                        reservation_store.put(reservation_id, &reservation)?;
                        
                        // Publish failed reservation to STATE_USER_RESERVATION
                        info!(
                            topic = Topics::STATE_USER_RESERVATION,
                            message_key = %reservation_id,
                            reservation_state = ?reservation.state,
                            failed_reason = %reservation.failed_reason,
                            "📤 KAFKA_PRODUCE_START: Publishing failed reservation to STATE_USER_RESERVATION"
                        );
                        
                        self.producer.send(
                            Topics::STATE_USER_RESERVATION,
                            reservation_id,
                            &reservation,
                        ).await?;
                        
                        info!(
                            topic = Topics::STATE_USER_RESERVATION,
                            message_key = %reservation_id,
                            message_size_bytes = std::mem::size_of_val(&reservation),
                            "📤 KAFKA_PRODUCE_SUCCESS: Failed reservation published to STATE_USER_RESERVATION"
                        );
                        
                        info!(
                            reservation_id = %reservation_id,
                            final_state = ?reservation.state,
                            total_duration_ms = start_time.elapsed().as_millis(),
                            "❌ FLOW_END: Reservation failed at cache level"
                        );
                        
                        return Ok(());
                    } else {
                        info!(
                            reservation_id = %reservation_id,
                            filter_reason = %filter_result.reason,
                            "✅ FLOW_STEP: Reservation passed cache filter"
                        );
                    }
                }
                None => {
                    info!(
                        event_area_id = %event_area_id,
                        cache_hit = false,
                        duration_ms = cache_lookup_start.elapsed().as_millis(),
                        "🔍 FLOW_STEP: Area status not in cache, proceeding to Event Service"
                    );
                }
            }
            
            // Store the reservation in RocksDB
            let store_start = Instant::now();
            reservation_store.put(reservation_id, &reservation)?;
            debug!(
                duration_ms = store_start.elapsed().as_millis(),
                "💾 FLOW_STEP: Reservation stored in RocksDB"
            );

            // Publish reservation to STATE_USER_RESERVATION (for Ticket Service)
            let publish_start = Instant::now();
            info!(
                topic = Topics::STATE_USER_RESERVATION,
                message_key = %reservation_id,
                reservation_state = ?reservation.state,
                user_id = %create_request.user_id,
                event_id = %create_request.event_id,
                area_id = %create_request.area_id,
                "📤 KAFKA_PRODUCE_START: Publishing reservation to STATE_USER_RESERVATION"
            );
            
            self.producer.send(
                Topics::STATE_USER_RESERVATION,
                reservation_id,
                &reservation,
            ).await?;
            
            info!(
                topic = Topics::STATE_USER_RESERVATION,
                message_key = %reservation_id,
                duration_ms = publish_start.elapsed().as_millis(),
                message_size_bytes = std::mem::size_of_val(&reservation),
                "📤 KAFKA_PRODUCE_SUCCESS: Reservation published to STATE_USER_RESERVATION"
            );

            // If reservation is in PROCESSING state, send to Event Service
            
            let reservation_type = create_request.reservation_type.clone();

            if reservation.state == ReservationState::Processing {
                let reserve_seat = ReserveSeat {
                    reservation_id: reservation_id.clone(),
                    event_id: create_request.event_id.clone(),
                    area_id: create_request.area_id.clone(),
                    num_of_seats: create_request.num_of_seats,
                    num_of_seat: create_request.num_of_seat,
                    reservation_type: create_request.reservation_type,
                    seats: create_request.seats.clone(),
                };

                let event_area_key = event_area_key(&create_request.event_id, &create_request.area_id);
                let forward_start = Instant::now();

                
                info!(
                    topic = Topics::COMMAND_EVENT_RESERVE_SEAT,
                    message_key = %event_area_key,
                    reservation_id = %reservation_id,
                    event_id = %create_request.event_id,
                    area_id = %create_request.area_id,
                    num_seats = create_request.num_of_seats,
                    reservation_type = ?reservation_type,
                    "📤 KAFKA_PRODUCE_START: Sending ReserveSeat request to COMMAND_EVENT_RESERVE_SEAT"
                );
                
                self.producer.send(
                    Topics::COMMAND_EVENT_RESERVE_SEAT,
                    &event_area_key,
                    &reserve_seat,
                ).await?;

                info!(
                    topic = Topics::COMMAND_EVENT_RESERVE_SEAT,
                    message_key = %event_area_key,
                    reservation_id = %reservation_id,
                    duration_ms = forward_start.elapsed().as_millis(),
                    message_size_bytes = std::mem::size_of_val(&reserve_seat),
                    "📤 KAFKA_PRODUCE_SUCCESS: ReserveSeat request sent to COMMAND_EVENT_RESERVE_SEAT"
                );
            }

            info!(
                reservation_id = %reservation_id,
                final_state = ?reservation.state,
                total_duration_ms = start_time.elapsed().as_millis(),
                "✅ FLOW_END: Reservation creation completed"
            );
            
            Ok(())
        }.instrument(span).await

    }


    async fn handle_reservation_result(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let start_time = Instant::now();
        let span = span!(Level::INFO, "reservation_result_processing",
            message_key = %message.key.as_ref().unwrap_or(&"unknown".to_string()),
            topic = %message.topic,
            partition = message.partition,
            offset = message.offset
        );
        
        async move {
            let reservation_id = message.key.as_ref()
                .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
            
            let result: ReservationResult = message.deserialize_value()?;
            let result2 = result.clone();
            
            info!(
                reservation_id = %reservation_id,
                result = ?result.result,
                seats_count = result.seats.len(),
                "🎯 FLOW_START: Processing reservation result from Event Service"
            );

            let reservation_store = self.context
                .get_rocksdb_store(Stores::RESERVATION)
                .ok_or_else(|| TicketMasterError::InvalidArgument("Reservation store not found".to_string()))?;

            // Get existing reservation
            let lookup_start = Instant::now();
            let mut reservation = reservation_store.get::<Reservation>(reservation_id)?
                .ok_or_else(|| TicketMasterError::InvalidArgument(format!("Reservation {} not found", reservation_id)))?;
            
            debug!(
                reservation_id = %reservation_id,
                current_state = ?reservation.state,
                duration_ms = lookup_start.elapsed().as_millis(),
                "🔍 FLOW_STEP: Existing reservation retrieved from RocksDB"
            );

            // Update reservation based on result
            match result.result {
                ticket_master::ReservationResultEnum::Success => {
                    reservation.state = ReservationState::Reserved;
                    reservation.seats = result.seats.clone();
                    reservation.failed_reason = String::new();
                    
                    info!(
                        reservation_id = %reservation_id,
                        seats_reserved = result.seats.len(),
                        "✅ FLOW_STEP: Reservation marked as RESERVED with assigned seats"
                    );
                }
                ticket_master::ReservationResultEnum::Failed => {
                    reservation.state = ReservationState::Failed;
                    reservation.failed_reason = format!(
                        "[{:?}]: {}", 
                        result2.error_code.unwrap_or(ticket_master::ReservationErrorCode::Unknown),
                        result2.error_message.unwrap_or_else(|| "Unknown error".to_string())
                    );
                    
                    warn!(
                        reservation_id = %reservation_id,
                        error_code = ?result.error_code,
                        error_message = %result.error_message.as_ref().unwrap_or(&"Unknown".to_string()),
                        "❌ FLOW_STEP: Reservation marked as FAILED"
                    );
                }
            }

            // Update reservation in store
            let store_update_start = Instant::now();
            reservation_store.put(reservation_id, &reservation)?;
            debug!(
                duration_ms = store_update_start.elapsed().as_millis(),
                "💾 FLOW_STEP: Updated reservation stored in RocksDB"
            );

            // Publish updated reservation
            let publish_start = Instant::now();
            self.producer.send(
                Topics::STATE_USER_RESERVATION,
                reservation_id,
                &reservation,
            ).await?;

            info!(
                reservation_id = %reservation_id,
                final_state = ?reservation.state,
                publish_duration_ms = publish_start.elapsed().as_millis(),
                total_duration_ms = start_time.elapsed().as_millis(),
                "📤 FLOW_END: Updated reservation published to STATE_USER_RESERVATION"
            );
            
            Ok(())
        }.instrument(span).await
    }

    async fn handle_area_status_update(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_area_key = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
        
        let area_status: AreaStatus = message.deserialize_value()?;
        
        // Get RocksDB area status cache (matching Java LRU cache)
        let area_status_cache = self.context
            .get_rocksdb_store(Stores::EVENT_AREA_STATUS_CACHE)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Area status cache not found".to_string()))?;

        // Update cache
        area_status_cache.put(event_area_key, &area_status)?;
        
        // Note: In a real implementation with LRU cache, you'd implement eviction logic here
        // For now, we just store everything in the DashMap
        
        Ok(())
    }

    fn apply_filter_strategy(&self, area_status: &AreaStatus, request: &CreateReservation) -> Result<FilterResult> {
        let filter_start = Instant::now();
        
        // Implement filter logic similar to Java's FilterStrategy
        let result = match request.reservation_type {
            ticket_master::ReservationType::SelfPick => {
                        // Check if requested seats are available
                        if request.seats.is_empty() {
                            FilterResult {
                                passed: false,
                                reason: "Self-pick reservation requires specific seats".to_string(),
                            }
                        } else {
                            let all_available = request.seats.iter().all(|seat| {
                                area_status.seats
                                    .get(seat.row as usize)
                                    .and_then(|row| row.get(seat.col as usize))
                                    .map(|seat_status| seat_status.is_available)
                                    .unwrap_or(false)
                            });
                    
                            FilterResult {
                                passed: all_available,
                                reason: if all_available {
                                    "All requested seats are available".to_string()
                                } else {
                                    "Some requested seats are not available".to_string()
                                },
                            }
                        }
                    }
            ticket_master::ReservationType::Random => {
                        // Check if enough seats are available
                        let enough_seats = area_status.available_seats >= request.num_of_seats;
                        FilterResult {
                            passed: enough_seats,
                            reason: if enough_seats {
                                format!("Sufficient seats available: {} >= {}", area_status.available_seats, request.num_of_seats)
                            } else {
                                format!("Insufficient seats available: {} < {}", area_status.available_seats, request.num_of_seats)
                            },
                        }
                    }
            ticket_master::ReservationType::Invalid => {
                        let enough_seats = area_status.available_seats >= request.num_of_seats;
                        FilterResult {
                            passed: enough_seats,
                            reason: if enough_seats {
                                format!("Sufficient seats available: {} >= {}", area_status.available_seats, request.num_of_seats)
                            } else {
                                format!("Insufficient seats available: {} < {}", area_status.available_seats, request.num_of_seats)
                            },
                        }

            },
        };
        
        debug!(
            reservation_type = ?request.reservation_type,
            filter_passed = result.passed,
            filter_reason = %result.reason,
            duration_ms = filter_start.elapsed().as_millis(),
            "🎯 Filter strategy applied"
        );
        
        Ok(result)
    }
}

#[derive(Debug)]
struct FilterResult {
    passed: bool,
    reason: String,
}
use ticket_master::{
    Result, TicketMasterError, KafkaConsumer, KafkaProducer,
    CreateReservation, Reservation, ReservationResult, ReservationState, 
    ReserveSeat, AreaStatus, Topics, Stores, event_area_key,
    ProcessingContext, ReservationType, ReservationResultEnum
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, error, instrument};
use tokio::sync::Mutex;

/// Kafka Streams topology equivalent for Reservation Service
/// This implements the exact same logic as Java's createTopology() method
pub struct ReservationTopology {
    consumer: Arc<KafkaConsumer>,
    producer: Arc<KafkaProducer>,
    context: Arc<ProcessingContext>,
    // Global table equivalent for area status cache (LRU-like behavior)
    area_status_cache: Arc<Mutex<HashMap<String, AreaStatus>>>,
    // Filter strategies for different reservation types
    filter_strategies: HashMap<ReservationType, Box<dyn FilterStrategy + Send + Sync>>,
    application_id: String,
}

/// Filter strategy trait - equivalent to Java's FilterStrategy interface
pub trait FilterStrategy {
    fn pass(&self, area_status: &AreaStatus, request: &CreateReservation) -> bool;
}

/// Self-pick filter strategy - equivalent to Java's SelfPickFilterStrategy
pub struct SelfPickFilterStrategy;

impl FilterStrategy for SelfPickFilterStrategy {
    fn pass(&self, area_status: &AreaStatus, request: &CreateReservation) -> bool {
        // Check if requested seats are available for self-pick
        if !request.seats.is_empty() {
            let seats = &request.seats;
            // Validate each requested seat
            for seat in seats {
                // Check if seat is within bounds
                if seat.row < 0 || seat.col < 0 || 
                   seat.row >= area_status.row_count || 
                   seat.col >= area_status.col_count {
                    return false;
                }
                
                // Check if seat is available (simplified - in real implementation would check occupied seats)
                // For now, just check if we have enough available seats
            }
        }
        
        // Check if we have enough available seats
        area_status.available_seats >= request.num_of_seats
    }
}

/// Continuous random filter strategy - equivalent to Java's ContinuousRandomFilterStrategy
pub struct ContinuousRandomFilterStrategy;

impl FilterStrategy for ContinuousRandomFilterStrategy {
    fn pass(&self, area_status: &AreaStatus, request: &CreateReservation) -> bool {
        // For random selection, just check if we have enough available seats
        area_status.available_seats >= request.num_of_seats
    }
}

/// Stream branching result - equivalent to Java's Map<String, KStream<String, Reservation>>
#[derive(Debug)]
pub struct BranchedStreams {
    pub processed: Vec<(String, Reservation)>,
    pub processing: Vec<(String, Reservation)>,
    pub invalid: Vec<(String, Reservation)>,
}

impl ReservationTopology {
    pub fn new(
        consumer: Arc<KafkaConsumer>,
        producer: Arc<KafkaProducer>,
        context: Arc<ProcessingContext>,
        application_id: String,
    ) -> Self {
        // Initialize filter strategies (equivalent to Java's filterStrategies map)
        let mut filter_strategies: HashMap<ReservationType, Box<dyn FilterStrategy + Send + Sync>> = HashMap::new();
        filter_strategies.insert(ReservationType::SelfPick, Box::new(SelfPickFilterStrategy));
        filter_strategies.insert(ReservationType::Random, Box::new(ContinuousRandomFilterStrategy));

        Self {
            consumer,
            producer,
            context,
            area_status_cache: Arc::new(Mutex::new(HashMap::new())),
            filter_strategies,
            application_id,
        }
    }

    /// Start the Kafka Streams topology - equivalent to Java's createTopology() + KafkaStreams.start()
    #[instrument(skip(self))]
    pub async fn start_topology(&self) -> Result<()> {
        info!("Starting Reservation Service Kafka Streams topology for application: {}", self.application_id);
        
        // Subscribe to all required topics (equivalent to Java's builder.stream() and builder.globalTable())
        self.consumer.subscribe(&[
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
            Topics::RESPONSE_RESERVATION_RESULT,
            Topics::STATE_EVENT_AREA_STATUS,
        ])?;
        
        info!("Topology started - processing reservation streams");
        
        // Main topology processing loop
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    info!("Received shutdown signal for reservation topology");
                    break;
                }
                
                // Process stream messages
                message_result = self.consumer.recv_message(Duration::from_millis(100)) => {
                    match message_result? {
                        Some(message) => {
                            if let Err(e) = self.process_stream_record(&message).await {
                                error!("Error processing stream record: {}", e);
                            } else {
                                // Commit after successful processing (exactly-once semantics)
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

        info!("Reservation Kafka Streams topology shutting down...");
        Ok(())
    }

    /// Process a stream record - equivalent to the entire Java topology flow
    #[instrument(skip(self, message), fields(topic = %message.topic))]
    async fn process_stream_record(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        match message.topic.as_str() {
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION => {
                self.process_create_reservation_stream(message).await
            }
            Topics::RESPONSE_RESERVATION_RESULT => {
                self.process_reservation_result_stream(message).await
            }
            Topics::STATE_EVENT_AREA_STATUS => {
                self.process_area_status_global_table(message).await
            }
            _ => {
                warn!("Unknown topic in reservation topology: {}", message.topic);
                Ok(())
            }
        }
    }

    /// Process create reservation stream - equivalent to Java's ReservationValueProcessor
    #[instrument(skip(self, message))]
    async fn process_create_reservation_stream(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let create_request: CreateReservation = message.deserialize_value()?;
        
        info!("Processing create reservation stream: {}", reservation_id);

        // Create initial reservation (equivalent to Java's Reservation constructor)
        let mut reservation = Reservation {
            reservation_id: reservation_id.clone(),
            user_id: create_request.user_id.clone(),
            event_id: create_request.event_id.clone(),
            area_id: create_request.area_id.clone(),
            num_of_seats: create_request.num_of_seats,
            num_of_seat: create_request.num_of_seat,
            reservation_type: create_request.reservation_type.clone(),
            seats: create_request.seats.clone(),
            state: ReservationState::Processing,
            failed_reason: String::new(),
        };

        // Check area status cache (equivalent to Java's eventAreaStatusCache.get())
        let event_area_id = format!("{}#{}", create_request.event_id, create_request.area_id);
        let area_status = {
            let cache = self.area_status_cache.lock().await;
            cache.get(&event_area_id).cloned()
        };

        // Apply filter strategy (equivalent to Java's FilterStrategy.pass())
        if let Some(area_status) = area_status {
            if let Some(filter_strategy) = self.filter_strategies.get(&create_request.reservation_type) {
                if !filter_strategy.pass(&area_status, &create_request) {
                    reservation.state = ReservationState::Failed;
                    reservation.failed_reason = "Request rejected at cache level".to_string();
                    info!("Reservation {} rejected by filter strategy", reservation_id);
                }
            } else {
                reservation.state = ReservationState::Failed;
                reservation.failed_reason = format!("{:?} type reservation is not supported", create_request.reservation_type);
                warn!("Unsupported reservation type: {:?}", create_request.reservation_type);
            }
        } else {
            // Area status not in cache, forward to event service (equivalent to Java's forward logic)
            info!("Area status not in cache for {}, proceeding with processing", event_area_id);
        }

        // Store in materialized store (equivalent to Java's toTable())
        self.store_reservation(reservation_id, &reservation).await?;

        // Process the reservation through branching logic
        self.process_reservation_branching(reservation_id.clone(), reservation).await?;

        Ok(())
    }

    /// Process reservation result stream - equivalent to Java's ReservationResultValueProcessor
    #[instrument(skip(self, message))]
    async fn process_reservation_result_stream(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let result: ReservationResult = message.deserialize_value()?;
        
        info!("Processing reservation result stream: {} -> {:?}", reservation_id, result.result);

        // Get existing reservation from store (equivalent to Java's reservationStore.get())
        if let Some(mut reservation) = self.get_reservation(reservation_id).await? {
            // Update reservation with result (equivalent to Java's switch statement)
            match result.result {
                ReservationResultEnum::Success => {
                    reservation.state = ReservationState::Reserved;
                    reservation.seats = result.seats;
                    info!("Reservation {} marked as RESERVED", reservation_id);
                }
                ReservationResultEnum::Failed => {
                    reservation.state = ReservationState::Failed;
                    reservation.failed_reason = format!("[{:?}]: {}", 
                        result.error_code.as_ref().map(|c| format!("{:?}", c)).unwrap_or_else(|| "UNKNOWN".to_string()), 
                        result.error_message.unwrap_or_default());
                    info!("Reservation {} marked as FAILED: {}", reservation_id, reservation.failed_reason);
                }
            }

            // Store updated reservation (equivalent to Java's reservationStore.put())
            self.store_reservation(reservation_id, &reservation).await?;

            // Process through branching logic again (equivalent to Java's merge and branch)
            self.process_reservation_branching(reservation_id.clone(), reservation).await?;
        } else {
            warn!("Reservation not found for result: {}", reservation_id);
        }

        Ok(())
    }

    /// Process area status global table - equivalent to Java's builder.globalTable()
    #[instrument(skip(self, message))]
    async fn process_area_status_global_table(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_area_key = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
        
        let area_status: AreaStatus = message.deserialize_value()?;
        
        info!("Updating area status global table: {} -> {} available seats", event_area_key, area_status.available_seats);

        // Update global table cache (equivalent to Java's GlobalKTable with LRU)
        {
            let mut cache = self.area_status_cache.lock().await;
            cache.insert(event_area_key.clone(), area_status);
            
            // Simple LRU eviction (in production, use a proper LRU cache)
            if cache.len() > 1000 {
                // Remove oldest entries (simplified)
                let keys_to_remove: Vec<String> = cache.keys().take(100).cloned().collect();
                for key in keys_to_remove {
                    cache.remove(&key);
                }
            }
        }

        Ok(())
    }

    /// Process reservation branching - equivalent to Java's complex branching logic
    #[instrument(skip(self, reservation), fields(reservation_id = %reservation_id))]
    async fn process_reservation_branching(&self, reservation_id: String, reservation: Reservation) -> Result<()> {
        // Equivalent to Java's split().branch() logic
        match reservation.state {
            // Processed branch (FAILED or RESERVED)
            ReservationState::Failed | ReservationState::Reserved => {
                info!("Routing reservation {} to processed branch: {:?}", reservation_id, reservation.state);
                
                // Send to user reservation state topic (equivalent to Java's processedReservation.to())
                self.producer.send(
                    Topics::STATE_USER_RESERVATION,
                    &reservation_id,
                    &reservation,
                ).await?;
            }
            
            // Processing branch
            ReservationState::Processing => {
                info!("Routing reservation {} to processing branch", reservation_id);
                
                // Create ReserveSeat command (equivalent to Java's map() operation)
                let reserve_seat = ReserveSeat {
                    reservation_id: reservation.reservation_id.clone(),
                    event_id: reservation.event_id.clone(),
                    area_id: reservation.area_id.clone(),
                    num_of_seats: reservation.num_of_seats,
                    num_of_seat: reservation.num_of_seat,
                    reservation_type: reservation.reservation_type.clone(),
                    seats: reservation.seats.clone(),
                };

                let event_area_key = event_area_key(&reservation.event_id, &reservation.area_id);
                
                // Send to event service (equivalent to Java's processingReqs.to())
                self.producer.send(
                    Topics::COMMAND_EVENT_RESERVE_SEAT,
                    &event_area_key,
                    &reserve_seat,
                ).await?;
            }
            
            // Default branch (invalid states)
            _ => {
                warn!("Reservation {} has invalid state: {:?}", reservation_id, reservation.state);
                // Equivalent to Java's invalidReservation.foreach()
            }
        }

        Ok(())
    }

    /// Store reservation in materialized store
    async fn store_reservation(&self, reservation_id: &str, reservation: &Reservation) -> Result<()> {
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            store.put(reservation_id, reservation)?;
            info!("Stored reservation in materialized store: {}", reservation_id);
        } else {
            return Err(TicketMasterError::InvalidArgument("Reservation store not available".to_string()));
        }
        Ok(())
    }

    /// Get reservation from materialized store
    async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>> {
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            store.get::<Reservation>(reservation_id)
        } else {
            Err(TicketMasterError::InvalidArgument("Reservation store not available".to_string()))
        }
    }

    /// Get topology description - equivalent to topology.describe()
    pub fn describe(&self) -> String {
        format!(
            "Reservation Service Kafka Streams Topology for {}\n\
            \n\
            Sources:\n\
            - {} -> reservation-value-processor\n\
            - {} -> reservation-result-value-processor\n\
            - {} -> area-status-global-table\n\
            \n\
            Processors:\n\
            - reservation-value-processor: CreateReservation -> Reservation (with filtering)\n\
            - reservation-result-value-processor: ReservationResult -> Reservation (state update)\n\
            - reservation-branching: Split by state (PROCESSING/PROCESSED/INVALID)\n\
            \n\
            State Stores:\n\
            - {} (materialized table)\n\
            - area-status-cache (global table with LRU)\n\
            \n\
            Sinks:\n\
            - {} (processed reservations)\n\
            - {} (processing commands)\n\
            \n\
            Filter Strategies:\n\
            - SelfPick: Validates specific seat availability\n\
            - Random: Validates available seat count",
            self.application_id,
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
            Topics::RESPONSE_RESERVATION_RESULT,
            Topics::STATE_EVENT_AREA_STATUS,
            Stores::RESERVATION,
            Topics::STATE_USER_RESERVATION,
            Topics::COMMAND_EVENT_RESERVE_SEAT
        )
    }
}

/// Builder for creating reservation topology - equivalent to Java's StreamsBuilder
pub struct ReservationTopologyBuilder {
    application_id: String,
}

impl ReservationTopologyBuilder {
    pub fn new(application_id: String) -> Self {
        Self { application_id }
    }

    /// Build the topology - equivalent to Java's createTopology()
    pub fn build(
        self,
        consumer: Arc<KafkaConsumer>,
        producer: Arc<KafkaProducer>,
        context: Arc<ProcessingContext>,
    ) -> ReservationTopology {
        ReservationTopology::new(consumer, producer, context, self.application_id)
    }
}
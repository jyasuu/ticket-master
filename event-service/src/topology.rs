use ticket_master::{
    Result, TicketMasterError, KafkaConsumer, KafkaProducer,
    CreateEvent, AreaStatus, ReserveSeat, ReservationResult, ReservationResultEnum,
    ReservationErrorCode, ReservationType, Seat, Topics, Stores, event_area_key,
    ProcessingContext, Area
};
use crate::strategies::ReservationStrategy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, error, instrument};
use tokio::sync::Mutex;

/// Kafka Streams topology equivalent for Event Service
/// This implements the exact same logic as Java's createTopology() method
pub struct EventTopology {
    consumer: Arc<KafkaConsumer>,
    producer: Arc<KafkaProducer>,
    context: Arc<ProcessingContext>,
    // Reservation strategies for different types
    reservation_strategies: HashMap<ReservationType, Box<dyn ReservationStrategy + Send + Sync>>,
    application_id: String,
}

impl EventTopology {
    pub fn new(
        consumer: Arc<KafkaConsumer>,
        producer: Arc<KafkaProducer>,
        context: Arc<ProcessingContext>,
        application_id: String,
    ) -> Self {
        // Initialize reservation strategies (equivalent to Java's ReserveSeatTransformer.init())
        let mut reservation_strategies: HashMap<ReservationType, Box<dyn ReservationStrategy + Send + Sync>> = HashMap::new();
        reservation_strategies.insert(ReservationType::SelfPick, Box::new(crate::strategies::SelfPickStrategy));
        reservation_strategies.insert(ReservationType::Random, Box::new(crate::strategies::ContinuousRandomStrategy));

        Self {
            consumer,
            producer,
            context,
            reservation_strategies,
            application_id,
        }
    }

    /// Start the Kafka Streams topology - equivalent to Java's createTopology() + KafkaStreams.start()
    #[instrument(skip(self))]
    pub async fn start_topology(&self) -> Result<()> {
        info!("Starting Event Service Kafka Streams topology for application: {}", self.application_id);
        
        // Subscribe to all required topics (equivalent to Java's builder.stream())
        self.consumer.subscribe(&[
            Topics::COMMAND_EVENT_CREATE_EVENT,
            Topics::COMMAND_EVENT_RESERVE_SEAT,
        ])?;
        
        info!("Topology started - processing event streams");
        
        // Main topology processing loop
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    info!("Received shutdown signal for event topology");
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

        info!("Event Kafka Streams topology shutting down...");
        Ok(())
    }

    /// Process a stream record - equivalent to the entire Java topology flow
    #[instrument(skip(self, message), fields(topic = %message.topic))]
    async fn process_stream_record(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        match message.topic.as_str() {
            Topics::COMMAND_EVENT_CREATE_EVENT => {
                self.process_create_event_stream(message).await
            }
            Topics::COMMAND_EVENT_RESERVE_SEAT => {
                self.process_reserve_seat_stream(message).await
            }
            _ => {
                warn!("Unknown topic in event topology: {}", message.topic);
                Ok(())
            }
        }
    }

    /// Process create event stream - equivalent to Java's flatMap + toTable operations
    #[instrument(skip(self, message))]
    async fn process_create_event_stream(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_name = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event name key".to_string()))?;
        
        let create_event: CreateEvent = message.deserialize_value()?;
        
        info!("Processing create event stream: {} with {} areas", event_name, create_event.areas.len());

        // Equivalent to Java's flatMap operation:
        // createEventReqs.flatMap((eventName, createEvent) -> {
        //     List<KeyValue<String, AreaStatus>> areas = new LinkedList<>();
        //     for(Area area: createEvent.getAreas()){
        //         areas.add(KeyValue.pair(eventName + "#" + area.getAreaId(), toAreaStatus(eventName, area)));
        //     }
        //     return areas;
        // })
        
        let area_status_records = self.flat_map_create_event_to_area_status(event_name, &create_event).await?;
        
        // Equivalent to Java's toTable operation:
        // createEventAreas.toTable(Materialized.as(Schemas.Stores.AREA_STATUS.name()))
        
        for (area_key, area_status) in area_status_records {
            self.materialize_area_status(&area_key, &area_status).await?;
        }

        info!("Create event stream processed: {} -> {} areas materialized", event_name, create_event.areas.len());
        Ok(())
    }

    /// FlatMap equivalent - converts one CreateEvent into multiple AreaStatus records
    #[instrument(skip(self, create_event), fields(event_name = %event_name))]
    async fn flat_map_create_event_to_area_status(&self, event_name: &str, create_event: &CreateEvent) -> Result<Vec<(String, AreaStatus)>> {
        let mut area_status_records = Vec::new();
        
        // Equivalent to Java's flatMap logic
        for area in &create_event.areas {
            let area_key = event_area_key(event_name, &area.area_id);
            let area_status = self.to_area_status(event_name, area);
            
            area_status_records.push((area_key, area_status));
            
            info!("FlatMap: {} -> area {} ({}x{}, {} seats)", 
                  event_name, area.area_id, area.row_count, area.col_count, 
                  area.row_count * area.col_count);
        }
        
        Ok(area_status_records)
    }

    /// Convert Area to AreaStatus - equivalent to Java's toAreaStatus() method
    fn to_area_status(&self, event_name: &str, area: &Area) -> AreaStatus {
        let row_count = area.row_count;
        let col_count = area.col_count;
        let available_seats = row_count * col_count;
        
        // Create seat status matrix
        let mut seats = Vec::new();
        for row in 0..row_count {
            let mut seat_row = Vec::new();
            for col in 0..col_count {
                seat_row.push(ticket_master::SeatStatus {
                    row,
                    col,
                    is_available: true,
                });
            }
            seats.push(seat_row);
        }
        
        AreaStatus {
            event_id: event_name.to_string(),
            area_id: area.area_id.clone(),
            price: area.price,
            row_count,
            col_count,
            available_seats,
            seats,
        }
    }

    /// Materialize area status to KTable equivalent - stores and emits to state topic
    #[instrument(skip(self, area_status), fields(area_key = %area_key))]
    async fn materialize_area_status(&self, area_key: &str, area_status: &AreaStatus) -> Result<()> {
        // Store in materialized state store (equivalent to KTable materialization)
        if let Some(store) = self.context.get_rocksdb_store(Stores::AREA_STATUS) {
            store.put(area_key, area_status)?;
            info!("Materialized area status: {} -> {} available seats", area_key, area_status.available_seats);
        } else {
            return Err(TicketMasterError::InvalidArgument("Area status store not available".to_string()));
        }

        // Emit to state topic (equivalent to Java's areaStatus.toStream().to())
        self.producer.send(
            Topics::STATE_EVENT_AREA_STATUS,
            area_key,
            area_status,
        ).await?;

        Ok(())
    }

    /// Process reserve seat stream - equivalent to Java's transform operation with ReserveSeatTransformer
    #[instrument(skip(self, message))]
    async fn process_reserve_seat_stream(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_area_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
        
        let reserve_request: ReserveSeat = message.deserialize_value()?;
        
        info!("Processing reserve seat stream: {} for area {}", reserve_request.reservation_id, event_area_id);

        // Equivalent to Java's ReserveSeatTransformer.transform() method
        let reservation_result = self.transform_reserve_seat(event_area_id, &reserve_request).await?;

        // Emit result (equivalent to Java's reserveResult.to())
        self.producer.send(
            Topics::RESPONSE_RESERVATION_RESULT,
            &reserve_request.reservation_id,
            &reservation_result,
        ).await?;

        info!("Reserve seat stream processed: {} -> {:?}", reserve_request.reservation_id, reservation_result.result);
        Ok(())
    }

    /// Transform reserve seat request - equivalent to Java's ReserveSeatTransformer.transform()
    #[instrument(skip(self, reserve_request), fields(event_area_id = %event_area_id, reservation_id = %reserve_request.reservation_id))]
    async fn transform_reserve_seat(&self, event_area_id: &str, reserve_request: &ReserveSeat) -> Result<ReservationResult> {
        // Equivalent to Java's ReserveSeatTransformer.transform() method:
        // 1. Get area status from state store
        // 2. Apply reservation strategy
        // 3. Update state store if successful
        // 4. Return result

        // Step 1: Get area status from state store (equivalent to areaStatusStore.get())
        let mut area_status = if let Some(store) = self.context.get_rocksdb_store(Stores::AREA_STATUS) {
            match store.get::<AreaStatus>(event_area_id)? {
                Some(status) => status,
                None => {
                    warn!("Area status not found for: {}", event_area_id);
                    return Ok(ReservationResult {
                        reservation_id: reserve_request.reservation_id.clone(),
                        result: ReservationResultEnum::Failed,
                        error_code: Some(ReservationErrorCode::InvalidEventArea),
                        error_message: Some(format!("{} event area does not exist", event_area_id)),
                        seats: Vec::new(),
                    });
                }
            }
        } else {
            return Err(TicketMasterError::InvalidArgument("Area status store not available".to_string()));
        };

        // Step 2: Apply reservation strategy (equivalent to reservationStrategy.reserve())
        let strategy = self.reservation_strategies.get(&reserve_request.reservation_type)
            .ok_or_else(|| TicketMasterError::InvalidReservationStrategy(format!("{:?}", reserve_request.reservation_type)))?;

        let mut result = strategy.reserve(&mut area_status, reserve_request)?;

        // Step 3: Update state store if successful (equivalent to areaStatusStore.put())
        if result.result == ReservationResultEnum::Success {
            // Update seat availability in area status
            for seat in &result.seats {
                let row = seat.row as usize;
                let col = seat.col as usize;
                
                if row < area_status.seats.len() && col < area_status.seats[row].len() {
                    area_status.seats[row][col].is_available = false;
                }
            }
            
            // Update available seats count
            area_status.available_seats -= result.seats.len() as i32;
            
            // Store updated area status (equivalent to areaStatusStore.put())
            if let Some(store) = self.context.get_rocksdb_store(Stores::AREA_STATUS) {
                store.put(event_area_id, &area_status)?;
                
                // Emit updated area status to state topic (automatic in Java via KTable.toStream())
                self.producer.send(
                    Topics::STATE_EVENT_AREA_STATUS,
                    event_area_id,
                    &area_status,
                ).await?;
                
                info!("Updated area status: {} -> {} available seats remaining", 
                      event_area_id, area_status.available_seats);
            }
        }

        // Step 4: Return result
        Ok(result)
    }

    /// Get topology description - equivalent to topology.describe()
    pub fn describe(&self) -> String {
        format!(
            "Event Service Kafka Streams Topology for {}\n\
            \n\
            Sources:\n\
            - {} -> create-event-processor\n\
            - {} -> reserve-seat-transformer\n\
            \n\
            Processors:\n\
            - create-event-processor: CreateEvent -> flatMap -> AreaStatus (materialized)\n\
            - reserve-seat-transformer: ReserveSeat -> transform -> ReservationResult\n\
            \n\
            State Stores:\n\
            - {} (materialized table for area status)\n\
            \n\
            Sinks:\n\
            - {} (area status state changes)\n\
            - {} (reservation results)\n\
            \n\
            Reservation Strategies:\n\
            - SelfPick: Validates specific seat availability\n\
            - Random: Continuous seat finding with fallback",
            self.application_id,
            Topics::COMMAND_EVENT_CREATE_EVENT,
            Topics::COMMAND_EVENT_RESERVE_SEAT,
            Stores::AREA_STATUS,
            Topics::STATE_EVENT_AREA_STATUS,
            Topics::RESPONSE_RESERVATION_RESULT
        )
    }
}

/// Builder for creating event topology - equivalent to Java's StreamsBuilder
pub struct EventTopologyBuilder {
    application_id: String,
}

impl EventTopologyBuilder {
    pub fn new(application_id: String) -> Self {
        Self { application_id }
    }

    /// Build the topology - equivalent to Java's createTopology()
    pub fn build(
        self,
        consumer: Arc<KafkaConsumer>,
        producer: Arc<KafkaProducer>,
        context: Arc<ProcessingContext>,
    ) -> EventTopology {
        EventTopology::new(consumer, producer, context, self.application_id)
    }
}

/// Kafka Streams configuration for event service
#[derive(Debug, Clone)]
pub struct EventStreamsConfig {
    pub application_id: String,
    pub state_dir: String,
    pub commit_interval_ms: u64,
    pub processing_guarantee: String,
}

impl EventStreamsConfig {
    pub fn new(application_id: String) -> Self {
        Self {
            application_id,
            state_dir: "/tmp/kafka-streams".to_string(),
            commit_interval_ms: 20,
            processing_guarantee: "exactly_once_v2".to_string(),
        }
    }

    pub fn state_dir(mut self, dir: String) -> Self {
        self.state_dir = dir;
        self
    }

    pub fn commit_interval_ms(mut self, interval: u64) -> Self {
        self.commit_interval_ms = interval;
        self
    }

    pub fn processing_guarantee(mut self, guarantee: String) -> Self {
        self.processing_guarantee = guarantee;
        self
    }
}
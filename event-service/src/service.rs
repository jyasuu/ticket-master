use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaConsumer, KafkaProducer,
    CreateEvent, AreaStatus, ReserveSeat, ReservationResult, ReservationResultEnum,
    ReservationErrorCode, ReservationType, Seat, Topics, Stores, event_area_key,
    StateStore, ProcessingContext
};
use crate::strategies::{ReservationStrategy, SelfPickStrategy, RandomStrategy};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, error, warn};
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

        // Initialize state stores with RocksDB
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
        match message.topic.as_str() {
            Topics::COMMAND_EVENT_CREATE_EVENT => {
                self.handle_create_event(message).await
            }
            Topics::COMMAND_EVENT_RESERVE_SEAT => {
                self.handle_reserve_seat(message).await
            }
            _ => {
                warn!("Unknown topic: {}", message.topic);
                Ok(())
            }
        }
    }

    async fn handle_create_event(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_name = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event name key".to_string()))?;
        
        let create_event: CreateEvent = message.deserialize_value()?;
        
        info!("Creating event: {}", event_name);

        let area_status_store = self.context
            .get_rocksdb_store(Stores::AREA_STATUS)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Area status store not found".to_string()))?;

        // Create area status for each area and store them
        for area in &create_event.areas {
            let area_status = AreaStatus::from_area(event_name, area);
            let key = event_area_key(event_name, &area.area_id);
            
            area_status_store.put(&key, &area_status)?;
            
            // Emit area status to state topic
            self.producer.send(
                Topics::STATE_EVENT_AREA_STATUS,
                &key,
                &area_status,
            ).await?;
        }

        info!("Event created successfully: {}", event_name);
        Ok(())
    }

    async fn handle_reserve_seat(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_area_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
        
        let reserve_request: ReserveSeat = message.deserialize_value()?;
        
        info!("Processing seat reservation: {}", reserve_request.reservation_id);

        let area_status_store = self.context
            .get_rocksdb_store(Stores::AREA_STATUS)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Area status store not found".to_string()))?;

        // Get current area status
        let mut area_status = area_status_store.get::<AreaStatus>(event_area_id)?
            .ok_or_else(|| TicketMasterError::InvalidEventArea(event_area_id.clone()))?;

        // Get reservation strategy
        let strategy = self.strategies.get(&reserve_request.reservation_type)
            .ok_or_else(|| TicketMasterError::InvalidReservationStrategy(format!("{:?}", reserve_request.reservation_type)))?;

        // Execute reservation
        let result = strategy.reserve(&mut area_status, &reserve_request)?;

        // If successful, update the area status
        if result.result == ReservationResultEnum::Success {
            // Update seat availability
            for seat in &result.seats {
                if let Some(seat_status) = area_status.seats
                    .get_mut(seat.row as usize)
                    .and_then(|row| row.get_mut(seat.col as usize)) {
                    seat_status.is_available = false;
                }
            }
            area_status.available_seats -= result.seats.len() as i32;
            
            // Update state store
            area_status_store.put(event_area_id, &area_status)?;
            
            // Emit updated area status
            self.producer.send(
                Topics::STATE_EVENT_AREA_STATUS,
                event_area_id,
                &area_status,
            ).await?;
        }

        // Send reservation result
        self.producer.send(
            Topics::RESPONSE_RESERVATION_RESULT,
            &reserve_request.reservation_id,
            &result,
        ).await?;

        info!("Seat reservation processed: {} -> {:?}", 
               reserve_request.reservation_id, result.result);
        Ok(())
    }
}
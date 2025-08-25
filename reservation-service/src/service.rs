use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaConsumer, KafkaProducer,
    CreateReservation, Reservation, ReservationResult, ReservationState, 
    ReserveSeat, AreaStatus, Topics, Stores, event_area_key,
    StateStore, ProcessingContext, RocksDBStore
};
use std::time::Duration;
use tracing::{info, error, warn};
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

        // Initialize state stores with RocksDB
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        
        // Reservation store
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;
        
        // Area status cache
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
        match message.topic.as_str() {
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION => {
                self.handle_create_reservation(message).await
            }
            Topics::RESPONSE_RESERVATION_RESULT => {
                self.handle_reservation_result(message).await
            }
            Topics::STATE_EVENT_AREA_STATUS => {
                self.handle_area_status_update(message).await
            }
            _ => {
                warn!("Unknown topic: {}", message.topic);
                Ok(())
            }
        }
    }

    async fn handle_create_reservation(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let create_request: CreateReservation = message.deserialize_value()?;
        
        info!("Creating reservation: {}", reservation_id);

        let reservation_store: StateStore<String, Reservation> = self.context
            .get_store(Stores::RESERVATION)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Reservation store not found".to_string()))?;

        // Create new reservation
        let reservation = Reservation::new(create_request);
        
        // Store the reservation
        reservation_store.put(reservation_id.clone(), reservation.clone());

        // Check reservation state and process accordingly
        match reservation.state {
            ReservationState::Processing => {
                // Send reserve seat command
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
                
                self.producer.send(
                    Topics::COMMAND_EVENT_RESERVE_SEAT,
                    &event_area_key,
                    &reserve_seat,
                ).await?;

                info!("Sent reserve seat command for reservation: {}", reservation_id);
            }
            ReservationState::Reserved | ReservationState::Failed => {
                // Send to user reservation state topic
                self.producer.send(
                    Topics::STATE_USER_RESERVATION,
                    reservation_id,
                    &reservation,
                ).await?;

                info!("Reservation processed: {} -> {:?}", reservation_id, reservation.state);
            }
            _ => {
                warn!("Reservation {} has invalid state: {:?}", reservation_id, reservation.state);
            }
        }

        Ok(())
    }

    async fn handle_reservation_result(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let result: ReservationResult = message.deserialize_value()?;
        
        info!("Processing reservation result: {} -> {:?}", reservation_id, result.result);

        let reservation_store: StateStore<String, Reservation> = self.context
            .get_store(Stores::RESERVATION)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Reservation store not found".to_string()))?;

        // Get existing reservation
        if let Some(mut reservation) = reservation_store.get(reservation_id) {
            // Update reservation with result
            reservation.update_from_result(&result);
            
            // Store updated reservation
            reservation_store.put(reservation_id.clone(), reservation.clone());

            // Send to user reservation state topic
            self.producer.send(
                Topics::STATE_USER_RESERVATION,
                reservation_id,
                &reservation,
            ).await?;

            info!("Updated reservation: {} -> {:?}", reservation_id, reservation.state);
        } else {
            warn!("Reservation not found for result: {}", reservation_id);
        }

        Ok(())
    }

    async fn handle_area_status_update(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let event_area_key = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing event area key".to_string()))?;
        
        let area_status: AreaStatus = message.deserialize_value()?;
        
        let area_status_cache: StateStore<String, AreaStatus> = self.context
            .get_store(Stores::EVENT_AREA_STATUS_CACHE)
            .ok_or_else(|| TicketMasterError::InvalidArgument("Area status cache not found".to_string()))?;

        // Update cache
        area_status_cache.put(event_area_key.clone(), area_status);
        
        // Note: In a real implementation with LRU cache, you'd implement eviction logic here
        // For now, we just store everything in the DashMap
        
        Ok(())
    }
}
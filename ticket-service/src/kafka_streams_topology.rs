use ticket_master::{
    Result, TicketMasterError, KafkaConsumer, Reservation, Topics, Stores,
    ProcessingContext
};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, error, warn, instrument};

/// Kafka Streams-style topology implementation for Rust
/// This mimics the Java createTopology() method functionality
pub struct KafkaStreamsTopology {
    consumer: Arc<KafkaConsumer>,
    context: Arc<ProcessingContext>,
    outstanding_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Reservation>>>>>,
    application_id: String,
}

impl KafkaStreamsTopology {
    pub fn new(
        consumer: Arc<KafkaConsumer>,
        context: Arc<ProcessingContext>,
        outstanding_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Reservation>>>>>,
        application_id: String,
    ) -> Self {
        Self {
            consumer,
            context,
            outstanding_requests,
            application_id,
        }
    }

    /// Creates and starts the Kafka Streams topology equivalent
    /// This mimics the Java createTopology() method
    #[instrument(skip(self))]
    pub async fn start_topology(&self) -> Result<()> {
        info!("Starting Kafka Streams topology for application: {}", self.application_id);
        
        // Subscribe to the reservation state topic (equivalent to builder.stream())
        self.consumer.subscribe(&[Topics::STATE_USER_RESERVATION])?;
        
        info!("Topology started - processing reservation stream updates");
        
        // Main topology processing loop
        // This is equivalent to the KStream -> KTable -> foreach chain in Java
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    info!("Received shutdown signal for topology");
                    break;
                }
                
                // Process stream messages (equivalent to KStream processing)
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

        info!("Kafka Streams topology shutting down...");
        Ok(())
    }

    /// Process a stream record - equivalent to the KStream -> KTable -> foreach chain
    #[instrument(skip(self, message), fields(topic = %message.topic))]
    async fn process_stream_record(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        match message.topic.as_str() {
            Topics::STATE_USER_RESERVATION => {
                self.process_reservation_stream_record(message).await
            }
            _ => {
                warn!("Unknown topic in topology: {}", message.topic);
                Ok(())
            }
        }
    }

    /// Process reservation stream record - equivalent to the reservationTable.toStream().foreach()
    #[instrument(skip(self, message))]
    async fn process_reservation_stream_record(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let reservation: Reservation = message.deserialize_value()?;
        
        info!("Processing reservation stream record: {} -> {:?}", reservation_id, reservation.state);

        // Step 1: Update materialized state store (equivalent to KTable materialization)
        self.update_materialized_store(reservation_id, &reservation).await?;
        
        // Step 2: Process outstanding requests (equivalent to foreach operation)
        self.process_outstanding_request(reservation_id, reservation).await?;

        Ok(())
    }

    /// Update the materialized state store - equivalent to KTable materialization
    #[instrument(skip(self, reservation), fields(reservation_id = %reservation_id))]
    async fn update_materialized_store(&self, reservation_id: &str, reservation: &Reservation) -> Result<()> {
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            store.put(reservation_id, reservation)?;
            info!("Updated materialized store for reservation: {}", reservation_id);
        } else {
            warn!("Reservation store not available for materialization");
        }
        Ok(())
    }

    /// Process outstanding requests - equivalent to the foreach((reservationId, reservation) -> {...})
    #[instrument(skip(self, reservation), fields(reservation_id = %reservation_id))]
    async fn process_outstanding_request(&self, reservation_id: &str, reservation: Reservation) -> Result<()> {
        // This is the direct equivalent of the Java foreach operation:
        // reservationTable.toStream().foreach((reservationId, reservation) -> {
        //     final AsyncResponseWithMetadata asyncResponseWithMetadata = outstandingRequests.remove(reservationId);
        //     if(asyncResponseWithMetadata == null) return;
        //     asyncResponse.resume(ReservationBean.fromAvro(reservation));
        // });

        let mut outstanding_requests = self.outstanding_requests.lock().await;
        
        if let Some(sender) = outstanding_requests.remove(reservation_id) {
            info!("Completing outstanding request for reservation: {}", reservation_id);
            
            // Complete the outstanding request (equivalent to asyncResponse.resume())
            if let Err(_) = sender.send(Ok(reservation)) {
                warn!("Failed to send reservation to outstanding request - receiver may have been dropped");
            }
        } else {
            // No outstanding request for this reservation (normal case)
            info!("No outstanding request for reservation: {}", reservation_id);
        }

        Ok(())
    }

    /// Get topology description - equivalent to topology.describe()
    pub fn describe(&self) -> String {
        format!(
            "Kafka Streams Topology for {}\n\
            Source: {} -> Processor: reservation-processor -> Sink: materialized-store\n\
            State Stores: [{}]\n\
            Outstanding Requests Processor: foreach-outstanding-requests",
            self.application_id,
            Topics::STATE_USER_RESERVATION,
            Stores::RESERVATION
        )
    }

    /// Get the number of outstanding requests - for monitoring
    pub async fn get_outstanding_requests_count(&self) -> usize {
        let outstanding_requests = self.outstanding_requests.lock().await;
        outstanding_requests.len()
    }
}

/// Builder for creating Kafka Streams topology - equivalent to StreamsBuilder
pub struct TopologyBuilder {
    application_id: String,
}

impl TopologyBuilder {
    pub fn new(application_id: String) -> Self {
        Self { application_id }
    }

    /// Build the topology - equivalent to createTopology()
    pub fn build(
        self,
        consumer: Arc<KafkaConsumer>,
        context: Arc<ProcessingContext>,
        outstanding_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Reservation>>>>>,
    ) -> KafkaStreamsTopology {
        KafkaStreamsTopology::new(consumer, context, outstanding_requests, self.application_id)
    }
}

/// Kafka Streams configuration - equivalent to StreamsConfig
#[derive(Debug, Clone)]
pub struct StreamsConfig {
    pub application_id: String,
    pub bootstrap_servers: String,
    pub state_dir: String,
    pub commit_interval_ms: u64,
    pub processing_guarantee: String,
}

impl StreamsConfig {
    pub fn new(application_id: String) -> Self {
        Self {
            application_id,
            bootstrap_servers: "localhost:29092,localhost:39092,localhost:49092".to_string(),
            state_dir: "/tmp/kafka-streams".to_string(),
            commit_interval_ms: 100,
            processing_guarantee: "exactly_once_v2".to_string(),
        }
    }

    pub fn bootstrap_servers(mut self, servers: String) -> Self {
        self.bootstrap_servers = servers;
        self
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
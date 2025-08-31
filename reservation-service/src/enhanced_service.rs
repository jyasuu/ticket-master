use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaConsumer, KafkaProducer,
    ProcessingContext, Stores
};
use crate::topology::{ReservationTopology, ReservationTopologyBuilder};
use std::sync::Arc;
use tracing::{info, error};

/// Enhanced Reservation Service with Kafka Streams topology support
/// This version implements the exact equivalent of Java's sophisticated stream processing
pub struct EnhancedReservationService {
    topology: ReservationTopology,
    config: ServiceConfig,
}

impl EnhancedReservationService {
    pub async fn new(config: ServiceConfig) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        
        let consumer = Arc::new(KafkaConsumer::new(kafka_config.clone())?);
        let producer = Arc::new(KafkaProducer::new(kafka_config)?);
        
        // Initialize state stores with RocksDB (matching Java implementation)
        let context = Arc::new(ProcessingContext::with_state_dir(config.state_dir.clone()));
        
        // Reservation store (equivalent to Java's RESERVATION materialized store)
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;
        
        // Area status cache (equivalent to Java's EVENT_AREA_STATUS_CACHE LRU store)
        context.add_rocksdb_store(Stores::EVENT_AREA_STATUS_CACHE.to_string(), "area-status-cache")?;

        // Build the Kafka Streams topology (equivalent to Java's createTopology())
        let topology_builder = ReservationTopologyBuilder::new(config.application_id.clone());
        let topology = topology_builder.build(consumer, producer, context);

        info!("Enhanced Reservation Service topology created");
        info!("Topology description:\n{}", topology.describe());

        Ok(Self {
            topology,
            config,
        })
    }

    /// Start the enhanced reservation service with Kafka Streams topology
    pub async fn run(&self) -> Result<()> {
        info!("Starting Enhanced Reservation Service with Kafka Streams topology...");
        info!("Application ID: {}", self.config.application_id);
        info!("State directory: {}", self.config.state_dir);
        
        // Start the Kafka Streams topology (equivalent to Java's streams.start())
        self.topology.start_topology().await?;
        
        Ok(())
    }

    /// Get topology description for monitoring
    pub fn describe_topology(&self) -> String {
        self.topology.describe()
    }
}

/// Configuration for the enhanced reservation service
#[derive(Debug, Clone)]
pub struct EnhancedReservationConfig {
    pub application_id: String,
    pub state_dir: String,
    pub commit_interval_ms: u64,
    pub processing_guarantee: String,
    pub max_lru_entries: usize,
}

impl Default for EnhancedReservationConfig {
    fn default() -> Self {
        Self {
            application_id: "enhanced-reservation-service".to_string(),
            state_dir: "/tmp/kafka-streams".to_string(),
            commit_interval_ms: 20,
            processing_guarantee: "exactly_once_v2".to_string(),
            max_lru_entries: 1000,
        }
    }
}

impl From<ServiceConfig> for EnhancedReservationConfig {
    fn from(config: ServiceConfig) -> Self {
        Self {
            application_id: config.application_id,
            state_dir: config.state_dir,
            commit_interval_ms: config.commit_interval_ms.unwrap_or(20),
            processing_guarantee: config.processing_guarantee.unwrap_or_else(|| "exactly_once_v2".to_string()),
            max_lru_entries: 1000,
        }
    }
}
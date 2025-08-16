use thiserror::Error;

#[derive(Error, Debug)]
pub enum TicketMasterError {
    #[error("Kafka error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] apache_avro::Error),
    
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Invalid event area: {0}")]
    InvalidEventArea(String),
    
    #[error("Invalid reservation strategy: {0}")]
    InvalidReservationStrategy(String),
    
    #[error("Seat not available: row {row}, col {col}")]
    SeatNotAvailable { row: i32, col: i32 },
    
    #[error("Insufficient seats available")]
    InsufficientSeats,
    
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

pub type Result<T> = std::result::Result<T, TicketMasterError>;
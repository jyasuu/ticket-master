use crate::{Result, TicketMasterError};
use rdkafka::producer::{FutureProducer, FutureRecord, Producer};
use rdkafka::ClientConfig;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone)]
pub struct KafkaProducer {
    producer: FutureProducer,
}

impl KafkaProducer {
    pub fn new(config: ClientConfig) -> Result<Self> {
        let producer: FutureProducer = config.create()?;
        Ok(Self { producer })
    }

    pub async fn send<T>(&self, topic: &str, key: &str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let payload = serde_json::to_string(value)?;
        
        let record = FutureRecord::to(topic)
            .key(key)
            .payload(&payload);

        self.producer
            .send(record, Duration::from_secs(10))
            .await
            .map_err(|(kafka_err, _)| TicketMasterError::Kafka(kafka_err))?;

        Ok(())
    }

    pub async fn flush(&self, timeout: Duration) -> Result<()> {
        self.producer.flush(timeout)?;
        Ok(())
    }
}
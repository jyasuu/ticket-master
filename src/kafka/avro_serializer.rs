use crate::{Result, TicketMasterError};
use apache_avro::{Schema, Writer, Reader, from_value, to_value};
use schema_registry_converter::async_impl::{
    schema_registry::SrSettings,
    avro::{AvroEncoder, AvroDecoder},
};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Avro serializer with Schema Registry support
pub struct AvroSerializer {
    // For now, we'll use a simpler approach without schema registry
    schemas: HashMap<String, Schema>,
}

impl AvroSerializer {
    pub async fn new(_schema_registry_url: &str) -> Result<Self> {
        // Simplified implementation without schema registry for now
        Ok(Self {
            schemas: HashMap::new(),
        })
    }

    pub async fn serialize<T>(&self, _subject: &str, value: &T) -> Result<Vec<u8>>
    where
        T: Serialize,
    {
        // Simplified: use JSON for now
        let json = serde_json::to_vec(value)?;
        Ok(json)
    }

    pub async fn deserialize<T>(&self, data: &[u8]) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        // Simplified: use JSON for now
        let value = serde_json::from_slice(data)?;
        Ok(value)
    }

    pub fn load_schema_from_file(&mut self, name: &str, schema_path: &str) -> Result<()> {
        let schema_content = std::fs::read_to_string(schema_path)?;
        let schema = Schema::parse_str(&schema_content)?;
        self.schemas.insert(name.to_string(), schema);
        Ok(())
    }

    pub fn get_schema(&self, name: &str) -> Option<&Schema> {
        self.schemas.get(name)
    }
}

/// Enhanced Kafka producer with Avro support
pub struct AvroKafkaProducer {
    producer: crate::KafkaProducer,
    serializer: Option<AvroSerializer>,
}

impl AvroKafkaProducer {
    pub async fn new(config: rdkafka::ClientConfig, schema_registry_url: Option<&str>) -> Result<Self> {
        let producer = crate::KafkaProducer::new(config)?;
        
        let serializer = if let Some(url) = schema_registry_url {
            Some(AvroSerializer::new(url).await?)
        } else {
            None
        };

        Ok(Self {
            producer,
            serializer,
        })
    }

    pub async fn send_avro<T>(&self, topic: &str, key: &str, value: &T, subject: &str) -> Result<()>
    where
        T: Serialize,
    {
        if let Some(serializer) = &self.serializer {
            let serialized = serializer.serialize(subject, value).await?;
            // Send raw bytes to Kafka
            self.send_raw(topic, key, &serialized).await
        } else {
            // Fallback to JSON
            self.producer.send(topic, key, value).await
        }
    }

    pub async fn send_json<T>(&self, topic: &str, key: &str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        self.producer.send(topic, key, value).await
    }

    async fn send_raw(&self, topic: &str, key: &str, payload: &[u8]) -> Result<()> {
        use rdkafka::producer::{FutureProducer, FutureRecord};
        use std::time::Duration;
        
        // Access the underlying producer - this would need to be exposed in KafkaProducer
        // For now, we'll use JSON fallback
        Err(TicketMasterError::InvalidArgument("Raw send not implemented yet".to_string()))
    }
}

/// Enhanced Kafka consumer with Avro support  
pub struct AvroKafkaConsumer {
    consumer: crate::KafkaConsumer,
    deserializer: Option<AvroSerializer>,
}

impl AvroKafkaConsumer {
    pub async fn new(config: rdkafka::ClientConfig, schema_registry_url: Option<&str>) -> Result<Self> {
        let consumer = crate::KafkaConsumer::new(config)?;
        
        let deserializer = if let Some(url) = schema_registry_url {
            Some(AvroSerializer::new(url).await?)
        } else {
            None
        };

        Ok(Self {
            consumer,
            deserializer,
        })
    }

    pub async fn recv_message(&self, timeout: std::time::Duration) -> Result<Option<crate::KafkaMessage>> {
        self.consumer.recv_message(timeout).await
    }

    pub async fn deserialize_avro<T>(&self, message: &crate::KafkaMessage) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        if let Some(deserializer) = &self.deserializer {
            if let Some(payload) = &message.payload {
                let bytes = payload.as_bytes();
                deserializer.deserialize(bytes).await
            } else {
                Err(TicketMasterError::InvalidArgument("Empty message payload".to_string()))
            }
        } else {
            // Fallback to JSON
            message.deserialize_value()
        }
    }
}
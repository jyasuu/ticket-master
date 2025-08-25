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
    encoder: AvroEncoder,
    decoder: AvroDecoder,
    schemas: HashMap<String, Schema>,
}

impl AvroSerializer {
    pub async fn new(schema_registry_url: &str) -> Result<Self> {
        let sr_settings = SrSettings::new(schema_registry_url.to_string());
        let encoder = AvroEncoder::new(sr_settings.clone());
        let decoder = AvroDecoder::new(sr_settings);
        
        Ok(Self {
            encoder,
            decoder,
            schemas: HashMap::new(),
        })
    }

    pub async fn serialize<T>(&self, subject: &str, value: &T) -> Result<Vec<u8>>
    where
        T: Serialize,
    {
        let avro_value = to_value(value)?;
        let encoded = self.encoder.encode(avro_value, subject).await
            .map_err(|e| TicketMasterError::InvalidArgument(format!("Avro encoding error: {}", e)))?;
        Ok(encoded)
    }

    pub async fn deserialize<T>(&self, data: &[u8]) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let decoded = self.decoder.decode(Some(data)).await
            .map_err(|e| TicketMasterError::InvalidArgument(format!("Avro decoding error: {}", e)))?;
        
        let value = from_value::<T>(&decoded.value)?;
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
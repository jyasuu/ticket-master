use crate::{Result, TicketMasterError};
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::{ClientConfig, Message, TopicPartitionList};
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::timeout;

pub struct KafkaConsumer {
    consumer: StreamConsumer,
}

impl KafkaConsumer {
    pub fn new(config: ClientConfig) -> Result<Self> {
        let consumer: StreamConsumer = config.create()?;
        Ok(Self { consumer })
    }

    pub fn subscribe(&self, topics: &[&str]) -> Result<()> {
        self.consumer.subscribe(topics)?;
        Ok(())
    }

    pub async fn recv_message(&self, timeout_duration: Duration) -> Result<Option<KafkaMessage>> {
        match timeout(timeout_duration, self.consumer.recv()).await {
            Ok(Ok(message)) => {
                let key = message.key()
                    .map(|k| String::from_utf8_lossy(k).to_string());
                
                let payload = message.payload()
                    .map(|p| String::from_utf8_lossy(p).to_string());

                let topic = message.topic().to_string();
                let partition = message.partition();
                let offset = message.offset();

                Ok(Some(KafkaMessage {
                    topic,
                    partition,
                    offset,
                    key,
                    payload,
                }))
            }
            Ok(Err(e)) => Err(TicketMasterError::Kafka(e)),
            Err(_) => Ok(None), // Timeout
        }
    }

    pub fn commit_message(&self, message: &KafkaMessage) -> Result<()> {
        let mut tpl = TopicPartitionList::new();
        tpl.add_partition_offset(&message.topic, message.partition, rdkafka::Offset::Offset(message.offset + 1))?;
        self.consumer.commit(&tpl, rdkafka::consumer::CommitMode::Sync)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct KafkaMessage {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub key: Option<String>,
    pub payload: Option<String>,
}

impl KafkaMessage {
    pub fn deserialize_value<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        match &self.payload {
            Some(payload) => {
                let value = serde_json::from_str(payload)?;
                Ok(value)
            }
            None => Err(TicketMasterError::InvalidArgument("Empty message payload".to_string())),
        }
    }
}
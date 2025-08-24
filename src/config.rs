use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KafkaConfig {
    pub bootstrap_servers: String,
    pub schema_registry_url: Option<String>,
    pub security_protocol: Option<String>,
    pub sasl_mechanism: Option<String>,
    pub sasl_username: Option<String>,
    pub sasl_password: Option<String>,
    pub ssl_ca_location: Option<String>,
    pub additional_properties: HashMap<String, String>,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            bootstrap_servers: "localhost:29092,localhost:39092,localhost:49092".to_string(),
            schema_registry_url: Some("http://localhost:8081".to_string()),
            security_protocol: None,
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
            ssl_ca_location: None,
            additional_properties: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub application_id: String,
    pub state_dir: String,
    pub kafka: KafkaConfig,
    pub commit_interval_ms: Option<u64>,
    pub processing_guarantee: Option<String>,
}

impl ServiceConfig {
    pub fn to_kafka_config(&self) -> rdkafka::ClientConfig {
        let mut config = rdkafka::ClientConfig::new();
        
        config.set("bootstrap.servers", &self.kafka.bootstrap_servers);
        config.set("group.id", &self.application_id);
        config.set("auto.offset.reset", "earliest");
        
        if let Some(security_protocol) = &self.kafka.security_protocol {
            config.set("security.protocol", security_protocol);
        }
        
        if let Some(sasl_mechanism) = &self.kafka.sasl_mechanism {
            config.set("sasl.mechanism", sasl_mechanism);
        }
        
        if let Some(sasl_username) = &self.kafka.sasl_username {
            config.set("sasl.username", sasl_username);
        }
        
        if let Some(sasl_password) = &self.kafka.sasl_password {
            config.set("sasl.password", sasl_password);
        }
        
        if let Some(ssl_ca_location) = &self.kafka.ssl_ca_location {
            config.set("ssl.ca.location", ssl_ca_location);
        }
        
        // Add additional properties
        for (key, value) in &self.kafka.additional_properties {
            config.set(key, value);
        }
        
        config
    }
}
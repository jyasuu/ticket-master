use crate::{Result, TicketMasterError, ServiceConfig, KafkaConfig};
use java_properties::PropertiesIter;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Parse Java properties file into ServiceConfig
pub fn parse_properties_file<P: AsRef<Path>>(path: P, application_id: &str) -> Result<ServiceConfig> {
    let file = File::open(&path).map_err(|e| {
        TicketMasterError::InvalidArgument(format!("Failed to open config file {:?}: {}", path.as_ref(), e))
    })?;
    
    let reader = BufReader::new(file);
    let properties: HashMap<String, String> = PropertiesIter::new(reader)
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| TicketMasterError::InvalidArgument(format!("Failed to parse properties: {}", e)))?;

    let mut kafka_config = KafkaConfig::default();
    let mut additional_properties = HashMap::new();
    let mut commit_interval_ms = None;
    let mut processing_guarantee = None;

    for (key, value) in properties {
        match key.as_str() {
            "bootstrap.servers" => kafka_config.bootstrap_servers = value,
            "schema.registry.url" => kafka_config.schema_registry_url = Some(value),
            "security.protocol" => kafka_config.security_protocol = Some(value),
            "sasl.mechanism" => kafka_config.sasl_mechanism = Some(value),
            "sasl.username" => kafka_config.sasl_username = Some(value),
            "sasl.password" => kafka_config.sasl_password = Some(value),
            "ssl.ca.location" => kafka_config.ssl_ca_location = Some(value),
            "commit.interval.ms" => {
                commit_interval_ms = Some(value.parse().unwrap_or(20));
            },
            "processing.guarantee" => processing_guarantee = Some(value),
            _ => {
                additional_properties.insert(key, value);
            }
        }
    }

    kafka_config.additional_properties = additional_properties;

    Ok(ServiceConfig {
        application_id: application_id.to_string(),
        state_dir: "/tmp/kafka-streams".to_string(),
        kafka: kafka_config,
        commit_interval_ms,
        processing_guarantee,
    })
}

/// Parse stream-specific properties file and merge with base config
pub fn merge_stream_properties<P: AsRef<Path>>(mut config: ServiceConfig, path: P) -> Result<ServiceConfig> {
    let file = File::open(&path).map_err(|e| {
        TicketMasterError::InvalidArgument(format!("Failed to open stream config file {:?}: {}", path.as_ref(), e))
    })?;
    
    let reader = BufReader::new(file);
    let properties: HashMap<String, String> = PropertiesIter::new(reader)
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| TicketMasterError::InvalidArgument(format!("Failed to parse stream properties: {}", e)))?;

    // Merge stream-specific properties
    for (key, value) in properties {
        config.kafka.additional_properties.insert(key, value);
    }

    Ok(config)
}
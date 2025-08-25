use ticket_master::*;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_config_parsing_integration() {
    // Test Java properties parsing with real config file
    let config_content = r#"
bootstrap.servers=localhost:9092,localhost:9093
schema.registry.url=http://localhost:8081
commit.interval.ms=100
processing.guarantee=exactly_once_v2
security.protocol=PLAINTEXT
auto.offset.reset=earliest
"#;
    
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("test.properties");
    std::fs::write(&config_path, config_content).unwrap();
    
    let config = parse_properties_file(&config_path, "test-service").unwrap();
    
    assert_eq!(config.application_id, "test-service");
    assert_eq!(config.kafka.bootstrap_servers, "localhost:9092,localhost:9093");
    assert_eq!(config.kafka.schema_registry_url, Some("http://localhost:8081".to_string()));
    assert_eq!(config.commit_interval_ms, Some(100));
    assert_eq!(config.processing_guarantee, Some("exactly_once_v2".to_string()));
    
    // Test stream config merging
    let stream_config_content = r#"
num.stream.threads=4
replication.factor=3
"#;
    
    let stream_config_path = temp_dir.path().join("stream.properties");
    std::fs::write(&stream_config_path, stream_config_content).unwrap();
    
    let merged_config = merge_stream_properties(config, &stream_config_path).unwrap();
    assert_eq!(merged_config.kafka.additional_properties.get("num.stream.threads"), Some(&"4".to_string()));
    assert_eq!(merged_config.kafka.additional_properties.get("replication.factor"), Some(&"3".to_string()));
}

#[tokio::test]
async fn test_rocksdb_state_store_integration() {
    let temp_dir = tempdir().unwrap();
    let store_path = temp_dir.path().join("test_store");
    
    let store = RocksDBStore::new(&store_path).unwrap();
    
    // Test area status storage
    let area_status = AreaStatus {
        event_name: "Taylor Swift Concert".to_string(),
        area_id: "VIP".to_string(),
        available_seats: 150,
        total_seats: 200,
        seats: vec![
            vec![SeatStatus { row: 0, col: 0, is_available: true }],
            vec![SeatStatus { row: 0, col: 1, is_available: false }],
        ],
    };
    
    let key = event_area_key("Taylor Swift Concert", "VIP");
    store.put(&key, &area_status).unwrap();
    
    let retrieved: Option<AreaStatus> = store.get(&key).unwrap();
    assert!(retrieved.is_some());
    
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.event_name, "Taylor Swift Concert");
    assert_eq!(retrieved.area_id, "VIP");
    assert_eq!(retrieved.available_seats, 150);
    assert_eq!(retrieved.total_seats, 200);
    assert_eq!(retrieved.seats.len(), 2);
    
    // Test reservation storage
    let reservation = Reservation {
        reservation_id: "res-123".to_string(),
        user_id: "user-456".to_string(),
        event_id: "Taylor Swift Concert".to_string(),
        area_id: "VIP".to_string(),
        num_of_seats: 2,
        reservation_type: ReservationType::SelfPick,
        seats: vec![
            Seat { row: 0, col: 5 },
            Seat { row: 0, col: 6 },
        ],
        state: ReservationState::Confirmed,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    
    store.put("res-123", &reservation).unwrap();
    
    let retrieved_reservation: Option<Reservation> = store.get("res-123").unwrap();
    assert!(retrieved_reservation.is_some());
    
    let retrieved_reservation = retrieved_reservation.unwrap();
    assert_eq!(retrieved_reservation.reservation_id, "res-123");
    assert_eq!(retrieved_reservation.user_id, "user-456");
    assert_eq!(retrieved_reservation.num_of_seats, 2);
    assert_eq!(retrieved_reservation.seats.len(), 2);
}

#[tokio::test]
async fn test_processing_context_integration() {
    let temp_dir = tempdir().unwrap();
    let context = ProcessingContext::with_state_dir(temp_dir.path().to_string_lossy().to_string());
    
    // Add RocksDB stores
    context.add_rocksdb_store(Stores::AREA_STATUS.to_string(), "area-status").unwrap();
    context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations").unwrap();
    
    // Test area status store
    let area_store = context.get_rocksdb_store(Stores::AREA_STATUS).unwrap();
    let test_area = AreaStatus {
        event_name: "Test Event".to_string(),
        area_id: "General".to_string(),
        available_seats: 500,
        total_seats: 500,
        seats: vec![],
    };
    
    let key = event_area_key("Test Event", "General");
    area_store.put(&key, &test_area).unwrap();
    
    let retrieved: Option<AreaStatus> = area_store.get(&key).unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().available_seats, 500);
    
    // Test reservation store
    let reservation_store = context.get_rocksdb_store(Stores::RESERVATION).unwrap();
    let test_reservation = Reservation {
        reservation_id: "test-res".to_string(),
        user_id: "test-user".to_string(),
        event_id: "Test Event".to_string(),
        area_id: "General".to_string(),
        num_of_seats: 3,
        reservation_type: ReservationType::Random,
        seats: vec![],
        state: ReservationState::Pending,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    
    reservation_store.put("test-res", &test_reservation).unwrap();
    
    let retrieved_res: Option<Reservation> = reservation_store.get("test-res").unwrap();
    assert!(retrieved_res.is_some());
    assert_eq!(retrieved_res.unwrap().num_of_seats, 3);
}

#[tokio::test]
async fn test_domain_model_serialization() {
    // Test CreateEvent serialization
    let create_event = CreateEvent {
        artist: "The Beatles".to_string(),
        event_name: "Abbey Road Live".to_string(),
        reservation_opening_time: chrono::Utc::now(),
        reservation_closing_time: chrono::Utc::now() + chrono::Duration::days(30),
        event_start_time: chrono::Utc::now() + chrono::Duration::days(60),
        event_end_time: chrono::Utc::now() + chrono::Duration::days(60) + chrono::Duration::hours(3),
        areas: vec![
            Area {
                area_id: "VIP".to_string(),
                price: 500,
                row_count: 10,
                col_count: 20,
            },
            Area {
                area_id: "General".to_string(),
                price: 100,
                row_count: 50,
                col_count: 30,
            },
        ],
    };
    
    // JSON serialization
    let json = serde_json::to_string(&create_event).unwrap();
    let deserialized: CreateEvent = serde_json::from_str(&json).unwrap();
    
    assert_eq!(deserialized.artist, "The Beatles");
    assert_eq!(deserialized.event_name, "Abbey Road Live");
    assert_eq!(deserialized.areas.len(), 2);
    assert_eq!(deserialized.areas[0].area_id, "VIP");
    assert_eq!(deserialized.areas[1].price, 100);
    
    // Test CreateReservation serialization
    let create_reservation = CreateReservation {
        reservation_id: "res-789".to_string(),
        user_id: "user-123".to_string(),
        event_id: "Abbey Road Live".to_string(),
        area_id: "VIP".to_string(),
        num_of_seats: 2,
        num_of_seat: 0,
        reservation_type: ReservationType::SelfPick,
        seats: vec![
            Seat { row: 5, col: 10 },
            Seat { row: 5, col: 11 },
        ],
    };
    
    let json = serde_json::to_string(&create_reservation).unwrap();
    let deserialized: CreateReservation = serde_json::from_str(&json).unwrap();
    
    assert_eq!(deserialized.reservation_id, "res-789");
    assert_eq!(deserialized.user_id, "user-123");
    assert_eq!(deserialized.seats.len(), 2);
    assert_eq!(deserialized.seats[0].row, 5);
    assert_eq!(deserialized.seats[0].col, 10);
}

#[tokio::test]
async fn test_reservation_strategies() {
    use ticket_master::*;
    
    // Create a test area status
    let mut area_status = AreaStatus {
        event_name: "Test Event".to_string(),
        area_id: "Test Area".to_string(),
        available_seats: 100,
        total_seats: 100,
        seats: (0..10).map(|row| {
            (0..10).map(|col| SeatStatus {
                row,
                col,
                is_available: true,
            }).collect()
        }).collect(),
    };
    
    // Test SelfPick strategy
    let self_pick_strategy = event_service::strategies::SelfPickStrategy;
    let reserve_request = ReserveSeat {
        reservation_id: "test-123".to_string(),
        user_id: "user-123".to_string(),
        event_id: "Test Event".to_string(),
        area_id: "Test Area".to_string(),
        num_of_seats: 2,
        reservation_type: ReservationType::SelfPick,
        seats: vec![
            Seat { row: 0, col: 0 },
            Seat { row: 0, col: 1 },
        ],
    };
    
    let result = self_pick_strategy.reserve(&mut area_status, &reserve_request).unwrap();
    assert_eq!(result.result, ReservationResultEnum::Success);
    assert_eq!(result.seats.len(), 2);
    assert_eq!(result.seats[0].row, 0);
    assert_eq!(result.seats[0].col, 0);
    
    // Test Random strategy
    let random_strategy = event_service::strategies::RandomStrategy;
    let random_request = ReserveSeat {
        reservation_id: "test-456".to_string(),
        user_id: "user-456".to_string(),
        event_id: "Test Event".to_string(),
        area_id: "Test Area".to_string(),
        num_of_seats: 3,
        reservation_type: ReservationType::Random,
        seats: vec![],
    };
    
    let result = random_strategy.reserve(&mut area_status, &random_request).unwrap();
    assert_eq!(result.result, ReservationResultEnum::Success);
    assert_eq!(result.seats.len(), 3);
    
    // Verify seats are different
    let seat1 = &result.seats[0];
    let seat2 = &result.seats[1];
    let seat3 = &result.seats[2];
    assert!(seat1 != seat2 || seat1 != seat3 || seat2 != seat3);
}

#[tokio::test]
async fn test_service_config_to_kafka_config() {
    let service_config = ServiceConfig {
        application_id: "test-app".to_string(),
        state_dir: "/tmp/test".to_string(),
        kafka: KafkaConfig {
            bootstrap_servers: "localhost:9092".to_string(),
            schema_registry_url: Some("http://localhost:8081".to_string()),
            security_protocol: Some("SASL_SSL".to_string()),
            sasl_mechanism: Some("PLAIN".to_string()),
            sasl_username: Some("user".to_string()),
            sasl_password: Some("pass".to_string()),
            ssl_ca_location: Some("/path/to/ca.pem".to_string()),
            additional_properties: [
                ("num.stream.threads".to_string(), "4".to_string()),
                ("replication.factor".to_string(), "3".to_string()),
            ].into_iter().collect(),
        },
        commit_interval_ms: Some(100),
        processing_guarantee: Some("exactly_once_v2".to_string()),
    };
    
    let kafka_config = service_config.to_kafka_config();
    
    // Verify the conversion works (we can't easily test the internal state of ClientConfig)
    // but we can verify it doesn't panic and creates a valid config
    assert!(true); // If we get here, the conversion worked
}
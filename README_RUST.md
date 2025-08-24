# Ticket Master - Rust Migration

This is a Rust migration of the Java-based Kafka Streams ticket reservation system. The system consists of three main services:

## Architecture

### Services

1. **Event Service** (`event-service/`) - Manages event creation and seat reservations
2. **Reservation Service** (`reservation-service/`) - Handles reservation requests and state management
3. **Ticket Service** (`ticket-service/`) - REST API for external interactions

### Key Components

- **Domain Models** - Rust structs representing events, reservations, and area status
- **Kafka Infrastructure** - Producer/consumer abstractions for Kafka messaging
- **State Stores** - In-memory state management (using DashMap)
- **Reservation Strategies** - Self-pick and random seat allocation algorithms

## Building and Running

### Prerequisites

- Rust 1.70+ 
- Kafka cluster running
- Schema Registry (optional, for Avro support)

### Build

```bash
# Build all services
cargo build --release

# Build individual services
cargo build -p event-service --release
cargo build -p reservation-service --release  
cargo build -p ticket-service --release
```

### Run Services

```bash
# Event Service
RUST_LOG=info ./target/release/event-service --config appConfig/client.dev.properties

# Reservation Service  
RUST_LOG=info ./target/release/reservation-service --config appConfig/client.dev.properties

# Ticket Service (REST API)
RUST_LOG=info ./target/release/ticket-service --config appConfig/client.dev.properties --port 8080
```

## API Examples

### Create Event

```bash
curl -X POST http://localhost:8080/events \
  -H "Content-Type: application/json" \
  -d '{
    "artist": "Taylor Swift",
    "event_name": "Eras Tour",
    "reservation_opening_time": "2024-01-01T00:00:00Z",
    "reservation_closing_time": "2024-06-01T00:00:00Z", 
    "event_start_time": "2024-06-15T19:00:00Z",
    "event_end_time": "2024-06-15T23:00:00Z",
    "areas": [
      {
        "area_id": "VIP",
        "price": 500,
        "row_count": 10,
        "col_count": 20
      }
    ]
  }'
```

### Create Reservation

```bash
curl -X POST http://localhost:8080/reservations \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "user123",
    "event_id": "Eras Tour", 
    "area_id": "VIP",
    "num_of_seats": 2,
    "reservation_type": "random"
  }'
```

### Self-Pick Reservation

```bash
curl -X POST http://localhost:8080/reservations \
  -H "Content-Type: application/json" \
  -d '{
    "user_id": "user456",
    "event_id": "Eras Tour",
    "area_id": "VIP", 
    "num_of_seats": 2,
    "reservation_type": "self_pick",
    "seats": [
      {"row": 0, "col": 5},
      {"row": 0, "col": 6}
    ]
  }'
```

## Migration Notes

### Key Differences from Java Version

1. **Async/Await** - Uses Tokio for async runtime instead of blocking I/O
2. **Memory Management** - Rust's ownership system eliminates garbage collection
3. **Error Handling** - Uses Result types instead of exceptions
4. **State Stores** - Currently uses in-memory DashMap instead of RocksDB
5. **Serialization** - Uses serde_json instead of Avro (can be upgraded)

### TODO Items

- [ ] Add RocksDB-based state stores for persistence
- [ ] Implement proper Avro serialization with schema registry
- [ ] Add comprehensive error handling and retry logic
- [ ] Implement proper configuration file parsing (Java properties)
- [ ] Add metrics and monitoring
- [ ] Add integration tests
- [ ] Implement proper LRU cache for area status
- [ ] Add graceful shutdown handling
- [ ] Implement exactly-once processing guarantees
- [ ] Add query endpoints for state stores

### Performance Considerations

- **Memory Usage** - Rust typically uses less memory than Java
- **Startup Time** - Much faster startup compared to JVM
- **Throughput** - Should achieve similar or better throughput
- **Latency** - Lower latency due to no GC pauses

## Configuration

The services expect configuration files in Java properties format. For now, they use default configurations, but this should be extended to parse actual property files.

Example configuration structure:
```
bootstrap.servers=localhost:9092
schema.registry.url=http://localhost:8081
application.id=event-service
processing.guarantee=exactly_once_v2
```

## Testing

```bash
# Run all tests
cargo test

# Run tests for specific service
cargo test -p event-service
```

## Deployment

The Rust services can be deployed using the existing Kubernetes configurations with minimal changes to the deployment manifests. The main differences would be:

1. Different container images (Rust binaries instead of Java JARs)
2. Potentially different resource requirements
3. Different health check endpoints

## Contributing

When contributing to the Rust migration:

1. Follow Rust naming conventions (snake_case)
2. Use proper error handling with Result types
3. Add comprehensive documentation
4. Write unit tests for new functionality
5. Ensure thread safety for concurrent operations
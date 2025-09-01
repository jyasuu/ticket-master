# Scalable Docker Compose Setup for Ticket Master Services

This directory contains scalable Docker Compose configurations for both Java and Rust implementations of the Ticket Master services, designed to handle high-load scenarios with multiple replicas and proper load balancing.

## 🚀 Quick Start

### Prerequisites
- Docker 20.10+
- Docker Compose 2.0+
- 8GB+ RAM recommended
- 20GB+ disk space

### Start Java Services (3 Ticket Service Replicas)
```bash
./scale.sh java up
```

### Start Rust Services (3 Ticket Service Replicas with Distributed Features)
```bash
./scale.sh rust up
```

## 📋 Architecture Overview

### Java Scalable Setup (`docker-compose.scalable.yml`)
```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Nginx LB      │    │   Ticket Svc 1  │    │   Ticket Svc 2  │
│   :8080         │────┤   :8080         │    │   :8080         │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                                │                       │
                       ┌─────────────────┐    ┌─────────────────┐
                       │   Ticket Svc 3  │    │   Event Svc 1   │
                       │   :8080         │    │                 │
                       └─────────────────┘    └─────────────────┘
                                │                       │
                       ┌─────────────────┐    ┌─────────────────┐
                       │   Event Svc 2   │    │ Reservation 1   │
                       │                 │    │                 │
                       └─────────────────┘    └─────────────────┘
                                │                       │
                       ┌─────────────────┐    ┌─────────────────┐
                       │ Reservation 2   │    │   Kafka Cluster │
                       │                 │    │   (3 brokers)   │
                       └─────────────────┘    └─────────────────┘
```

### Rust Scalable Setup (`docker-compose.rust.scalable.yml`)
```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Nginx LB      │    │ Rust Ticket 1  │    │ Rust Ticket 2  │
│   :8080         │────┤ (Distributed)   │◄──►│ (Distributed)   │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                                │                       │
                       ┌─────────────────┐              │
                       │ Rust Ticket 3  │◄─────────────┘
                       │ (Distributed)   │
                       └─────────────────┘
                                │
                       ┌─────────────────┐
                       │   Kafka Cluster │
                       │   (9 partitions)│
                       └─────────────────┘
```

## 🔧 Service Configurations

### Java Services
- **Ticket Service**: 3 replicas with load balancing
- **Event Service**: 2 replicas with Kafka Streams
- **Reservation Service**: 2 replicas with state stores
- **Memory**: 2-3GB per service
- **Processing**: `exactly_once_v2` guarantee

### Rust Services  
- **Ticket Service**: 3 replicas with **distributed querying**
- **Event Service**: 2 replicas with enhanced topology
- **Reservation Service**: 2 replicas with enhanced processing
- **Memory**: Lower footprint (~500MB per service)
- **Features**: Real-time updates, timeout handling, inter-service communication

## 📊 Kafka Configuration

### Partitioning Strategy
- **Java**: 6 partitions per topic (2x replicas)
- **Rust**: 9 partitions per topic (3x replicas)
- **Replication Factor**: 3 (high availability)
- **Min In-Sync Replicas**: 2

### Topics
```
command-event-create-event       (6/9 partitions, RF=3)
command-event-reserve-seat       (6/9 partitions, RF=3)
command-reservation-create-reservation (6/9 partitions, RF=3)
response-reservation-result      (6/9 partitions, RF=3)
state-event-area-status         (6/9 partitions, RF=3, compacted)
state-user-reservation          (6/9 partitions, RF=3, compacted)
```

## 🌐 Load Balancing

### Nginx Configuration
- **Algorithm**: `least_conn` for optimal distribution
- **Health Checks**: Automatic failover on service failure
- **Timeouts**: Optimized for different operation types
- **Logging**: Detailed request tracking with upstream info

### Access Points
- **Main API**: http://localhost:8080
- **Nginx Status**: http://localhost:8080/nginx_status
- **Health Check**: http://localhost:8080/health

## 🛠️ Management Commands

### Basic Operations
```bash
# Start services
./scale.sh java up      # Java implementation
./scale.sh rust up      # Rust implementation

# Stop services
./scale.sh java down
./scale.sh rust down

# Check status
./scale.sh java status
./scale.sh rust status
```

### Scaling Operations
```bash
# Scale ticket service to 5 replicas
./scale.sh java scale ticket-service 5
./scale.sh rust scale ticket-service-rust 5

# Scale event service to 3 replicas
./scale.sh java scale event-service 3
./scale.sh rust scale event-service-rust 3
```

### Monitoring & Debugging
```bash
# View all logs
./scale.sh java logs
./scale.sh rust logs

# View specific service logs
./scale.sh java logs ticket-service-1
./scale.sh rust logs ticket-service-rust-2

# Run connectivity tests
./scale.sh java test
./scale.sh rust test
```

## 📈 Monitoring & Observability

### Available Dashboards
- **Kafka UI**: http://localhost:9000
- **Jaeger Tracing**: http://localhost:16686  
- **Prometheus**: http://localhost:9090
- **Nginx Status**: http://localhost:8080/nginx_status

### Key Metrics to Monitor
- **Request Distribution**: Check Nginx logs for load balancing
- **Kafka Consumer Lag**: Monitor in Kafka UI
- **Service Health**: Individual service health endpoints
- **Resource Usage**: CPU/Memory via `docker stats`

## 🧪 Testing Scalability

### Load Testing Scenarios

#### 1. Create Event (Distributed to Event Services)
```bash
# Test event creation across replicas
for i in {1..10}; do
  curl -X POST http://localhost:8080/events \
    -H "Content-Type: application/json" \
    -d '{
      "event_name": "Concert'$i'",
      "artist": "Artist'$i'",
      "reservation_opening_time": "2024-01-01T10:00:00Z",
      "reservation_closing_time": "2024-01-01T23:59:59Z",
      "event_start_time": "2024-01-02T19:00:00Z",
      "event_end_time": "2024-01-02T22:00:00Z",
      "areas": [{"area_id": "A", "price": 100, "row_count": 10, "col_count": 10}]
    }'
done
```

#### 2. Create Reservations (Distributed Processing)
```bash
# Test reservation creation across replicas
for i in {1..20}; do
  curl -X POST http://localhost:8080/reservations \
    -H "Content-Type: application/json" \
    -d '{
      "user_id": "user'$i'",
      "event_id": "Concert1",
      "area_id": "A",
      "num_of_seats": 2,
      "reservation_type": "random"
    }'
done
```

#### 3. Get Reservations (Distributed Queries)
```bash
# Test distributed querying - requests may hit different replicas
for reservation_id in $(curl -s http://localhost:8080/reservations | jq -r '.data[].reservation_id'); do
  curl -s http://localhost:8080/reservations/$reservation_id
done
```

### Verify Load Distribution
```bash
# Check Nginx access logs to see request distribution
docker-compose -f docker-compose.scalable.yml logs nginx-lb | grep upstream_addr

# Check individual service logs
./scale.sh java logs ticket-service-1 | grep "Processing request"
./scale.sh java logs ticket-service-2 | grep "Processing request"  
./scale.sh java logs ticket-service-3 | grep "Processing request"
```

## 🔍 Troubleshooting

### Common Issues

#### 1. Services Not Starting
```bash
# Check service health
./scale.sh java status

# Check individual service logs
./scale.sh java logs ticket-service-1

# Verify Kafka connectivity
docker-compose -f docker-compose.scalable.yml exec kafka-1 kafka-topics --list --bootstrap-server localhost:9092
```

#### 2. Load Balancer Issues
```bash
# Check Nginx configuration
docker-compose -f docker-compose.scalable.yml exec nginx-lb nginx -t

# Check upstream health
curl http://localhost:8080/nginx_status
```

#### 3. Kafka Issues
```bash
# Check Kafka cluster health
docker-compose -f docker-compose.scalable.yml exec kafka-1 kafka-broker-api-versions --bootstrap-server kafka-1:19092,kafka-2:19092,kafka-3:19092

# Check topic partitions
docker-compose -f docker-compose.scalable.yml exec kafka-1 kafka-topics --describe --bootstrap-server kafka-1:19092
```

#### 4. State Store Issues
```bash
# Check state directories
ls -la state/

# Clear state (WARNING: loses all data)
rm -rf state/
./scale.sh java down
./scale.sh java up
```

## 🚀 Performance Optimization

### Java Services
- **JVM Flags**: ZGC with generational collection
- **Memory**: 2-3GB heap per service
- **Threads**: Optimized for virtual threads (Java 21+)

### Rust Services  
- **Memory**: ~500MB per service
- **Async Runtime**: Tokio with optimized thread pool
- **HTTP Client**: Connection pooling for inter-service calls

### Kafka Optimization
- **Batch Settings**: Optimized `linger.ms` and `batch.size`
- **Compression**: Snappy compression enabled
- **Replication**: 3-broker cluster with proper ISR settings

## 📚 Additional Resources

### Configuration Files
- `nginx.conf` - Java services load balancer config
- `nginx.rust.conf` - Rust services load balancer config  
- `appConfig/client.docker.properties` - Kafka connection settings
- `appConfig/*/stream.properties` - Service-specific stream settings

### Scripts
- `scale.sh` - Main management script
- Individual service health check endpoints
- Kafka topic initialization scripts

### Monitoring
- Prometheus configuration in `otel/prometheus/config.yml`
- Jaeger for distributed tracing
- Kafka UI for cluster monitoring

## 🤝 Contributing

When adding new services or modifying scaling:

1. Update the appropriate Docker Compose file
2. Add service to Nginx upstream configuration  
3. Update the `scale.sh` script with new service names
4. Test scaling scenarios
5. Update this documentation

## 📄 License

This scalable setup inherits the license from the main Ticket Master project.
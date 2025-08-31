# Distributed Ticket Service

This enhanced Rust implementation provides distributed features matching the Java service's capabilities.

## Features Comparison

### Enhanced Rust Implementation (Distributed Mode)

✅ **Distributed Querying**: Can query remote service instances for data not available locally
✅ **Async Processing**: Non-blocking operations with proper timeout handling  
✅ **Real-time Updates**: Waits for reservation updates via Kafka streams when data not immediately available
✅ **Timeout Handling**: Configurable timeouts (default 10 seconds) with proper cleanup
✅ **Outstanding Requests Tracking**: Manages pending requests waiting for real-time updates
✅ **Inter-service Communication**: HTTP/2 client for querying other service instances
✅ **Enhanced Error Handling**: Detailed error responses with proper HTTP status codes
✅ **Observability**: Structured logging with tracing spans and metrics endpoints
✅ **Health Checks**: Enhanced health checks considering service state
✅ **Retry Logic**: Automatic retry when metadata/stores are unavailable

### Standard Rust Implementation

❌ **Local Only**: Simple local store queries without distributed capabilities
❌ **Synchronous**: Blocking operations without timeout handling
❌ **No Real-time Updates**: Returns immediately if data not found
❌ **Basic Error Handling**: Simple Result types without detailed HTTP responses

## Usage

### Standard Mode (Original)
```bash
cargo run -- --port 8080 --config ../client.dev.properties
```

### Distributed Mode (Enhanced)
```bash
cargo run -- --port 8080 --config ../client.dev.properties --distributed --hostname localhost
```

## API Endpoints

### Standard Mode
- `GET /reservations/{id}` - Simple local lookup
- `GET /health` - Basic health check

### Distributed Mode (Additional)
- `GET /reservations/{id}` - Enhanced with distributed querying and real-time updates
- `GET /reservations/{id}/timeout/{seconds}` - Custom timeout support
- `GET /health` - Enhanced health check with service state validation
- `GET /metrics/outstanding-requests` - Monitor pending requests

## Enhanced Features Detail

### 1. Distributed Querying
When a reservation is not found locally, the service:
1. Queries Kafka metadata to determine which host has the data
2. Makes HTTP request to the appropriate service instance
3. Returns the result with proper error handling

### 2. Real-time Updates
If a reservation doesn't exist yet, the service:
1. Registers an outstanding request
2. Waits for the reservation to arrive via Kafka stream
3. Automatically responds when data becomes available
4. Cleans up expired requests after timeout

### 3. Timeout Management
- Default 10-second timeout (matching Java implementation)
- Custom timeout via `/reservations/{id}/timeout/{seconds}` endpoint
- Automatic cleanup of expired outstanding requests
- Proper timeout error responses

### 4. Error Handling
- `503 Service Unavailable` when stores not ready
- `408 Request Timeout` for expired requests
- `404 Not Found` for missing reservations
- `500 Internal Server Error` for system errors

### 5. Observability
- Structured logging with reservation IDs and operation context
- Tracing spans for distributed operations
- Metrics endpoint for monitoring outstanding requests
- Performance monitoring capabilities

## Configuration

The distributed service uses the same configuration as the standard service but adds:

```properties
# Optional: Configure HTTP client timeouts
http.client.timeout.seconds=10
http.client.pool.max.idle.per.host=10
```

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Client        │    │   Service A     │    │   Service B     │
│                 │    │   (localhost:   │    │   (localhost:   │
│                 │    │    8080)        │    │    8081)        │
└─────────┬───────┘    └─────────┬───────┘    └─────────┬───────┘
          │                      │                      │
          │ GET /reservations/123│                      │
          ├─────────────────────►│                      │
          │                      │                      │
          │                      │ Check local store    │
          │                      │ (not found)          │
          │                      │                      │
          │                      │ Query metadata       │
          │                      │ (data on Service B)  │
          │                      │                      │
          │                      │ HTTP GET /reservations/123
          │                      ├─────────────────────►│
          │                      │                      │
          │                      │ ◄─────────────────────┤
          │                      │ Return reservation   │
          │ ◄─────────────────────┤                      │
          │ Return reservation   │                      │
          │                      │                      │
```

## Performance Considerations

1. **Connection Pooling**: HTTP client uses connection pooling for efficient inter-service communication
2. **Async Operations**: All operations are non-blocking to handle high concurrency
3. **Memory Management**: Outstanding requests are automatically cleaned up to prevent memory leaks
4. **Caching**: Local RocksDB stores provide fast local access when data is available

## Monitoring

Monitor the service using:
- `GET /metrics/outstanding-requests` - Number of pending requests
- Log analysis for timeout patterns and error rates
- Health check endpoint for service readiness
- Kafka consumer lag monitoring

## Migration from Java

The enhanced Rust implementation provides feature parity with the Java service:

| Java Feature | Rust Implementation | Status |
|-------------|-------------------|---------|
| AsyncResponse with timeout | tokio::time::timeout + oneshot channels | ✅ Complete |
| Outstanding requests map | HashMap with cleanup task | ✅ Complete |
| Distributed querying | HTTP client + metadata lookup | ✅ Complete |
| Real-time updates | Kafka consumer + outstanding request completion | ✅ Complete |
| Error handling | Enhanced error types + HTTP status codes | ✅ Complete |
| Observability | tracing + structured logging | ✅ Complete |
| Virtual threads | Tokio async runtime | ✅ Complete |

This implementation maintains the same behavior and performance characteristics as the Java version while leveraging Rust's memory safety and performance benefits.
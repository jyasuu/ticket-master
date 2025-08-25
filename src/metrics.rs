use prometheus::{
    Counter, Histogram, Gauge, Registry, Opts, HistogramOpts,
    register_counter_with_registry, register_histogram_with_registry, 
    register_gauge_with_registry, Encoder, TextEncoder,
};
use std::sync::Arc;
use crate::Result;

/// Metrics collector for the ticket master system
#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    
    // Kafka metrics
    pub kafka_messages_sent: Counter,
    pub kafka_messages_received: Counter,
    pub kafka_send_duration: Histogram,
    pub kafka_errors: Counter,
    
    // State store metrics
    pub state_store_reads: Counter,
    pub state_store_writes: Counter,
    pub state_store_read_duration: Histogram,
    pub state_store_write_duration: Histogram,
    pub state_store_size: Gauge,
    
    // Business metrics
    pub events_created: Counter,
    pub reservations_created: Counter,
    pub reservations_successful: Counter,
    pub reservations_failed: Counter,
    pub seats_reserved: Counter,
    pub available_seats: Gauge,
    
    // Service metrics
    pub service_uptime: Gauge,
    pub active_connections: Gauge,
    pub request_duration: Histogram,
    pub error_rate: Counter,
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let registry = Arc::new(Registry::new());
        
        // Kafka metrics
        let kafka_messages_sent = register_counter_with_registry!(
            Opts::new("kafka_messages_sent_total", "Total number of Kafka messages sent"),
            registry
        )?;
        
        let kafka_messages_received = register_counter_with_registry!(
            Opts::new("kafka_messages_received_total", "Total number of Kafka messages received"),
            registry
        )?;
        
        let kafka_send_duration = register_histogram_with_registry!(
            HistogramOpts::new("kafka_send_duration_seconds", "Time spent sending Kafka messages")
                .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
            registry
        )?;
        
        let kafka_errors = register_counter_with_registry!(
            Opts::new("kafka_errors_total", "Total number of Kafka errors"),
            registry
        )?;
        
        // State store metrics
        let state_store_reads = register_counter_with_registry!(
            Opts::new("state_store_reads_total", "Total number of state store reads"),
            registry
        )?;
        
        let state_store_writes = register_counter_with_registry!(
            Opts::new("state_store_writes_total", "Total number of state store writes"),
            registry
        )?;
        
        let state_store_read_duration = register_histogram_with_registry!(
            HistogramOpts::new("state_store_read_duration_seconds", "Time spent reading from state store")
                .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1]),
            registry
        )?;
        
        let state_store_write_duration = register_histogram_with_registry!(
            HistogramOpts::new("state_store_write_duration_seconds", "Time spent writing to state store")
                .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1]),
            registry
        )?;
        
        let state_store_size = register_gauge_with_registry!(
            Opts::new("state_store_size_bytes", "Size of state store in bytes"),
            registry
        )?;
        
        // Business metrics
        let events_created = register_counter_with_registry!(
            Opts::new("events_created_total", "Total number of events created"),
            registry
        )?;
        
        let reservations_created = register_counter_with_registry!(
            Opts::new("reservations_created_total", "Total number of reservations created"),
            registry
        )?;
        
        let reservations_successful = register_counter_with_registry!(
            Opts::new("reservations_successful_total", "Total number of successful reservations"),
            registry
        )?;
        
        let reservations_failed = register_counter_with_registry!(
            Opts::new("reservations_failed_total", "Total number of failed reservations"),
            registry
        )?;
        
        let seats_reserved = register_counter_with_registry!(
            Opts::new("seats_reserved_total", "Total number of seats reserved"),
            registry
        )?;
        
        let available_seats = register_gauge_with_registry!(
            Opts::new("available_seats", "Current number of available seats"),
            registry
        )?;
        
        // Service metrics
        let service_uptime = register_gauge_with_registry!(
            Opts::new("service_uptime_seconds", "Service uptime in seconds"),
            registry
        )?;
        
        let active_connections = register_gauge_with_registry!(
            Opts::new("active_connections", "Number of active connections"),
            registry
        )?;
        
        let request_duration = register_histogram_with_registry!(
            HistogramOpts::new("request_duration_seconds", "Time spent processing requests")
                .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
            registry
        )?;
        
        let error_rate = register_counter_with_registry!(
            Opts::new("errors_total", "Total number of errors"),
            registry
        )?;
        
        Ok(Self {
            registry,
            kafka_messages_sent,
            kafka_messages_received,
            kafka_send_duration,
            kafka_errors,
            state_store_reads,
            state_store_writes,
            state_store_read_duration,
            state_store_write_duration,
            state_store_size,
            events_created,
            reservations_created,
            reservations_successful,
            reservations_failed,
            seats_reserved,
            available_seats,
            service_uptime,
            active_connections,
            request_duration,
            error_rate,
        })
    }
    
    /// Export metrics in Prometheus format
    pub fn export(&self) -> Result<String> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }
    
    /// Record a Kafka message send operation
    pub fn record_kafka_send(&self, duration: std::time::Duration, success: bool) {
        self.kafka_messages_sent.inc();
        self.kafka_send_duration.observe(duration.as_secs_f64());
        if !success {
            self.kafka_errors.inc();
        }
    }
    
    /// Record a state store operation
    pub fn record_state_store_read(&self, duration: std::time::Duration) {
        self.state_store_reads.inc();
        self.state_store_read_duration.observe(duration.as_secs_f64());
    }
    
    pub fn record_state_store_write(&self, duration: std::time::Duration) {
        self.state_store_writes.inc();
        self.state_store_write_duration.observe(duration.as_secs_f64());
    }
    
    /// Record business events
    pub fn record_event_created(&self) {
        self.events_created.inc();
    }
    
    pub fn record_reservation_attempt(&self, success: bool, seats_count: i32) {
        self.reservations_created.inc();
        if success {
            self.reservations_successful.inc();
            self.seats_reserved.inc_by(seats_count as f64);
        } else {
            self.reservations_failed.inc();
        }
    }
    
    pub fn update_available_seats(&self, count: i32) {
        self.available_seats.set(count as f64);
    }
    
    /// Record service metrics
    pub fn record_request(&self, duration: std::time::Duration, success: bool) {
        self.request_duration.observe(duration.as_secs_f64());
        if !success {
            self.error_rate.inc();
        }
    }
    
    pub fn update_active_connections(&self, count: i32) {
        self.active_connections.set(count as f64);
    }
    
    pub fn update_uptime(&self, uptime: std::time::Duration) {
        self.service_uptime.set(uptime.as_secs_f64());
    }
}

/// Metrics middleware for HTTP requests
pub async fn metrics_middleware<B>(
    req: axum::extract::Request<B>,
    next: axum::middleware::Next<B>,
) -> axum::response::Response {
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed();
    
    // Record metrics (would need access to metrics instance)
    // This is a simplified version - in practice you'd inject metrics via state
    tracing::info!("Request completed in {:?}", duration);
    
    response
}

/// Health check endpoint that includes metrics
pub async fn health_with_metrics(
    metrics: Option<axum::extract::State<Arc<Metrics>>>,
) -> axum::response::Result<String> {
    let mut health_info = serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "ticket-master"
    });
    
    if let Some(metrics) = metrics {
        if let Ok(metrics_data) = metrics.export() {
            health_info["metrics_available"] = serde_json::Value::Bool(true);
            // Don't include full metrics in health check to keep it lightweight
        }
    }
    
    Ok(health_info.to_string())
}

/// Metrics endpoint for Prometheus scraping
pub async fn metrics_endpoint(
    axum::extract::State(metrics): axum::extract::State<Arc<Metrics>>,
) -> axum::response::Result<String> {
    match metrics.export() {
        Ok(metrics_data) => Ok(metrics_data),
        Err(e) => {
            tracing::error!("Failed to export metrics: {}", e);
            Err(axum::response::ErrorResponse::from("Failed to export metrics"))
        }
    }
}
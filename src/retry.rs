use crate::{Result, TicketMasterError};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{warn, error, info};

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            ..Default::default()
        }
    }

    pub fn with_delays(max_attempts: u32, initial_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay,
            max_delay,
            ..Default::default()
        }
    }

    pub fn kafka_producer() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 1.5,
            jitter: true,
        }
    }

    pub fn state_store() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
            jitter: false,
        }
    }
}

/// Retry a future with exponential backoff
pub async fn retry_with_backoff<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 1;
    let mut delay = config.initial_delay;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    info!("Operation '{}' succeeded on attempt {}", operation_name, attempt);
                }
                return Ok(result);
            }
            Err(e) => {
                if attempt >= config.max_attempts {
                    error!(
                        "Operation '{}' failed after {} attempts. Final error: {}",
                        operation_name, config.max_attempts, e
                    );
                    return Err(e);
                }

                warn!(
                    "Operation '{}' failed on attempt {} ({}). Retrying in {:?}...",
                    operation_name, attempt, e, delay
                );

                sleep(delay).await;

                // Calculate next delay with exponential backoff
                delay = Duration::from_millis(
                    ((delay.as_millis() as f64) * config.backoff_multiplier) as u64
                ).min(config.max_delay);

                // Add jitter if enabled
                if config.jitter {
                    let jitter_ms = (delay.as_millis() as f64 * 0.1 * rand::random::<f64>()) as u64;
                    delay = delay + Duration::from_millis(jitter_ms);
                }

                attempt += 1;
            }
        }
    }
}

/// Retry specifically for Kafka operations
pub async fn retry_kafka_operation<F, Fut, T>(
    operation_name: &str,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    retry_with_backoff(&RetryConfig::kafka_producer(), operation_name, operation).await
}

/// Retry specifically for state store operations
pub async fn retry_state_store_operation<F, Fut, T>(
    operation_name: &str,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    retry_with_backoff(&RetryConfig::state_store(), operation_name, operation).await
}

/// Circuit breaker for preventing cascading failures
#[derive(Debug)]
pub struct CircuitBreaker {
    failure_count: std::sync::atomic::AtomicU32,
    last_failure_time: std::sync::Mutex<Option<std::time::Instant>>,
    failure_threshold: u32,
    recovery_timeout: Duration,
    state: std::sync::Mutex<CircuitBreakerState>,
}

#[derive(Debug, Clone, PartialEq)]
enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_timeout: Duration) -> Self {
        Self {
            failure_count: std::sync::atomic::AtomicU32::new(0),
            last_failure_time: std::sync::Mutex::new(None),
            failure_threshold,
            recovery_timeout,
            state: std::sync::Mutex::new(CircuitBreakerState::Closed),
        }
    }

    pub async fn call<F, Fut, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // Check if circuit breaker should allow the call
        if !self.should_allow_call() {
            return Err(TicketMasterError::InvalidArgument(
                "Circuit breaker is open".to_string()
            ));
        }

        match operation().await {
            Ok(result) => {
                self.on_success();
                Ok(result)
            }
            Err(e) => {
                self.on_failure();
                Err(e)
            }
        }
    }

    fn should_allow_call(&self) -> bool {
        let state = self.state.lock().unwrap();
        match *state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                // Check if recovery timeout has passed
                if let Some(last_failure) = *self.last_failure_time.lock().unwrap() {
                    if last_failure.elapsed() >= self.recovery_timeout {
                        drop(state);
                        *self.state.lock().unwrap() = CircuitBreakerState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    fn on_success(&self) {
        self.failure_count.store(0, std::sync::atomic::Ordering::Relaxed);
        *self.state.lock().unwrap() = CircuitBreakerState::Closed;
    }

    fn on_failure(&self) {
        let failure_count = self.failure_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *self.last_failure_time.lock().unwrap() = Some(std::time::Instant::now());

        if failure_count >= self.failure_threshold {
            *self.state.lock().unwrap() = CircuitBreakerState::Open;
            warn!("Circuit breaker opened after {} failures", failure_count);
        }
    }
}

/// Enhanced error context for better debugging
#[derive(Debug)]
pub struct ErrorContext {
    pub operation: String,
    pub component: String,
    pub metadata: std::collections::HashMap<String, String>,
}

impl ErrorContext {
    pub fn new(operation: &str, component: &str) -> Self {
        Self {
            operation: operation.to_string(),
            component: component.to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn add_metadata(&mut self, key: &str, value: &str) {
        self.metadata.insert(key.to_string(), value.to_string());
    }
}

/// Wrap errors with additional context
pub fn wrap_error(error: TicketMasterError, context: ErrorContext) -> TicketMasterError {
    let context_str = format!(
        "Operation: {}, Component: {}, Metadata: {:?}",
        context.operation, context.component, context.metadata
    );
    
    match error {
        TicketMasterError::InvalidArgument(msg) => {
            TicketMasterError::InvalidArgument(format!("{} | Context: {}", msg, context_str))
        }
        TicketMasterError::Kafka(e) => {
            TicketMasterError::InvalidArgument(format!("Kafka error: {} | Context: {}", e, context_str))
        }
        TicketMasterError::RocksDB(e) => {
            TicketMasterError::InvalidArgument(format!("RocksDB error: {} | Context: {}", e, context_str))
        }
        other => other,
    }
}
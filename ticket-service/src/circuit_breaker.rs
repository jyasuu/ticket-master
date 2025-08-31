use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn, error};

/// Circuit breaker states
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,    // Normal operation
    Open,      // Failing, reject requests
    HalfOpen,  // Testing if service recovered
}

/// Circuit breaker for remote service calls
/// Equivalent to Java's circuit breaker patterns used in microservices
#[derive(Debug)]
pub struct CircuitBreaker {
    state: Arc<Mutex<CircuitBreakerState>>,
    failure_threshold: u32,
    recovery_timeout: Duration,
    request_timeout: Duration,
}

#[derive(Debug)]
struct CircuitBreakerState {
    state: CircuitState,
    failure_count: u32,
    last_failure_time: Option<Instant>,
    success_count: u32,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure_time: None,
                success_count: 0,
            })),
            failure_threshold,
            recovery_timeout,
            request_timeout,
        }
    }

    /// Execute a request through the circuit breaker
    pub async fn execute<F, T, E>(&self, operation: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        // Check if we should allow the request
        if !self.should_allow_request().await {
            return Err(CircuitBreakerError::CircuitOpen);
        }

        // Execute with timeout
        match tokio::time::timeout(self.request_timeout, operation).await {
            Ok(Ok(result)) => {
                self.on_success().await;
                Ok(result)
            }
            Ok(Err(e)) => {
                self.on_failure().await;
                Err(CircuitBreakerError::RequestFailed(e))
            }
            Err(_) => {
                self.on_failure().await;
                Err(CircuitBreakerError::Timeout)
            }
        }
    }

    async fn should_allow_request(&self) -> bool {
        let mut state = self.state.lock().await;
        
        match state.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if we should transition to half-open
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= self.recovery_timeout {
                        info!("Circuit breaker transitioning to half-open state");
                        state.state = CircuitState::HalfOpen;
                        state.success_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    async fn on_success(&self) {
        let mut state = self.state.lock().await;
        
        match state.state {
            CircuitState::Closed => {
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= 3 {  // Require 3 successes to close
                    info!("Circuit breaker closing - service recovered");
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.last_failure_time = None;
                }
            }
            CircuitState::Open => {
                // Shouldn't happen, but reset if it does
                state.state = CircuitState::Closed;
                state.failure_count = 0;
                state.last_failure_time = None;
            }
        }
    }

    async fn on_failure(&self) {
        let mut state = self.state.lock().await;
        
        state.failure_count += 1;
        state.last_failure_time = Some(Instant::now());
        
        match state.state {
            CircuitState::Closed => {
                if state.failure_count >= self.failure_threshold {
                    warn!("Circuit breaker opening - failure threshold reached: {}", state.failure_count);
                    state.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                warn!("Circuit breaker reopening - test request failed");
                state.state = CircuitState::Open;
                state.success_count = 0;
            }
            CircuitState::Open => {
                // Already open, just update failure time
            }
        }
    }

    /// Get current circuit breaker state for monitoring
    pub async fn get_state(&self) -> CircuitState {
        let state = self.state.lock().await;
        state.state.clone()
    }

    /// Get circuit breaker metrics
    pub async fn get_metrics(&self) -> CircuitBreakerMetrics {
        let state = self.state.lock().await;
        CircuitBreakerMetrics {
            state: state.state.clone(),
            failure_count: state.failure_count,
            success_count: state.success_count,
            last_failure_time: state.last_failure_time,
        }
    }
}

#[derive(Debug)]
pub enum CircuitBreakerError<E> {
    CircuitOpen,
    Timeout,
    RequestFailed(E),
}

impl<E: std::fmt::Display> std::fmt::Display for CircuitBreakerError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitBreakerError::CircuitOpen => write!(f, "Circuit breaker is open"),
            CircuitBreakerError::Timeout => write!(f, "Request timed out"),
            CircuitBreakerError::RequestFailed(e) => write!(f, "Request failed: {}", e),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CircuitBreakerError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CircuitBreakerError::RequestFailed(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerMetrics {
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_failure_time: Option<Instant>,
}
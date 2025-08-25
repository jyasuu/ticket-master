use crate::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{info, warn, error};

/// Graceful shutdown coordinator
#[derive(Clone)]
pub struct ShutdownCoordinator {
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: Arc<Mutex<broadcast::Receiver<()>>>,
    components: Arc<Mutex<Vec<Box<dyn ShutdownComponent + Send + Sync>>>>,
    shutdown_timeout: Duration,
}

/// Trait for components that need graceful shutdown
#[async_trait::async_trait]
pub trait ShutdownComponent {
    async fn shutdown(&self) -> Result<()>;
    fn name(&self) -> &str;
}

impl ShutdownCoordinator {
    pub fn new(shutdown_timeout: Duration) -> Self {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        Self {
            shutdown_tx,
            shutdown_rx: Arc::new(Mutex::new(shutdown_rx)),
            components: Arc::new(Mutex::new(Vec::new())),
            shutdown_timeout,
        }
    }

    pub fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }

    /// Register a component for graceful shutdown
    pub async fn register_component(&self, component: Box<dyn ShutdownComponent + Send + Sync>) {
        let mut components = self.components.lock().await;
        info!("Registering component '{}' for graceful shutdown", component.name());
        components.push(component);
    }

    /// Get a shutdown signal receiver
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Trigger graceful shutdown
    pub async fn shutdown(&self) -> Result<()> {
        info!("Initiating graceful shutdown...");
        
        // Send shutdown signal to all subscribers
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("Failed to send shutdown signal: {}", e);
        }

        // Shutdown all registered components
        let components = self.components.lock().await;
        let mut shutdown_tasks = Vec::new();

        for component in components.iter() {
            let component_name = component.name().to_string();
            let component_shutdown = async move {
                info!("Shutting down component '{}'", component_name);
                match component.shutdown().await {
                    Ok(()) => {
                        info!("Component '{}' shutdown successfully", component_name);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Component '{}' shutdown failed: {}", component_name, e);
                        Err(e)
                    }
                }
            };
            shutdown_tasks.push(tokio::spawn(component_shutdown));
        }

        // Wait for all components to shutdown with timeout
        let shutdown_future = async {
            for task in shutdown_tasks {
                if let Err(e) = task.await {
                    error!("Shutdown task failed: {}", e);
                }
            }
        };

        match tokio::time::timeout(self.shutdown_timeout, shutdown_future).await {
            Ok(()) => {
                info!("All components shutdown successfully");
                Ok(())
            }
            Err(_) => {
                error!("Shutdown timeout exceeded, forcing exit");
                Err(crate::TicketMasterError::InvalidArgument(
                    "Shutdown timeout exceeded".to_string()
                ))
            }
        }
    }

    /// Wait for shutdown signal
    pub async fn wait_for_shutdown(&self) {
        let mut rx = self.shutdown_rx.lock().await;
        let _ = rx.recv().await;
    }
}

/// Kafka producer shutdown component
pub struct KafkaProducerShutdown {
    producer: crate::KafkaProducer,
}

impl KafkaProducerShutdown {
    pub fn new(producer: crate::KafkaProducer) -> Self {
        Self { producer }
    }
}

#[async_trait::async_trait]
impl ShutdownComponent for KafkaProducerShutdown {
    async fn shutdown(&self) -> Result<()> {
        info!("Flushing Kafka producer...");
        self.producer.flush(Duration::from_secs(10)).await?;
        info!("Kafka producer flushed successfully");
        Ok(())
    }

    fn name(&self) -> &str {
        "kafka-producer"
    }
}

/// RocksDB store shutdown component
pub struct RocksDBShutdown {
    store: Arc<crate::RocksDBStore>,
}

impl RocksDBShutdown {
    pub fn new(store: Arc<crate::RocksDBStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl ShutdownComponent for RocksDBShutdown {
    async fn shutdown(&self) -> Result<()> {
        info!("Flushing RocksDB store...");
        self.store.flush()?;
        info!("RocksDB store flushed successfully");
        Ok(())
    }

    fn name(&self) -> &str {
        "rocksdb-store"
    }
}

/// HTTP server shutdown component
pub struct HttpServerShutdown {
    name: String,
}

impl HttpServerShutdown {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ShutdownComponent for HttpServerShutdown {
    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down HTTP server '{}'", self.name);
        // In a real implementation, you'd gracefully shutdown the HTTP server
        // For now, we'll just simulate the shutdown
        sleep(Duration::from_millis(100)).await;
        info!("HTTP server '{}' shutdown successfully", self.name);
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Signal handler for graceful shutdown
pub async fn setup_signal_handlers(coordinator: ShutdownCoordinator) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            
            let mut sigterm = signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
            let mut sigint = signal(SignalKind::interrupt()).expect("Failed to setup SIGINT handler");
            
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, initiating graceful shutdown");
                }
                _ = sigint.recv() => {
                    info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
                }
            }
        }
        
        #[cfg(windows)]
        {
            use tokio::signal::windows::{ctrl_c, ctrl_break};
            
            let mut ctrl_c = ctrl_c().expect("Failed to setup Ctrl+C handler");
            let mut ctrl_break = ctrl_break().expect("Failed to setup Ctrl+Break handler");
            
            tokio::select! {
                _ = ctrl_c.recv() => {
                    info!("Received Ctrl+C, initiating graceful shutdown");
                }
                _ = ctrl_break.recv() => {
                    info!("Received Ctrl+Break, initiating graceful shutdown");
                }
            }
        }
        
        if let Err(e) = coordinator.shutdown().await {
            error!("Graceful shutdown failed: {}", e);
            std::process::exit(1);
        }
        
        info!("Graceful shutdown completed");
        std::process::exit(0);
    });
}

/// Utility for running services with graceful shutdown
pub async fn run_with_graceful_shutdown<F, Fut>(
    service_name: &str,
    service_future: F,
    shutdown_coordinator: ShutdownCoordinator,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    info!("Starting service '{}'", service_name);
    
    // Setup signal handlers
    setup_signal_handlers(shutdown_coordinator.clone()).await;
    
    // Run the service
    let service_result = tokio::select! {
        result = service_future() => {
            info!("Service '{}' completed", service_name);
            result
        }
        _ = shutdown_coordinator.wait_for_shutdown() => {
            info!("Service '{}' received shutdown signal", service_name);
            Ok(())
        }
    };
    
    // Ensure graceful shutdown
    if let Err(e) = shutdown_coordinator.shutdown().await {
        error!("Failed to shutdown service '{}': {}", service_name, e);
        return Err(e);
    }
    
    service_result
}
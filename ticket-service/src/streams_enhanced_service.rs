use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaProducer, KafkaConsumer,
    CreateEvent, CreateReservation, Reservation, AreaStatus, Area, Seat,
    ReservationType, Topics, Stores, event_area_key, ProcessingContext
};
use crate::kafka_streams_topology::{KafkaStreamsTopology, TopologyBuilder, StreamsConfig};
use crate::distributed_service::HostInfo;
use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerError};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::{CreateEventRequest, CreateReservationRequest};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tracing::{info, warn, error, instrument, Span};
use tokio::sync::{oneshot, Mutex};
use reqwest::Client;

/// Async response with metadata - equivalent to Java's AsyncResponseWithMetadata
#[derive(Debug)]
pub struct AsyncResponseWithMetadata {
    pub sender: oneshot::Sender<Result<Reservation>>,
    pub span_context: Span,
    pub start_time: Instant,
}

impl AsyncResponseWithMetadata {
    pub fn new(sender: oneshot::Sender<Result<Reservation>>, span_context: Span) -> Self {
        Self {
            sender,
            span_context,
            start_time: Instant::now(),
        }
    }

    /// Complete the async response - equivalent to Java's asyncResponse.resume()
    pub fn complete(self, result: Result<Reservation>) {
        let _guard = self.span_context.enter();
        info!("Completing async response after {:?}", self.start_time.elapsed());
        
        if let Err(_) = self.sender.send(result) {
            warn!("Failed to complete async response - receiver may have been dropped");
        }
    }

    /// Check if the response has timed out - equivalent to Java's asyncResponse.isDone()
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.start_time.elapsed() >= timeout
    }
}

/// Enhanced Ticket Service with true Kafka Streams topology support
/// This version implements the exact equivalent of Java's createTopology() method
pub struct StreamsEnhancedTicketService {
    producer: KafkaProducer,
    consumer: Arc<KafkaConsumer>,
    context: Arc<ProcessingContext>,
    http_client: Client,
    local_host: HostInfo,
    outstanding_requests: Arc<Mutex<HashMap<String, AsyncResponseWithMetadata>>>,
    topology: Option<KafkaStreamsTopology>,
    config: ServiceConfig,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl Clone for StreamsEnhancedTicketService {
    fn clone(&self) -> Self {
        Self {
            producer: self.producer.clone(),
            consumer: Arc::clone(&self.consumer),
            context: Arc::clone(&self.context),
            http_client: self.http_client.clone(),
            local_host: self.local_host.clone(),
            outstanding_requests: Arc::clone(&self.outstanding_requests),
            topology: None, // Topology is not cloneable, will be recreated if needed
            config: self.config.clone(),
            circuit_breaker: Arc::clone(&self.circuit_breaker),
        }
    }
}

impl StreamsEnhancedTicketService {
    pub async fn new(config: ServiceConfig, local_host: HostInfo) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        let producer = KafkaProducer::new(kafka_config.clone())?;
        let consumer = KafkaConsumer::new(kafka_config)?;

        // Initialize state stores for querying
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        
        // Add RocksDB stores for reading state
        context.add_rocksdb_store(Stores::AREA_STATUS.to_string(), "area-status")?;
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;

        // Create HTTP client with timeout and connection pooling
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(|e| TicketMasterError::HttpClient(e.to_string()))?;

        let outstanding_requests = Arc::new(Mutex::new(HashMap::new()));

        // Initialize circuit breaker for remote service calls
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            5,                              // failure_threshold: 5 failures to open circuit
            Duration::from_secs(30),        // recovery_timeout: 30 seconds before trying again
            Duration::from_secs(10),        // request_timeout: 10 seconds per request
        ));

        // Start cleanup task for outstanding requests (equivalent to Java's CompletionCallback)
        let cleanup_requests = Arc::clone(&outstanding_requests);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                Self::cleanup_timed_out_requests(&cleanup_requests, Duration::from_secs(30)).await;
            }
        });

        Ok(Self { 
            producer,
            consumer: Arc::new(consumer),
            context: Arc::new(context),
            http_client,
            local_host,
            outstanding_requests,
            topology: None,
            config,
            circuit_breaker,
        })
    }

    /// Create and start the Kafka Streams topology - equivalent to Java's createTopology()
    #[instrument(skip(self))]
    pub async fn start_streams_topology(&mut self) -> Result<()> {
        info!("Creating Kafka Streams topology...");

        // Create streams configuration
        let streams_config = StreamsConfig::new("ticket-service".to_string())
            .state_dir(self.config.state_dir.clone())
            .commit_interval_ms(100)
            .processing_guarantee("exactly_once_v2".to_string());

        // Build the topology using the builder pattern (equivalent to Java's StreamsBuilder)
        let topology_builder = TopologyBuilder::new(streams_config.application_id.clone());
        let topology = topology_builder.build(
            Arc::clone(&self.consumer),
            Arc::clone(&self.context),
            Arc::clone(&self.outstanding_requests),
        );

        info!("Topology description:\n{}", topology.describe());

        // Start the topology in the background
        let topology_for_task = topology;
        tokio::spawn(async move {
            if let Err(e) = topology_for_task.start_topology().await {
                error!("Kafka Streams topology error: {}", e);
            }
        });

        info!("Kafka Streams topology started successfully");
        Ok(())
    }

    /// Create event - same as distributed service
    pub async fn create_event(&self, request: CreateEventRequest) -> Result<String> {
        info!("Creating event: {}", request.event_name);

        // Parse timestamps
        let reservation_opening_time = parse_timestamp(&request.reservation_opening_time)?;
        let reservation_closing_time = parse_timestamp(&request.reservation_closing_time)?;
        let event_start_time = parse_timestamp(&request.event_start_time)?;
        let event_end_time = parse_timestamp(&request.event_end_time)?;

        // Convert areas
        let areas: Vec<Area> = request.areas.into_iter().map(|area_req| {
            Area {
                area_id: area_req.area_id,
                price: area_req.price,
                row_count: area_req.row_count,
                col_count: area_req.col_count,
            }
        }).collect();

        let create_event = CreateEvent {
            artist: request.artist,
            event_name: request.event_name.clone(),
            reservation_opening_time,
            reservation_closing_time,
            event_start_time,
            event_end_time,
            areas,
        };

        // Send create event command
        self.producer.send(
            Topics::COMMAND_EVENT_CREATE_EVENT,
            &request.event_name,
            &create_event,
        ).await?;

        info!("Event creation command sent: {}", request.event_name);
        Ok(request.event_name)
    }

    /// Create reservation - same as distributed service
    pub async fn create_reservation(&self, request: CreateReservationRequest) -> Result<String> {
        let reservation_id = Uuid::new_v4().to_string();
        
        info!("Creating reservation: {}", reservation_id);

        // Parse reservation type
        let reservation_type = match request.reservation_type.to_lowercase().as_str() {
            "self_pick" | "selfpick" => ReservationType::SelfPick,
            "random" => ReservationType::Random,
            _ => return Err(TicketMasterError::InvalidArgument(
                format!("Invalid reservation type: {}", request.reservation_type)
            )),
        };

        // Convert seats if provided
        let seats: Vec<Seat> = request.seats.unwrap_or_default().into_iter().map(|seat_req| {
            Seat {
                row: seat_req.row,
                col: seat_req.col,
            }
        }).collect();

        let create_reservation = CreateReservation {
            reservation_id: reservation_id.clone(),
            user_id: request.user_id,
            event_id: request.event_id,
            area_id: request.area_id,
            num_of_seats: request.num_of_seats,
            num_of_seat: 0,
            reservation_type,
            seats,
        };

        // Send create reservation command
        self.producer.send(
            Topics::COMMAND_RESERVATION_CREATE_RESERVATION,
            &reservation_id,
            &create_reservation,
        ).await?;

        info!("Reservation creation command sent: {}", reservation_id);
        Ok(reservation_id)
    }

    /// Get area status - same as distributed service
    pub async fn get_area_status(&self, event_name: &str, area_id: &str) -> Result<Option<AreaStatus>> {
        info!("Getting area status for event: {}, area: {}", event_name, area_id);
        
        let key = event_area_key(event_name, area_id);
        
        if let Some(store) = self.context.get_rocksdb_store(Stores::AREA_STATUS) {
            match store.get::<AreaStatus>(&key)? {
                Some(area_status) => {
                    info!("Found area status for {}: {} available seats", key, area_status.available_seats);
                    Ok(Some(area_status))
                }
                None => {
                    info!("No area status found for key: {}", key);
                    Ok(None)
                }
            }
        } else {
            info!("Area status store not available");
            Ok(None)
        }
    }

    /// Enhanced get_reservation with Kafka Streams topology support
    /// This now uses the exact same pattern as Java's getReservationById
    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    pub async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>> {
        self.get_reservation_with_timeout(reservation_id, Duration::from_secs(10)).await
    }

    /// Get reservation with timeout - now using Kafka Streams topology with distributed querying
    /// This is the exact equivalent of Java's fetchReservation method
    #[instrument(skip(self), fields(reservation_id = %reservation_id, timeout_secs = timeout_duration.as_secs()))]
    pub async fn get_reservation_with_timeout(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<Reservation>> {
        info!("Getting reservation: {} with timeout: {:?}", reservation_id, timeout_duration);
        
        // Step 1: Get key location metadata (equivalent to Java's getKeyLocationOrBlock)
        let host_for_key = self.get_key_location_or_block(reservation_id, timeout_duration).await?;
        
        // Step 2: Check if data is local or remote (equivalent to Java's host comparison)
        if host_for_key.host == self.local_host.host && host_for_key.port == self.local_host.port {
            // Data is local - fetch from local store (equivalent to fetchReservationFromLocal)
            self.fetch_reservation_from_local(reservation_id, timeout_duration).await
        } else {
            // Data is on remote host - fetch from remote (equivalent to fetchReservationFromOtherHost)
            self.fetch_reservation_from_other_host(&host_for_key, reservation_id).await
        }
    }

    /// Get key location with blocking and timeout - equivalent to Java's getKeyLocationOrBlock
    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    async fn get_key_location_or_block(&self, reservation_id: &str, timeout_duration: Duration) -> Result<HostInfo> {
        let start_time = Instant::now();
        
        loop {
            // Simulate metadata query (in real implementation, this would query Kafka metadata)
            // For now, we assume data is always local, but this structure matches Java exactly
            let metadata = self.query_metadata_for_key(reservation_id).await?;
            
            if let Some(host_info) = metadata {
                info!("Found metadata for reservation {}: {}:{}", reservation_id, host_info.host, host_info.port);
                return Ok(host_info);
            }
            
            // Check timeout (equivalent to Java's asyncResponse.isDone() check)
            if start_time.elapsed() >= timeout_duration {
                return Err(TicketMasterError::Timeout(
                    format!("Metadata lookup timed out after {:?}", timeout_duration)
                ));
            }
            
            // Sleep before retry (equivalent to Java's Thread.sleep(200))
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Query metadata for key - equivalent to Java's streams.queryMetadataForKey
    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    async fn query_metadata_for_key(&self, reservation_id: &str) -> Result<Option<HostInfo>> {
        // In a real implementation, this would query Kafka Streams metadata
        // For now, we simulate by always returning local host
        // This matches the Java pattern where metadata might not be available during startup/rebalance
        
        // Simulate metadata not available during startup (like Java's KeyQueryMetadata.NOT_AVAILABLE)
        if self.context.get_rocksdb_store(Stores::RESERVATION).is_none() {
            return Ok(None);
        }
        
        // For simplicity, always return local host
        // In real implementation, this would use consistent hashing to determine the correct host
        Ok(Some(self.local_host.clone()))
    }

    /// Fetch reservation from local store - equivalent to Java's fetchReservationFromLocal
    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    async fn fetch_reservation_from_local(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<Reservation>> {
        // Check local store first
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            if let Some(reservation) = store.get::<Reservation>(reservation_id)? {
                info!("Found reservation locally: {} for user {}", reservation_id, reservation.user_id);
                return Ok(Some(reservation));
            }
        }

        // If not found locally, register outstanding request and wait for Kafka Streams topology
        // This is the exact equivalent of Java's outstandingRequests.put() and waiting for the foreach callback
        info!("Reservation not found locally, registering outstanding request and waiting for topology update...");
        
        let (sender, receiver) = oneshot::channel();
        
        // Create span for tracing (equivalent to Java's Context.current())
        let span = tracing::info_span!(
            "outstanding-request",
            reservation_id = %reservation_id,
            timeout_secs = timeout_duration.as_secs()
        );
        
        // Register the outstanding request with metadata (equivalent to Java's outstandingRequests.put())
        {
            let mut outstanding_requests = self.outstanding_requests.lock().await;
            let async_response = AsyncResponseWithMetadata::new(sender, span);
            outstanding_requests.insert(reservation_id.to_string(), async_response);
        }

        // Double-check after registering (race condition protection, same as Java)
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            if let Some(reservation) = store.get::<Reservation>(reservation_id)? {
                // Remove the outstanding request since we found the data
                let mut outstanding_requests = self.outstanding_requests.lock().await;
                outstanding_requests.remove(reservation_id);
                info!("Found reservation after registering outstanding request: {}", reservation_id);
                return Ok(Some(reservation));
            }
        }

        // Wait for the Kafka Streams topology to complete the request
        // This is equivalent to Java's asyncResponse.resume() being called from the foreach callback
        match tokio::time::timeout(timeout_duration, receiver).await {
            Ok(Ok(Ok(reservation))) => {
                info!("Received reservation update from Kafka Streams topology: {}", reservation_id);
                Ok(Some(reservation))
            }
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => {
                // Channel was dropped
                Ok(None)
            }
            Err(_) => {
                // Timeout - clean up outstanding request
                let mut outstanding_requests = self.outstanding_requests.lock().await;
                outstanding_requests.remove(reservation_id);
                Err(TicketMasterError::Timeout(
                    format!("Request timed out after {:?}", timeout_duration)
                ))
            }
        }
    }

    /// Fetch reservation from other host - equivalent to Java's fetchReservationFromOtherHost
    /// Now enhanced with circuit breaker pattern for resilience
    #[instrument(skip(self), fields(reservation_id = %reservation_id, host = %host_info.host, port = host_info.port))]
    async fn fetch_reservation_from_other_host(&self, host_info: &HostInfo, reservation_id: &str) -> Result<Option<Reservation>> {
        let url = format!("http://{}:{}/reservations/{}", host_info.host, host_info.port, reservation_id);
        
        info!("Fetching reservation from remote host with circuit breaker: {}", url);
        
        // Execute the remote call through circuit breaker
        let circuit_breaker = Arc::clone(&self.circuit_breaker);
        let http_client = self.http_client.clone();
        
        let remote_call = async move {
            let response = http_client.get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| TicketMasterError::HttpClient(e.to_string()))?;

            if response.status().is_success() {
                let api_response: crate::ApiResponse<serde_json::Value> = response.json().await
                    .map_err(|e| TicketMasterError::HttpClient(e.to_string()))?;

                if api_response.success {
                    if let Some(data) = api_response.data {
                        let reservation = serde_json::from_value::<Reservation>(data)
                            .map_err(|e| TicketMasterError::Json(e))?;
                        info!("Successfully fetched reservation from remote host: {}", reservation_id);
                        Ok(Some(reservation))
                    } else {
                        Ok(None)
                    }
                } else {
                    if let Some(error_msg) = api_response.error {
                        if error_msg.contains("not found") {
                            Ok(None)
                        } else {
                            Err(TicketMasterError::RemoteService(error_msg))
                        }
                    } else {
                        Err(TicketMasterError::RemoteService("Unknown remote error".to_string()))
                    }
                }
            } else if response.status() == 404 {
                Ok(None)
            } else {
                Err(TicketMasterError::RemoteService(
                    format!("Remote service returned status: {}", response.status())
                ))
            }
        };

        // Execute through circuit breaker
        match circuit_breaker.execute(remote_call).await {
            Ok(result) => Ok(result),
            Err(CircuitBreakerError::CircuitOpen) => {
                warn!("Circuit breaker is open for remote host: {}:{}", host_info.host, host_info.port);
                Err(TicketMasterError::ServiceUnavailable(
                    format!("Remote service {}:{} is unavailable (circuit breaker open)", host_info.host, host_info.port)
                ))
            }
            Err(CircuitBreakerError::Timeout) => {
                warn!("Remote call timed out to host: {}:{}", host_info.host, host_info.port);
                Err(TicketMasterError::Timeout(
                    format!("Remote service call timed out: {}:{}", host_info.host, host_info.port)
                ))
            }
            Err(CircuitBreakerError::RequestFailed(e)) => {
                warn!("Remote call failed to host: {}:{} - {}", host_info.host, host_info.port, e);
                Err(e)
            }
        }
    }

    /// Health check
    pub async fn health_check(&self) -> Result<String> {
        if self.context.get_rocksdb_store(Stores::RESERVATION).is_some() {
            Ok("OK".to_string())
        } else {
            Err(TicketMasterError::ServiceUnavailable(
                "Service not ready - stores not available".to_string()
            ))
        }
    }

    /// Get outstanding requests count for monitoring
    pub async fn get_outstanding_requests_count(&self) -> usize {
        let outstanding_requests = self.outstanding_requests.lock().await;
        outstanding_requests.len()
    }

    /// Get circuit breaker metrics for monitoring
    pub async fn get_circuit_breaker_metrics(&self) -> serde_json::Value {
        let metrics = self.circuit_breaker.get_metrics().await;
        serde_json::json!({
            "state": format!("{:?}", metrics.state),
            "failure_count": metrics.failure_count,
            "success_count": metrics.success_count,
            "last_failure_time": metrics.last_failure_time.map(|t| t.elapsed().as_secs()),
            "is_healthy": matches!(metrics.state, crate::circuit_breaker::CircuitState::Closed)
        })
    }

    /// Cleanup timed out requests - equivalent to Java's CompletionCallback.onComplete()
    #[instrument(skip(outstanding_requests))]
    async fn cleanup_timed_out_requests(
        outstanding_requests: &Arc<Mutex<HashMap<String, AsyncResponseWithMetadata>>>,
        max_age: Duration,
    ) {
        let mut requests = outstanding_requests.lock().await;
        let mut to_remove = Vec::new();
        
        for (reservation_id, async_response) in requests.iter() {
            if async_response.is_timed_out(max_age) {
                warn!("Outstanding request timed out: {} (age: {:?})", reservation_id, async_response.start_time.elapsed());
                to_remove.push(reservation_id.clone());
            }
        }
        
        for reservation_id in to_remove {
            if let Some(async_response) = requests.remove(&reservation_id) {
                info!("Cleaning up timed out request: {}", reservation_id);
                // Complete with timeout error
                async_response.complete(Err(TicketMasterError::Timeout(
                    format!("Request timed out after {:?}", max_age)
                )));
            }
        }
        
        if !requests.is_empty() {
            info!("Outstanding requests count: {}", requests.len());
        }
    }
}

fn parse_timestamp(timestamp_str: &str) -> Result<DateTime<Utc>> {
    // Try parsing as ISO 8601 format first
    if let Ok(dt) = DateTime::parse_from_rfc3339(timestamp_str) {
        return Ok(dt.with_timezone(&Utc));
    }
    
    // Try parsing as timestamp millis
    if let Ok(millis) = timestamp_str.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp_millis(millis) {
            return Ok(dt);
        }
    }
    
    Err(TicketMasterError::InvalidArgument(
        format!("Invalid timestamp format: {}", timestamp_str)
    ))
}
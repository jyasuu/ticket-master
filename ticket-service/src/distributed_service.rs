use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaProducer, KafkaConsumer,
    CreateEvent, CreateReservation, Reservation, AreaStatus, Area, Seat,
    ReservationType, Topics, Stores, event_area_key, ProcessingContext
};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::{CreateEventRequest, CreateReservationRequest};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tracing::{info, warn, error, instrument};
use tokio::sync::{RwLock, oneshot, Mutex};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

// Host information for distributed queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    pub host: String,
    pub port: u16,
}

impl HostInfo {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

// Metadata for key location queries
#[derive(Debug, Clone)]
pub struct KeyQueryMetadata {
    pub active_host: Option<HostInfo>,
    pub standby_hosts: Vec<HostInfo>,
    pub partition: i32,
}

impl KeyQueryMetadata {
    pub const NOT_AVAILABLE: Self = Self {
        active_host: None,
        standby_hosts: Vec::new(),
        partition: -1,
    };

    pub fn is_available(&self) -> bool {
        self.active_host.is_some()
    }
}

// Outstanding request tracking
#[derive(Debug)]
struct OutstandingRequest {
    sender: oneshot::Sender<Result<Reservation>>,
    created_at: Instant,
    timeout_duration: Duration,
}

// Service state for tracking cluster topology
#[derive(Debug)]
struct ServiceState {
    // Current service host info
    local_host: HostInfo,
    // Known cluster hosts (simplified - in real implementation would come from Kafka metadata)
    cluster_hosts: Vec<HostInfo>,
    // Outstanding reservation requests waiting for updates
    outstanding_requests: HashMap<String, OutstandingRequest>,
}

impl ServiceState {
    fn new(local_host: HostInfo) -> Self {
        Self {
            local_host,
            cluster_hosts: Vec::new(),
            outstanding_requests: HashMap::new(),
        }
    }

    fn add_outstanding_request(&mut self, reservation_id: String, request: OutstandingRequest) {
        self.outstanding_requests.insert(reservation_id, request);
    }

    fn complete_outstanding_request(&mut self, reservation_id: &str, result: Result<Reservation>) {
        if let Some(request) = self.outstanding_requests.remove(reservation_id) {
            let _ = request.sender.send(result);
        }
    }

    fn cleanup_expired_requests(&mut self) {
        let now = Instant::now();
        let expired_keys: Vec<String> = self.outstanding_requests
            .iter()
            .filter(|(_, request)| now.duration_since(request.created_at) > request.timeout_duration)
            .map(|(key, _)| key.clone())
            .collect();

        for key in expired_keys {
            if let Some(request) = self.outstanding_requests.remove(&key) {
                let _ = request.sender.send(Err(TicketMasterError::Timeout(
                    "Request timed out waiting for reservation update".to_string()
                )));
            }
        }
    }
}

pub struct DistributedTicketService {
    producer: KafkaProducer,
    consumer: Arc<KafkaConsumer>,
    context: Arc<ProcessingContext>,
    http_client: Client,
    state: Arc<Mutex<ServiceState>>,
    config: ServiceConfig,
}

impl Clone for DistributedTicketService {
    fn clone(&self) -> Self {
        Self {
            producer: self.producer.clone(),
            consumer: Arc::clone(&self.consumer),
            context: Arc::clone(&self.context),
            http_client: self.http_client.clone(),
            state: Arc::clone(&self.state),
            config: self.config.clone(),
        }
    }
}

impl DistributedTicketService {
    pub async fn new(config: ServiceConfig, local_host: HostInfo) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        let producer = KafkaProducer::new(kafka_config.clone())?;
        let consumer = KafkaConsumer::new(kafka_config)?;

        // Subscribe to state topics to populate local stores
        consumer.subscribe(&[
            Topics::STATE_USER_RESERVATION,
        ])?;

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

        let state = Arc::new(Mutex::new(ServiceState::new(local_host)));

        Ok(Self { 
            producer,
            consumer: Arc::new(consumer),
            context: Arc::new(context),
            http_client,
            state,
            config,
        })
    }

    pub async fn run_consumer(&self) -> Result<()> {
        info!("Starting Distributed Ticket Service consumer for state synchronization...");
        
        // Start cleanup task for expired requests
        let state_for_cleanup = Arc::clone(&self.state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                if let Ok(mut state) = state_for_cleanup.try_lock() {
                    state.cleanup_expired_requests();
                }
            }
        });
        
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    info!("Received shutdown signal for consumer");
                    break;
                }
                
                // Process state messages
                message_result = self.consumer.recv_message(Duration::from_millis(100)) => {
                    match message_result? {
                        Some(message) => {
                            if let Err(e) = self.process_state_message(&message).await {
                                error!("Error processing state message: {}", e);
                            } else {
                                // Commit the message after successful processing
                                if let Err(e) = self.consumer.commit_message(&message) {
                                    error!("Error committing message: {}", e);
                                }
                            }
                        }
                        None => {
                            // No message received (timeout)
                            continue;
                        }
                    }
                }
            }
        }

        info!("Distributed Ticket Service consumer shutting down...");
        Ok(())
    }

    async fn process_state_message(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        match message.topic.as_str() {
            Topics::STATE_USER_RESERVATION => {
                self.handle_reservation_state_update(message).await
            }
            _ => {
                warn!("Unknown state topic: {}", message.topic);
                Ok(())
            }
        }
    }

    async fn handle_reservation_state_update(&self, message: &ticket_master::KafkaMessage) -> Result<()> {
        let reservation_id = message.key.as_ref()
            .ok_or_else(|| TicketMasterError::InvalidArgument("Missing reservation ID key".to_string()))?;
        
        let reservation: Reservation = message.deserialize_value()?;
        
        info!("Updating local reservation store: {} -> {:?}", reservation_id, reservation.state);

        // Update local store
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            store.put(reservation_id, &reservation)?;
            info!("Successfully updated reservation {} in local store", reservation_id);
        } else {
            warn!("Reservation store not available for update");
        }

        // Check for outstanding requests and complete them
        if let Ok(mut state) = self.state.try_lock() {
            state.complete_outstanding_request(reservation_id, Ok(reservation));
        }

        Ok(())
    }

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

    /// Enhanced get_reservation with distributed querying, timeouts, and real-time updates
    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    pub async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>> {
        self.get_reservation_with_timeout(reservation_id, Duration::from_secs(10)).await
    }

    /// Get reservation with custom timeout
    #[instrument(skip(self), fields(reservation_id = %reservation_id, timeout_secs = timeout_duration.as_secs()))]
    pub async fn get_reservation_with_timeout(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<Reservation>> {
        info!("Getting reservation: {} with timeout: {:?}", reservation_id, timeout_duration);
        
        // Apply timeout to the entire operation
        match timeout(timeout_duration, self.fetch_reservation(reservation_id, timeout_duration)).await {
            Ok(result) => result,
            Err(_) => Err(TicketMasterError::Timeout(
                format!("Request timed out after {:?}", timeout_duration)
            )),
        }
    }

    async fn fetch_reservation(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<Reservation>> {
        // Get key location with retry logic
        let host_for_key = self.get_key_location_or_wait(reservation_id, timeout_duration).await?;

        match host_for_key {
            Some(host) => {
                let local_host = {
                    let state = self.state.lock().await;
                    state.local_host.clone()
                };

                // Check if data is local or remote
                if host == local_host {
                    self.fetch_reservation_from_local(reservation_id, timeout_duration).await
                } else {
                    self.fetch_reservation_from_remote_host(&host, reservation_id).await
                }
            }
            None => {
                // No host available - this shouldn't happen if get_key_location_or_wait works correctly
                Err(TicketMasterError::ServiceUnavailable(
                    "No host available for reservation query".to_string()
                ))
            }
        }
    }

    #[instrument(skip(self), fields(reservation_id = %reservation_id))]
    async fn fetch_reservation_from_local(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<Reservation>> {
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            match store.get::<Reservation>(reservation_id)? {
                Some(reservation) => {
                    info!("Found reservation: {} for user {}", reservation_id, reservation.user_id);
                    Ok(Some(reservation))
                }
                None => {
                    info!("No reservation found locally for id: {}, waiting for updates...", reservation_id);
                    
                    // Set up outstanding request to wait for real-time updates
                    let (sender, receiver) = oneshot::channel();
                    let outstanding_request = OutstandingRequest {
                        sender,
                        created_at: Instant::now(),
                        timeout_duration,
                    };

                    {
                        let mut state = self.state.lock().await;
                        state.add_outstanding_request(reservation_id.to_string(), outstanding_request);
                    }

                    // Double-check after registering the outstanding request (race condition protection)
                    if let Some(reservation) = store.get::<Reservation>(reservation_id)? {
                        // Remove the outstanding request since we found the data
                        let mut state = self.state.lock().await;
                        state.outstanding_requests.remove(reservation_id);
                        info!("Found reservation after registering outstanding request: {}", reservation_id);
                        return Ok(Some(reservation));
                    }

                    // Wait for the reservation to arrive via Kafka stream
                    match receiver.await {
                        Ok(Ok(reservation)) => {
                            info!("Received reservation update for: {}", reservation_id);
                            Ok(Some(reservation))
                        }
                        Ok(Err(e)) => Err(e),
                        Err(_) => {
                            // Channel was dropped, likely due to timeout cleanup
                            Ok(None)
                        }
                    }
                }
            }
        } else {
            Err(TicketMasterError::ServiceUnavailable(
                "Reservation store not available".to_string()
            ))
        }
    }

    #[instrument(skip(self), fields(host = %host.url(), reservation_id = %reservation_id))]
    async fn fetch_reservation_from_remote_host(&self, host: &HostInfo, reservation_id: &str) -> Result<Option<Reservation>> {
        let url = format!("{}/reservations/{}", host.url(), reservation_id);
        
        info!("Fetching reservation from remote host: {}", url);

        match self.http_client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<crate::ApiResponse<serde_json::Value>>().await {
                        Ok(api_response) => {
                            if api_response.success {
                                if let Some(data) = api_response.data {
                                    match serde_json::from_value::<Reservation>(data) {
                                        Ok(reservation) => {
                                            info!("Successfully fetched reservation from remote host: {}", reservation_id);
                                            Ok(Some(reservation))
                                        }
                                        Err(e) => Err(TicketMasterError::Json(e)),
                                    }
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
                        }
                        Err(e) => Err(TicketMasterError::HttpClient(e.to_string())),
                    }
                } else if response.status() == 404 {
                    Ok(None)
                } else if response.status() == 503 {
                    Err(TicketMasterError::ServiceUnavailable(
                        "Remote service unavailable".to_string()
                    ))
                } else {
                    Err(TicketMasterError::RemoteService(
                        format!("Remote service error: {}", response.status())
                    ))
                }
            }
            Err(e) => {
                error!("Failed to fetch from remote host {}: {}", url, e);
                Err(TicketMasterError::HttpClient(e.to_string()))
            }
        }
    }

    async fn get_key_location_or_wait(&self, reservation_id: &str, timeout_duration: Duration) -> Result<Option<HostInfo>> {
        let start_time = Instant::now();
        
        loop {
            let metadata = self.query_metadata_for_key(reservation_id).await;
            
            if metadata.is_available() {
                return Ok(metadata.active_host);
            }

            // Check if we've exceeded the timeout
            if start_time.elapsed() >= timeout_duration {
                return Err(TicketMasterError::Timeout(
                    "Timed out waiting for metadata to become available".to_string()
                ));
            }

            // Sleep briefly before retrying (similar to Java implementation)
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn query_metadata_for_key(&self, reservation_id: &str) -> KeyQueryMetadata {
        // Simplified implementation - in a real system, this would query Kafka metadata
        // For now, we'll simulate the metadata lookup
        
        // In a real implementation, you would:
        // 1. Hash the key to determine the partition
        // 2. Query Kafka for the current partition assignment
        // 3. Return the host information for the partition leader
        
        // For this demo, we'll assume the local host handles all keys
        let local_host = {
            let state = self.state.lock().await;
            state.local_host.clone()
        };

        KeyQueryMetadata {
            active_host: Some(local_host),
            standby_hosts: Vec::new(),
            partition: 0, // Simplified
        }
    }

    /// Health check that considers the state of Kafka Streams
    pub async fn health_check(&self) -> Result<String> {
        // In a real implementation, you'd check:
        // - Kafka Streams state
        // - Store availability
        // - Consumer lag
        // - Producer health
        
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
        let state = self.state.lock().await;
        state.outstanding_requests.len()
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
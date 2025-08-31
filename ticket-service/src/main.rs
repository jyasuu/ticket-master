use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use ticket_master::{Result, ServiceConfig};
use tower_http::cors::CorsLayer;
use tracing::{info, error};

mod service;
mod distributed_service;

use service::TicketService;
use distributed_service::{DistributedTicketService, HostInfo};

#[derive(Parser, Debug)]
#[command(name = "ticket-service")]
#[command(about = "Ticket Service REST API for Ticket Master")]
struct Args {
    /// Port to listen on
    #[arg(short = 'p', long = "port", default_value = "8080")]
    port: u16,

    /// Config file path
    #[arg(short = 'c', long = "config", default_value = "../client.dev.properties")]
    config: PathBuf,

    /// Producer config file path
    #[arg(long = "producer-config")]
    producer_config: Option<PathBuf>,

    /// Show help information
    #[arg(short = 'h', long = "help")]
    help: bool,

    /// Enable distributed mode with enhanced features
    #[arg(long = "distributed")]
    distributed: bool,

    /// Hostname for this service instance
    #[arg(long = "hostname", default_value = "localhost")]
    hostname: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateEventRequest {
    artist: String,
    event_name: String,
    reservation_opening_time: String,
    reservation_closing_time: String,
    event_start_time: String,
    event_end_time: String,
    areas: Vec<AreaRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AreaRequest {
    area_id: String,
    price: i32,
    row_count: i32,
    col_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateReservationRequest {
    user_id: String,
    event_id: String,
    area_id: String,
    num_of_seats: i32,
    reservation_type: String,
    seats: Option<Vec<SeatRequest>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeatRequest {
    row: i32,
    col: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if args.help {
        println!("Ticket Service REST API for Ticket Master");
        return Ok(());
    }

    info!("Starting Ticket Service on port {}", args.port);
    info!("Config file: {:?}", args.config);

    // Load configuration
    let mut config = load_config(&args.config)?;

    if let Some(ref producer_config_path) = args.producer_config {
        info!("Loading producer config from: {:?}", producer_config_path);
        config = ticket_master::merge_stream_properties(config, producer_config_path.clone())?;
    }

    if args.distributed {
        info!("Starting in distributed mode with enhanced features");
        run_distributed_service(config, args).await
    } else {
        info!("Starting in standard mode");
        run_standard_service(config, args).await
    }
}

async fn create_event(
    State(service): State<Arc<TicketService>>,
    Json(request): Json<CreateEventRequest>,
) -> std::result::Result<Json<ApiResponse<String>>, StatusCode> {
    match service.create_event(request).await {
        Ok(event_name) => Ok(Json(ApiResponse::success(event_name))),
        Err(e) => {
            error!("Error creating event: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn get_area_status(
    State(service): State<Arc<TicketService>>,
    Path((event_name, area_id)): Path<(String, String)>,
) -> std::result::Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    match service.get_area_status(&event_name, &area_id).await {
        Ok(Some(area_status)) => Ok(Json(ApiResponse::success(serde_json::to_value(area_status).unwrap()))),
        Ok(None) => Ok(Json(ApiResponse::error("Area not found".to_string()))),
        Err(e) => {
            error!("Error getting area status: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn create_reservation(
    State(service): State<Arc<TicketService>>,
    Json(request): Json<CreateReservationRequest>,
) -> std::result::Result<Json<ApiResponse<String>>, StatusCode> {
    match service.create_reservation(request).await {
        Ok(reservation_id) => Ok(Json(ApiResponse::success(reservation_id))),
        Err(e) => {
            error!("Error creating reservation: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn get_reservation(
    State(service): State<Arc<TicketService>>,
    Path(reservation_id): Path<String>,
) -> std::result::Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    match service.get_reservation(&reservation_id).await {
        Ok(Some(reservation)) => Ok(Json(ApiResponse::success(serde_json::to_value(reservation).unwrap()))),
        Ok(None) => Ok(Json(ApiResponse::error("Reservation not found".to_string()))),
        Err(e) => {
            error!("Error getting reservation: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn health_check() -> Json<ApiResponse<String>> {
    Json(ApiResponse::success("OK".to_string()))
}

async fn run_standard_service(config: ServiceConfig, args: Args) -> Result<()> {
    // Create the ticket service
    let ticket_service = Arc::new(TicketService::new(config).await?);

    // Start the consumer in the background to sync state
    let service_for_consumer = Arc::clone(&ticket_service);
    tokio::spawn(async move {
        if let Err(e) = service_for_consumer.run_consumer().await {
            tracing::error!("Consumer error: {}", e);
        }
    });

    // Build the router
    let app = Router::new()
        .route("/events", post(create_event))
        .route("/events/:event_name/areas/:area_id", get(get_area_status))
        .route("/reservations", post(create_reservation))
        .route("/reservations/:reservation_id", get(get_reservation))
        .route("/health", get(health_check))
        .layer(CorsLayer::permissive())
        .with_state(ticket_service);

    // Start the server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    info!("Ticket Service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_distributed_service(config: ServiceConfig, args: Args) -> Result<()> {
    let host_info = HostInfo::new(args.hostname, args.port);
    
    // Create the distributed ticket service
    let distributed_service = Arc::new(DistributedTicketService::new(config, host_info).await?);

    // Start the consumer in the background to sync state
    let service_for_consumer = Arc::clone(&distributed_service);
    tokio::spawn(async move {
        if let Err(e) = service_for_consumer.run_consumer().await {
            tracing::error!("Distributed consumer error: {}", e);
        }
    });

    // Build the router with enhanced endpoints
    let app = Router::new()
        .route("/events", post(create_event_distributed))
        .route("/events/:event_name/areas/:area_id", get(get_area_status_distributed))
        .route("/reservations", post(create_reservation_distributed))
        .route("/reservations/:reservation_id", get(get_reservation_distributed))
        .route("/reservations/:reservation_id/timeout/:timeout_secs", get(get_reservation_with_timeout))
        .route("/health", get(health_check_distributed))
        .route("/metrics/outstanding-requests", get(get_outstanding_requests_count))
        .layer(CorsLayer::permissive())
        .with_state(distributed_service);

    // Start the server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    info!("Distributed Ticket Service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// Enhanced distributed endpoints
async fn create_event_distributed(
    State(service): State<Arc<DistributedTicketService>>,
    Json(request): Json<CreateEventRequest>,
) -> std::result::Result<Json<ApiResponse<String>>, StatusCode> {
    match service.create_event(request).await {
        Ok(event_name) => Ok(Json(ApiResponse::success(event_name))),
        Err(e) => {
            error!("Error creating event: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn get_area_status_distributed(
    State(service): State<Arc<DistributedTicketService>>,
    Path((event_name, area_id)): Path<(String, String)>,
) -> std::result::Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    match service.get_area_status(&event_name, &area_id).await {
        Ok(Some(area_status)) => Ok(Json(ApiResponse::success(serde_json::to_value(area_status).unwrap()))),
        Ok(None) => Ok(Json(ApiResponse::error("Area not found".to_string()))),
        Err(e) => {
            error!("Error getting area status: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn create_reservation_distributed(
    State(service): State<Arc<DistributedTicketService>>,
    Json(request): Json<CreateReservationRequest>,
) -> std::result::Result<Json<ApiResponse<String>>, StatusCode> {
    match service.create_reservation(request).await {
        Ok(reservation_id) => Ok(Json(ApiResponse::success(reservation_id))),
        Err(e) => {
            error!("Error creating reservation: {}", e);
            Ok(Json(ApiResponse::error(e.to_string())))
        }
    }
}

async fn get_reservation_distributed(
    State(service): State<Arc<DistributedTicketService>>,
    Path(reservation_id): Path<String>,
) -> std::result::Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    match service.get_reservation(&reservation_id).await {
        Ok(Some(reservation)) => Ok(Json(ApiResponse::success(serde_json::to_value(reservation).unwrap()))),
        Ok(None) => Ok(Json(ApiResponse::error("Reservation not found".to_string()))),
        Err(e) => {
            error!("Error getting reservation: {}", e);
            match e {
                ticket_master::TicketMasterError::ServiceUnavailable(_) => {
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                }
                ticket_master::TicketMasterError::Timeout(_) => {
                    Ok(Json(ApiResponse::error("Request timed out".to_string())))
                }
                _ => Ok(Json(ApiResponse::error(e.to_string())))
            }
        }
    }
}

async fn get_reservation_with_timeout(
    State(service): State<Arc<DistributedTicketService>>,
    Path((reservation_id, timeout_secs)): Path<(String, u64)>,
) -> std::result::Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    let timeout_duration = Duration::from_secs(timeout_secs);
    
    match service.get_reservation_with_timeout(&reservation_id, timeout_duration).await {
        Ok(Some(reservation)) => Ok(Json(ApiResponse::success(serde_json::to_value(reservation).unwrap()))),
        Ok(None) => Ok(Json(ApiResponse::error("Reservation not found".to_string()))),
        Err(e) => {
            error!("Error getting reservation with timeout: {}", e);
            match e {
                ticket_master::TicketMasterError::ServiceUnavailable(_) => {
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                }
                ticket_master::TicketMasterError::Timeout(_) => {
                    Ok(Json(ApiResponse::error("Request timed out".to_string())))
                }
                _ => Ok(Json(ApiResponse::error(e.to_string())))
            }
        }
    }
}

async fn health_check_distributed(
    State(service): State<Arc<DistributedTicketService>>,
) -> std::result::Result<Json<ApiResponse<String>>, StatusCode> {
    match service.health_check().await {
        Ok(status) => Ok(Json(ApiResponse::success(status))),
        Err(e) => {
            error!("Health check failed: {}", e);
            match e {
                ticket_master::TicketMasterError::ServiceUnavailable(_) => {
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                }
                _ => Ok(Json(ApiResponse::error(e.to_string())))
            }
        }
    }
}

async fn get_outstanding_requests_count(
    State(service): State<Arc<DistributedTicketService>>,
) -> Json<ApiResponse<usize>> {
    let count = service.get_outstanding_requests_count().await;
    Json(ApiResponse::success(count))
}

fn load_config(config_path: &PathBuf) -> Result<ServiceConfig> {
    use ticket_master::parse_properties_file;
    
    // Parse the Java properties file
    let config = parse_properties_file(config_path, "ticket-service")?;
    
    Ok(config)
}
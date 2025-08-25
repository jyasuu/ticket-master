use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};
use ticket_master::{Result, ServiceConfig};
use tower_http::cors::CorsLayer;
use tracing::{info, error};

mod service;

use service::TicketService;

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
    let config = load_config(&args.config)?;

    if let Some(producer_config_path) = args.producer_config {
        info!("Loading producer config from: {:?}", producer_config_path);
        config = ticket_master::merge_stream_properties(config, producer_config_path)?;
    }

    // Create the ticket service
    let ticket_service = TicketService::new(config).await?;

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

async fn create_event(
    State(service): State<TicketService>,
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
    State(service): State<TicketService>,
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
    State(service): State<TicketService>,
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
    State(service): State<TicketService>,
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

fn load_config(config_path: &PathBuf) -> Result<ServiceConfig> {
    use ticket_master::parse_properties_file;
    
    // Parse the Java properties file
    let config = parse_properties_file(config_path, "ticket-service")?;
    
    Ok(config)
}
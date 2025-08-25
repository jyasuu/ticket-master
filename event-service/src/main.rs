use clap::Parser;
use std::path::PathBuf;
use ticket_master::{Result, ServiceConfig};
use tracing::{info, error};

mod service;
mod strategies;

use service::EventService;

#[derive(Parser, Debug)]
#[command(name = "event-service")]
#[command(about = "Event Service for Ticket Master")]
struct Args {
    /// State directory for storage
    #[arg(short = 'd', long = "state-dir", default_value = "/tmp/kafka-streams")]
    state_dir: PathBuf,

    /// Config file path
    #[arg(short = 'c', long = "config", default_value = "../client.dev.properties")]
    config: PathBuf,

    /// Stream config file path
    #[arg(long = "stream-config")]
    stream_config: Option<PathBuf>,

    /// Show help information
    #[arg(short = 'h', long = "help")]
    help: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if args.help {
        println!("Event Service for Ticket Master");
        return Ok(());
    }

    info!("Starting Event Service");
    info!("State directory: {:?}", args.state_dir);
    info!("Config file: {:?}", args.config);

    // Load configuration
    let mut config = load_config(&args.config)?;
    config.application_id = "event-service".to_string();
    config.state_dir = args.state_dir.to_string_lossy().to_string();

    if let Some(stream_config_path) = args.stream_config {
        // Load additional stream configuration
        info!("Loading stream config from: {:?}", stream_config_path);
        config = ticket_master::merge_stream_properties(config, stream_config_path)?;
    }

    // Create and start the event service
    let service = EventService::new(config).await?;
    
    info!("Event Service started successfully");
    
    // Run the service
    service.run().await?;

    Ok(())
}

fn load_config(config_path: &PathBuf) -> Result<ServiceConfig> {
    use ticket_master::parse_properties_file;
    
    // Parse the Java properties file
    let mut config = parse_properties_file(config_path, "event-service")?;
    
    // Set service-specific defaults
    if config.commit_interval_ms.is_none() {
        config.commit_interval_ms = Some(20);
    }
    if config.processing_guarantee.is_none() {
        config.processing_guarantee = Some("exactly_once_v2".to_string());
    }
    
    Ok(config)
}
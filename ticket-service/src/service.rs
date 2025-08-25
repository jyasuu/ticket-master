use ticket_master::{
    Result, TicketMasterError, ServiceConfig, KafkaProducer, KafkaConsumer,
    CreateEvent, CreateReservation, Reservation, AreaStatus, Area, Seat,
    ReservationType, Topics, Stores, event_area_key, ProcessingContext, RocksDBStore
};
use crate::{CreateEventRequest, CreateReservationRequest};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tracing::info;

#[derive(Clone)]
pub struct TicketService {
    producer: KafkaProducer,
    consumer: KafkaConsumer,
    context: ProcessingContext,
}

impl TicketService {
    pub async fn new(config: ServiceConfig) -> Result<Self> {
        let kafka_config = config.to_kafka_config();
        let producer = KafkaProducer::new(kafka_config.clone())?;
        let consumer = KafkaConsumer::new(kafka_config)?;

        // Initialize state stores for querying
        let context = ProcessingContext::with_state_dir(config.state_dir.clone());
        
        // Add RocksDB stores for reading state
        context.add_rocksdb_store(Stores::AREA_STATUS.to_string(), "area-status")?;
        context.add_rocksdb_store(Stores::RESERVATION.to_string(), "reservations")?;

        Ok(Self { 
            producer,
            consumer,
            context,
        })
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
            num_of_seat: 0, // This seems to be used for numbering, defaulting to 0
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

    pub async fn get_reservation(&self, reservation_id: &str) -> Result<Option<Reservation>> {
        info!("Getting reservation: {}", reservation_id);
        
        if let Some(store) = self.context.get_rocksdb_store(Stores::RESERVATION) {
            match store.get::<Reservation>(reservation_id)? {
                Some(reservation) => {
                    info!("Found reservation: {} for user {}", reservation_id, reservation.user_id);
                    Ok(Some(reservation))
                }
                None => {
                    info!("No reservation found for id: {}", reservation_id);
                    Ok(None)
                }
            }
        } else {
            info!("Reservation store not available");
            Ok(None)
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
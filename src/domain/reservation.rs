use serde::{Deserialize, Serialize};
use super::event::{Seat, ReservationType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateReservation {
    pub reservation_id: String,
    pub user_id: String,
    pub event_id: String,
    pub area_id: String,
    pub num_of_seats: i32,
    pub num_of_seat: i32,
    pub reservation_type: ReservationType,
    pub seats: Vec<Seat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reservation {
    pub reservation_id: String,
    pub user_id: String,
    pub event_id: String,
    pub area_id: String,
    pub num_of_seats: i32,
    pub num_of_seat: i32,
    pub reservation_type: ReservationType,
    pub seats: Vec<Seat>,
    pub state: ReservationState,
    pub failed_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReservationState {
    Processing,
    Reserved,
    Failed,
    Paid,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationResult {
    pub reservation_id: String,
    pub result: ReservationResultEnum,
    pub error_code: Option<ReservationErrorCode>,
    pub error_message: Option<String>,
    pub seats: Vec<Seat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReservationResultEnum {
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReservationErrorCode {
    InvalidEventArea,
    InvalidArgument,
    SeatNotAvailable,
    InsufficientSeats,
}

impl Reservation {
    pub fn new(create_req: CreateReservation) -> Self {
        Self {
            reservation_id: create_req.reservation_id,
            user_id: create_req.user_id,
            event_id: create_req.event_id,
            area_id: create_req.area_id,
            num_of_seats: create_req.num_of_seats,
            num_of_seat: create_req.num_of_seat,
            reservation_type: create_req.reservation_type,
            seats: create_req.seats,
            state: ReservationState::Processing,
            failed_reason: String::new(),
        }
    }

    pub fn update_from_result(&mut self, result: &ReservationResult) {
        match result.result {
            ReservationResultEnum::Success => {
                self.state = ReservationState::Reserved;
                self.seats = result.seats.clone();
            }
            ReservationResultEnum::Failed => {
                self.state = ReservationState::Failed;
                self.failed_reason = result.error_message.clone().unwrap_or_default();
            }
        }
    }
}
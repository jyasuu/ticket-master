// Kafka topic definitions
pub struct Topics;

impl Topics {
    pub const COMMAND_EVENT_CREATE_EVENT: &'static str = "command.event.create_event";
    pub const COMMAND_EVENT_RESERVE_SEAT: &'static str = "command.event.reserve_seat";
    pub const RESPONSE_RESERVATION_RESULT: &'static str = "response.reservation.result";
    pub const STATE_EVENT_AREA_STATUS: &'static str = "state.event.area_status";
    pub const COMMAND_RESERVATION_CREATE_RESERVATION: &'static str = "command.reservation.create_reservation";
    pub const STATE_USER_RESERVATION: &'static str = "state.user.reservation";
}

// State store definitions
pub struct Stores;

impl Stores {
    pub const AREA_STATUS: &'static str = "AreaStatus";
    pub const RESERVATION: &'static str = "Reservation";
    pub const EVENT_AREA_STATUS_CACHE: &'static str = "eventAreaStatusCache";
}

// Utility functions for key generation
pub fn event_area_key(event_id: &str, area_id: &str) -> String {
    format!("{}#{}", event_id, area_id)
}
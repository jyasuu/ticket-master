use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Area {
    pub area_id: String,
    pub price: i32,
    pub row_count: i32,
    pub col_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEvent {
    pub artist: String,
    pub event_name: String,
    pub reservation_opening_time: DateTime<Utc>,
    pub reservation_closing_time: DateTime<Utc>,
    pub event_start_time: DateTime<Utc>,
    pub event_end_time: DateTime<Utc>,
    pub areas: Vec<Area>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatStatus {
    pub row: i32,
    pub col: i32,
    pub is_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreaStatus {
    pub event_id: String,
    pub area_id: String,
    pub price: i32,
    pub row_count: i32,
    pub col_count: i32,
    pub available_seats: i32,
    pub seats: Vec<Vec<SeatStatus>>,
}

impl AreaStatus {
    pub fn from_area(event_name: &str, area: &Area) -> Self {
        let area_id = area.area_id.clone();
        let row_count = area.row_count;
        let col_count = area.col_count;
        let available_seats = row_count * col_count;
        
        let mut seats = Vec::new();
        for i in 0..row_count {
            let mut row = Vec::new();
            for j in 0..col_count {
                row.push(SeatStatus {
                    row: i,
                    col: j,
                    is_available: true,
                });
            }
            seats.push(row);
        }
        
        Self {
            event_id: event_name.to_string(),
            area_id,
            price: area.price,
            row_count,
            col_count,
            available_seats,
            seats,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveSeat {
    pub reservation_id: String,
    pub event_id: String,
    pub area_id: String,
    pub num_of_seats: i32,
    pub num_of_seat: i32,
    pub reservation_type: ReservationType,
    pub seats: Vec<Seat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seat {
    pub row: i32,
    pub col: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ReservationType {
    SelfPick,
    Random,
    Invalid,
}

impl Default for ReservationType {
    fn default() -> Self {
        Self::Invalid
    }
}
use ticket_master::{
    Result, TicketMasterError, AreaStatus, ReserveSeat, ReservationResult, 
    ReservationResultEnum, ReservationErrorCode, Seat, ReservationType
};
use rand::Rng;

pub trait ReservationStrategy {
    fn reserve(&self, area_status: &mut AreaStatus, request: &ReserveSeat) -> Result<ReservationResult>;
}

pub struct SelfPickStrategy;

impl ReservationStrategy for SelfPickStrategy {
    fn reserve(&self, area_status: &mut AreaStatus, request: &ReserveSeat) -> Result<ReservationResult> {
        let mut result = ReservationResult {
            reservation_id: request.reservation_id.clone(),
            result: ReservationResultEnum::Failed,
            error_code: None,
            error_message: None,
            seats: Vec::new(),
        };

        // Validate requested seats
        for seat in &request.seats {
            let row = seat.row as usize;
            let col = seat.col as usize;

            // Check bounds
            if row >= area_status.seats.len() || col >= area_status.seats[row].len() {
                result.error_code = Some(ReservationErrorCode::InvalidArgument);
                result.error_message = Some(format!("Seat out of bounds: row {}, col {}", seat.row, seat.col));
                return Ok(result);
            }

            // Check availability
            if !area_status.seats[row][col].is_available {
                result.error_code = Some(ReservationErrorCode::SeatNotAvailable);
                result.error_message = Some(format!("Seat not available: row {}, col {}", seat.row, seat.col));
                return Ok(result);
            }
        }

        // All seats are available, reserve them
        result.result = ReservationResultEnum::Success;
        result.seats = request.seats.clone();
        
        Ok(result)
    }
}

pub struct RandomStrategy;

impl ReservationStrategy for RandomStrategy {
    fn reserve(&self, area_status: &mut AreaStatus, request: &ReserveSeat) -> Result<ReservationResult> {
        let mut result = ReservationResult {
            reservation_id: request.reservation_id.clone(),
            result: ReservationResultEnum::Failed,
            error_code: None,
            error_message: None,
            seats: Vec::new(),
        };

        let num_seats_requested = request.num_of_seats;
        
        // Check if enough seats are available
        if area_status.available_seats < num_seats_requested {
            result.error_code = Some(ReservationErrorCode::InsufficientSeats);
            result.error_message = Some(format!(
                "Not enough seats available. Requested: {}, Available: {}", 
                num_seats_requested, area_status.available_seats
            ));
            return Ok(result);
        }

        // Collect all available seats
        let mut available_seats = Vec::new();
        for (row_idx, row) in area_status.seats.iter().enumerate() {
            for (col_idx, seat_status) in row.iter().enumerate() {
                if seat_status.is_available {
                    available_seats.push(Seat {
                        row: row_idx as i32,
                        col: col_idx as i32,
                    });
                }
            }
        }

        // Randomly select seats
        let mut rng = rand::thread_rng();
        let mut selected_seats = Vec::new();
        
        for _ in 0..num_seats_requested {
            if available_seats.is_empty() {
                break;
            }
            
            let idx = rng.gen_range(0..available_seats.len());
            let seat = available_seats.remove(idx);
            selected_seats.push(seat);
        }

        if selected_seats.len() == num_seats_requested as usize {
            result.result = ReservationResultEnum::Success;
            result.seats = selected_seats;
        } else {
            result.error_code = Some(ReservationErrorCode::InsufficientSeats);
            result.error_message = Some("Could not allocate enough seats".to_string());
        }

        Ok(result)
    }
}

// Continuous random strategy that tries to find adjacent seats
pub struct ContinuousRandomStrategy;

impl ReservationStrategy for ContinuousRandomStrategy {
    fn reserve(&self, area_status: &mut AreaStatus, request: &ReserveSeat) -> Result<ReservationResult> {
        let mut result = ReservationResult {
            reservation_id: request.reservation_id.clone(),
            result: ReservationResultEnum::Failed,
            error_code: None,
            error_message: None,
            seats: Vec::new(),
        };

        let num_seats_requested = request.num_of_seats;
        
        // Check if enough seats are available
        if area_status.available_seats < num_seats_requested {
            result.error_code = Some(ReservationErrorCode::InsufficientSeats);
            result.error_message = Some(format!(
                "Not enough seats available. Requested: {}, Available: {}", 
                num_seats_requested, area_status.available_seats
            ));
            return Ok(result);
        }

        // Try to find continuous seats in each row
        for (row_idx, row) in area_status.seats.iter().enumerate() {
            let mut continuous_seats = Vec::new();
            
            for (col_idx, seat_status) in row.iter().enumerate() {
                if seat_status.is_available {
                    continuous_seats.push(Seat {
                        row: row_idx as i32,
                        col: col_idx as i32,
                    });
                    
                    if continuous_seats.len() == num_seats_requested as usize {
                        result.result = ReservationResultEnum::Success;
                        result.seats = continuous_seats;
                        return Ok(result);
                    }
                } else {
                    continuous_seats.clear();
                }
            }
        }

        // If no continuous seats found, fall back to random selection
        let random_strategy = RandomStrategy;
        random_strategy.reserve(area_status, request)
    }
}
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

// Advanced continuous random strategy - equivalent to Java's ContinuousRandomStrategy
// Implements the sophisticated sliding window algorithm from Java
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
        
        // Validate input
        if num_seats_requested <= 0 {
            result.error_code = Some(ReservationErrorCode::InvalidArgument);
            result.error_message = Some(format!("{} continuous seats is invalid", num_seats_requested));
            return Ok(result);
        }

        let row_count = area_status.row_count as usize;
        let col_count = area_status.col_count as usize;
        
        // Implement Java's sophisticated sliding window algorithm
        for row_idx in 0..row_count {
            let row_seats = &area_status.seats[row_idx];
            let mut left = 0;
            
            // Sliding window approach - equivalent to Java's while loop
            while num_seats_requested as usize <= col_count - left {
                // Skip unavailable starting seats
                if !row_seats[left].is_available {
                    left += 1;
                    continue;
                }
                
                // Expand window to the right
                let mut right = left + 1;
                while right < left + num_seats_requested as usize {
                    if right >= col_count || !row_seats[right].is_available {
                        // Gap found, move left pointer past the gap
                        left = right + 1;
                        break;
                    }
                    right += 1;
                }
                
                // Check if we found enough continuous seats
                if right - left == num_seats_requested as usize {
                    // Found continuous seats - create the seat list
                    let mut seats = Vec::new();
                    for col in left..right {
                        seats.push(Seat {
                            row: row_idx as i32,
                            col: col as i32,
                        });
                    }
                    
                    result.result = ReservationResultEnum::Success;
                    result.seats = seats;
                    return Ok(result);
                }
                
                // If we didn't break out of the inner loop, increment left
                if right == left + num_seats_requested as usize {
                    left += 1;
                }
            }
        }

        // No continuous seats found - return failure with detailed message
        result.error_code = Some(ReservationErrorCode::SeatNotAvailable);
        result.error_message = Some(format!(
            "no continuous {} seats at area {} in event {}", 
            num_seats_requested, 
            request.area_id, 
            request.event_id
        ));
        
        Ok(result)
    }
}
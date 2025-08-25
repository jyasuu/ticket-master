/// Avro schema definitions matching the Java implementation
pub mod schemas {
    
    pub const CREATE_EVENT_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "CreateEvent",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "artist", "type": "string"},
            {"name": "eventName", "type": "string"},
            {"name": "reservationOpeningTime", "type": "long"},
            {"name": "reservationClosingTime", "type": "long"},
            {"name": "eventStartTime", "type": "long"},
            {"name": "eventEndTime", "type": "long"},
            {"name": "areas", "type": {"type": "array", "items": {
                "type": "record",
                "name": "Area",
                "fields": [
                    {"name": "areaId", "type": "string"},
                    {"name": "price", "type": "int"},
                    {"name": "rowCount", "type": "int"},
                    {"name": "colCount", "type": "int"}
                ]
            }}}
        ]
    }
    "#;

    pub const AREA_STATUS_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "AreaStatus",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "eventName", "type": "string"},
            {"name": "areaId", "type": "string"},
            {"name": "availableSeats", "type": "int"},
            {"name": "totalSeats", "type": "int"},
            {"name": "seats", "type": {"type": "array", "items": {"type": "array", "items": {
                "type": "record",
                "name": "SeatStatus",
                "fields": [
                    {"name": "row", "type": "int"},
                    {"name": "col", "type": "int"},
                    {"name": "isAvailable", "type": "boolean"}
                ]
            }}}}
        ]
    }
    "#;

    pub const RESERVE_SEAT_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "ReserveSeat",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "reservationId", "type": "string"},
            {"name": "userId", "type": "string"},
            {"name": "eventId", "type": "string"},
            {"name": "areaId", "type": "string"},
            {"name": "numOfSeats", "type": "int"},
            {"name": "reservationType", "type": {
                "type": "enum",
                "name": "ReservationType",
                "symbols": ["SELF_PICK", "RANDOM"]
            }},
            {"name": "seats", "type": {"type": "array", "items": {
                "type": "record",
                "name": "Seat",
                "fields": [
                    {"name": "row", "type": "int"},
                    {"name": "col", "type": "int"}
                ]
            }}}
        ]
    }
    "#;

    pub const CREATE_RESERVATION_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "CreateReservation",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "reservationId", "type": "string"},
            {"name": "userId", "type": "string"},
            {"name": "eventId", "type": "string"},
            {"name": "areaId", "type": "string"},
            {"name": "numOfSeats", "type": "int"},
            {"name": "numOfSeat", "type": "int"},
            {"name": "reservationType", "type": {
                "type": "enum",
                "name": "ReservationType",
                "symbols": ["SELF_PICK", "RANDOM"]
            }},
            {"name": "seats", "type": {"type": "array", "items": {
                "type": "record",
                "name": "Seat",
                "fields": [
                    {"name": "row", "type": "int"},
                    {"name": "col", "type": "int"}
                ]
            }}}
        ]
    }
    "#;

    pub const RESERVATION_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "Reservation",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "reservationId", "type": "string"},
            {"name": "userId", "type": "string"},
            {"name": "eventId", "type": "string"},
            {"name": "areaId", "type": "string"},
            {"name": "numOfSeats", "type": "int"},
            {"name": "reservationType", "type": {
                "type": "enum",
                "name": "ReservationType",
                "symbols": ["SELF_PICK", "RANDOM"]
            }},
            {"name": "seats", "type": {"type": "array", "items": {
                "type": "record",
                "name": "Seat",
                "fields": [
                    {"name": "row", "type": "int"},
                    {"name": "col", "type": "int"}
                ]
            }}},
            {"name": "state", "type": {
                "type": "enum",
                "name": "ReservationState",
                "symbols": ["PENDING", "CONFIRMED", "CANCELLED", "EXPIRED"]
            }},
            {"name": "createdAt", "type": "long"},
            {"name": "updatedAt", "type": "long"}
        ]
    }
    "#;

    pub const RESERVATION_RESULT_SCHEMA: &str = r#"
    {
        "type": "record",
        "name": "ReservationResult",
        "namespace": "lab.tall15421542.app.domain.beans",
        "fields": [
            {"name": "reservationId", "type": "string"},
            {"name": "result", "type": {
                "type": "enum",
                "name": "ReservationResultEnum",
                "symbols": ["SUCCESS", "FAILURE", "INSUFFICIENT_SEATS", "SEAT_NOT_AVAILABLE"]
            }},
            {"name": "seats", "type": {"type": "array", "items": {
                "type": "record",
                "name": "Seat",
                "fields": [
                    {"name": "row", "type": "int"},
                    {"name": "col", "type": "int"}
                ]
            }}},
            {"name": "errorCode", "type": ["null", {
                "type": "enum",
                "name": "ReservationErrorCode",
                "symbols": ["SEAT_UNAVAILABLE", "INSUFFICIENT_SEATS", "INVALID_AREA", "INVALID_EVENT"]
            }], "default": null},
            {"name": "errorMessage", "type": ["null", "string"], "default": null}
        ]
    }
    "#;
}

/// Schema subjects for Schema Registry
pub mod subjects {
    pub const CREATE_EVENT: &str = "create-event-value";
    pub const AREA_STATUS: &str = "area-status-value";
    pub const RESERVE_SEAT: &str = "reserve-seat-value";
    pub const CREATE_RESERVATION: &str = "create-reservation-value";
    pub const RESERVATION: &str = "reservation-value";
    pub const RESERVATION_RESULT: &str = "reservation-result-value";
}

/// Initialize all schemas in the serializer
pub fn load_all_schemas(serializer: &mut crate::AvroSerializer) -> crate::Result<()> {
    use apache_avro::Schema;
    
    // Parse and load all schemas
    let schemas = [
        ("create_event", schemas::CREATE_EVENT_SCHEMA),
        ("area_status", schemas::AREA_STATUS_SCHEMA),
        ("reserve_seat", schemas::RESERVE_SEAT_SCHEMA),
        ("create_reservation", schemas::CREATE_RESERVATION_SCHEMA),
        ("reservation", schemas::RESERVATION_SCHEMA),
        ("reservation_result", schemas::RESERVATION_RESULT_SCHEMA),
    ];

    for (name, schema_str) in schemas {
        let schema = Schema::parse_str(schema_str)?;
        serializer.schemas.insert(name.to_string(), schema);
    }

    Ok(())
}
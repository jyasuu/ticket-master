# Event Ticketing System - Sequence Flow Diagram

This diagram shows the complete flow of the event ticketing system built with Kafka Streams.

## System Overview

The system consists of three main services:
- **Ticket Service**: REST API gateway that handles HTTP requests
- **Event Service**: Manages event creation and seat reservations using Kafka Streams
- **Reservation Service**: Processes reservation requests and manages reservation state

## Mermaid Sequence Diagram

```mermaid
sequenceDiagram
    participant Client
    participant TicketService as Ticket Service<br/>(REST API)
    participant EventTopic as command.event.create_event<br/>(Kafka Topic)
    participant EventService as Event Service<br/>(Kafka Streams)
    participant AreaStatusTopic as state.event.area_status<br/>(Kafka Topic)
    participant ReservationTopic as command.reservation.create_reservation<br/>(Kafka Topic)
    participant ReservationService as Reservation Service<br/>(Kafka Streams)
    participant ReserveSeatTopic as command.event.reserve_seat<br/>(Kafka Topic)
    participant ReservationResultTopic as response.reservation.result<br/>(Kafka Topic)
    participant UserReservationTopic as state.user.reservation<br/>(Kafka Topic)

    Note over Client, UserReservationTopic: Event Creation Flow
    
    Client->>TicketService: POST /v1/event<br/>(EventBean)
    TicketService->>EventTopic: Publish CreateEvent
    TicketService-->>Client: Return EventBean (async)
    
    EventTopic->>EventService: Consume CreateEvent
    EventService->>EventService: Process CreateEvent<br/>- Convert areas to AreaStatus<br/>- Initialize seat availability
    EventService->>AreaStatusTopic: Publish AreaStatus for each area
    EventService->>EventService: Store AreaStatus in local state store
    
    AreaStatusTopic->>ReservationService: GlobalTable consumption<br/>Build eventAreaStatusCache (LRU)
    ReservationService->>ReservationService: Maintain local AreaStatus cache<br/>for reservation filtering

    Note over Client, UserReservationTopic: Reservation Flow
    
    Client->>TicketService: POST /v1/event/{id}/reservation<br/>(CreateReservationBean)
    TicketService->>ReservationTopic: Publish CreateReservation<br/>(with generated reservationId)
    TicketService-->>Client: Return reservationId (async)
    
    ReservationTopic->>ReservationService: Consume CreateReservation
    ReservationService->>ReservationService: ReservationValueProcessor<br/>- Create Reservation object<br/>- Check eventAreaStatusCache (GlobalTable)<br/>- Apply filter strategy (SelfPick/Random)
    
    alt Area status available in cache and passes filter
        ReservationService->>ReservationService: Set state to PROCESSING
    else Area not in cache or fails filter
        ReservationService->>ReservationService: Set state to FAILED
    end
    
    ReservationService->>ReservationService: Store reservation in state store
    
    alt Reservation state is PROCESSING
        ReservationService->>ReserveSeatTopic: Publish ReserveSeat request
        
        ReserveSeatTopic->>EventService: Consume ReserveSeat
        EventService->>EventService: ReserveSeatTransformer<br/>- Get AreaStatus from store<br/>- Apply reservation strategy<br/>- Update seat availability
        
        alt Reservation successful
            EventService->>EventService: Mark seats as unavailable<br/>Update AreaStatus
            EventService->>AreaStatusTopic: Publish updated AreaStatus
            EventService->>ReservationResultTopic: Publish SUCCESS result
        else Reservation failed
            EventService->>ReservationResultTopic: Publish FAILED result
        end
        
        ReservationResultTopic->>ReservationService: Consume ReservationResult
        ReservationService->>ReservationService: ReservationResultValueProcessor<br/>- Update reservation state<br/>- Set seats if successful<br/>- Set failure reason if failed
        ReservationService->>ReservationService: Update reservation in state store
    end
    
    ReservationService->>UserReservationTopic: Publish final Reservation state
    
    Note over Client, UserReservationTopic: Reservation Status Query Flow
    
    UserReservationTopic->>TicketService: Consume Reservation updates
    TicketService->>TicketService: Update local reservation store<br/>Resume pending async responses
    
    Client->>TicketService: GET /v1/reservation/{reservationId}
    
    alt Reservation data is local
        TicketService->>TicketService: Query local state store
        TicketService-->>Client: Return ReservationBean
    else Reservation data is on another instance
        TicketService->>TicketService: Discover host via Kafka Streams metadata
        TicketService->>TicketService: HTTP call to other instance
        TicketService-->>Client: Return ReservationBean
    else Reservation not yet available
        TicketService->>TicketService: Add to outstanding requests<br/>Wait for Kafka update
        Note over TicketService: Async response will be resumed<br/>when reservation update arrives
    end

    Note over Client, UserReservationTopic: Key Components & Strategies
    
    Note over EventService: Reservation Strategies:<br/>- SelfPickStrategy: User selects specific seats<br/>- ContinuousRandomStrategy: System assigns random seats
    
    Note over ReservationService: Filter Strategies:<br/>- SelfPickFilterStrategy: Validates seat selection<br/>- ContinuousRandomFilterStrategy: Checks availability
    
    Note over TicketService: Features:<br/>- Async HTTP responses<br/>- Distributed state queries<br/>- Virtual thread pool<br/>- OpenTelemetry tracing
```

## Key Kafka Topics

| Topic | Purpose | Key | Value |
|-------|---------|-----|-------|
| `command.event.create_event` | Event creation commands | Event name | CreateEvent |
| `command.event.reserve_seat` | Seat reservation commands | eventId#areaId | ReserveSeat |
| `command.reservation.create_reservation` | Reservation creation commands | Reservation ID | CreateReservation |
| `response.reservation.result` | Reservation processing results | Reservation ID | ReservationResult |
| `state.event.area_status` | Event area status updates | eventId#areaId | AreaStatus |
| `state.user.reservation` | Final reservation states | Reservation ID | Reservation |

## State Stores

| Store | Service | Purpose | Key | Value |
|-------|---------|---------|-----|-------|
| `AreaStatus` | Event Service | Track seat availability | eventId#areaId | AreaStatus |
| `Reservation` | Reservation Service | Track reservation state | Reservation ID | Reservation |
| `eventAreaStatusCache` | Reservation Service | **GlobalTable** LRU cache (1000 entries) for filtering | eventId#areaId | AreaStatus |

## Architecture Highlights

1. **Event-Driven Architecture**: Uses Kafka topics for async communication between services
2. **CQRS Pattern**: Separate command and query responsibilities
3. **Distributed State Management**: Each service maintains its own state stores
4. **Async Processing**: Non-blocking HTTP responses with virtual threads
5. **Exactly-Once Processing**: Kafka Streams guarantees for data consistency
6. **Horizontal Scalability**: Services can be scaled independently
7. **Fault Tolerance**: Built-in retry mechanisms and error handling
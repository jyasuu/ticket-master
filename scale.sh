#!/bin/bash

# Scalable Docker Compose Management Script for Ticket Master Services
# Usage: ./scale.sh [java|rust] [up|down|scale|status|logs|test]

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
JAVA_COMPOSE_FILE="docker-compose.scalable.yml"
RUST_COMPOSE_FILE="docker-compose.rust.scalable.yml"
PROJECT_NAME="ticket-master-scalable"

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if Docker and Docker Compose are available
check_prerequisites() {
    if ! command -v docker &> /dev/null; then
        print_error "Docker is not installed or not in PATH"
        exit 1
    fi

    if ! command -v docker-compose &> /dev/null; then
        print_error "Docker Compose is not installed or not in PATH"
        exit 1
    fi

    print_success "Prerequisites check passed"
}

# Function to create necessary directories
create_directories() {
    print_status "Creating state directories..."
    
    # Create state directories for persistent storage
    mkdir -p state/ticket-service-{1,2,3}
    mkdir -p state/event-service-{1,2}
    mkdir -p state/reservation-service-{1,2}
    mkdir -p state/rust/ticket-service-{1,2,3}
    mkdir -p state/rust/event-service-{1,2}
    mkdir -p state/rust/reservation-service-{1,2}
    
    print_success "State directories created"
}

# Function to start services
start_services() {
    local service_type=$1
    local compose_file=""
    
    if [ "$service_type" = "java" ]; then
        compose_file=$JAVA_COMPOSE_FILE
        print_status "Starting Java-based scalable services..."
    elif [ "$service_type" = "rust" ]; then
        compose_file=$RUST_COMPOSE_FILE
        print_status "Starting Rust-based scalable services..."
    else
        print_error "Invalid service type. Use 'java' or 'rust'"
        exit 1
    fi

    create_directories

    print_status "Building and starting services with $compose_file..."
    docker-compose -f $compose_file -p $PROJECT_NAME up -d --build

    print_status "Waiting for services to be healthy..."
    sleep 30

    print_success "Services started successfully!"
    print_status "Access points:"
    echo "  - Load Balanced API: http://localhost:8080"
    echo "  - Kafka UI: http://localhost:9000"
    echo "  - Jaeger Tracing: http://localhost:16686"
    echo "  - Prometheus: http://localhost:9090"
}

# Function to stop services
stop_services() {
    local service_type=$1
    local compose_file=""
    
    if [ "$service_type" = "java" ]; then
        compose_file=$JAVA_COMPOSE_FILE
    elif [ "$service_type" = "rust" ]; then
        compose_file=$RUST_COMPOSE_FILE
    else
        print_error "Invalid service type. Use 'java' or 'rust'"
        exit 1
    fi

    print_status "Stopping services..."
    docker-compose -f $compose_file -p $PROJECT_NAME down -v
    print_success "Services stopped successfully!"
}

# Function to scale specific services
scale_services() {
    local service_type=$1
    local service_name=$2
    local replica_count=$3
    local compose_file=""
    
    if [ "$service_type" = "java" ]; then
        compose_file=$JAVA_COMPOSE_FILE
    elif [ "$service_type" = "rust" ]; then
        compose_file=$RUST_COMPOSE_FILE
    else
        print_error "Invalid service type. Use 'java' or 'rust'"
        exit 1
    fi

    if [ -z "$service_name" ] || [ -z "$replica_count" ]; then
        print_error "Usage: ./scale.sh $service_type scale <service_name> <replica_count>"
        print_status "Available services:"
        if [ "$service_type" = "java" ]; then
            echo "  - ticket-service"
            echo "  - event-service"
            echo "  - reservation-service"
        else
            echo "  - ticket-service-rust"
            echo "  - event-service-rust"
            echo "  - reservation-service-rust"
        fi
        exit 1
    fi

    print_status "Scaling $service_name to $replica_count replicas..."
    docker-compose -f $compose_file -p $PROJECT_NAME up -d --scale $service_name=$replica_count
    print_success "Service scaled successfully!"
}

# Function to show service status
show_status() {
    local service_type=$1
    local compose_file=""
    
    if [ "$service_type" = "java" ]; then
        compose_file=$JAVA_COMPOSE_FILE
    elif [ "$service_type" = "rust" ]; then
        compose_file=$RUST_COMPOSE_FILE
    else
        print_error "Invalid service type. Use 'java' or 'rust'"
        exit 1
    fi

    print_status "Service Status:"
    docker-compose -f $compose_file -p $PROJECT_NAME ps

    print_status "Resource Usage:"
    docker stats --no-stream --format "table {{.Container}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}" $(docker-compose -f $compose_file -p $PROJECT_NAME ps -q) 2>/dev/null || true
}

# Function to show logs
show_logs() {
    local service_type=$1
    local service_name=$2
    local compose_file=""
    
    if [ "$service_type" = "java" ]; then
        compose_file=$JAVA_COMPOSE_FILE
    elif [ "$service_type" = "rust" ]; then
        compose_file=$RUST_COMPOSE_FILE
    else
        print_error "Invalid service type. Use 'java' or 'rust'"
        exit 1
    fi

    if [ -z "$service_name" ]; then
        print_status "Showing logs for all services..."
        docker-compose -f $compose_file -p $PROJECT_NAME logs -f --tail=100
    else
        print_status "Showing logs for $service_name..."
        docker-compose -f $compose_file -p $PROJECT_NAME logs -f --tail=100 $service_name
    fi
}

# Function to run basic tests
run_tests() {
    local service_type=$1
    
    print_status "Running basic connectivity tests..."
    
    # Test load balancer health
    if curl -f http://localhost:8080/health > /dev/null 2>&1; then
        print_success "Load balancer is healthy"
    else
        print_error "Load balancer health check failed"
        return 1
    fi
    
    # Test service health
    if curl -f http://localhost:8080/v1/health_check > /dev/null 2>&1; then
        print_success "Service health check passed"
    else
        print_error "Service health check failed"
        return 1
    fi
    
    # Test Kafka UI
    if curl -f http://localhost:9000 > /dev/null 2>&1; then
        print_success "Kafka UI is accessible"
    else
        print_warning "Kafka UI is not accessible"
    fi
    
    # Test Jaeger
    if curl -f http://localhost:16686 > /dev/null 2>&1; then
        print_success "Jaeger UI is accessible"
    else
        print_warning "Jaeger UI is not accessible"
    fi
    
    print_success "Basic tests completed!"
}

# Function to show help
show_help() {
    echo "Scalable Docker Compose Management Script for Ticket Master Services"
    echo ""
    echo "Usage: $0 [java|rust] [command] [options]"
    echo ""
    echo "Commands:"
    echo "  up                    Start all services"
    echo "  down                  Stop all services"
    echo "  scale <service> <n>   Scale specific service to n replicas"
    echo "  status                Show service status and resource usage"
    echo "  logs [service]        Show logs (all services or specific service)"
    echo "  test                  Run basic connectivity tests"
    echo "  help                  Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 java up                           # Start Java services"
    echo "  $0 rust up                           # Start Rust services"
    echo "  $0 java scale ticket-service 5       # Scale Java ticket service to 5 replicas"
    echo "  $0 rust scale ticket-service-rust 3  # Scale Rust ticket service to 3 replicas"
    echo "  $0 java status                       # Show Java services status"
    echo "  $0 rust logs ticket-service-rust-1   # Show logs for specific Rust service"
    echo "  $0 java test                         # Test Java services"
}

# Main script logic
main() {
    check_prerequisites

    if [ $# -lt 2 ]; then
        show_help
        exit 1
    fi

    local service_type=$1
    local command=$2

    case $command in
        "up")
            start_services $service_type
            ;;
        "down")
            stop_services $service_type
            ;;
        "scale")
            scale_services $service_type $3 $4
            ;;
        "status")
            show_status $service_type
            ;;
        "logs")
            show_logs $service_type $3
            ;;
        "test")
            run_tests $service_type
            ;;
        "help")
            show_help
            ;;
        *)
            print_error "Unknown command: $command"
            show_help
            exit 1
            ;;
    esac
}

# Run main function with all arguments
main "$@"
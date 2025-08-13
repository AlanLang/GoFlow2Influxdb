# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

GoFlow2Influxdb is a Rust application that synchronizes network flow data from GoFlow2 to InfluxDB. The application receives JSON-formatted network flow data from GoFlow2 (listening on netflow://:2055,sflow://:6343) and stores it in InfluxDB after filtering out LAN traffic.

### Data Flow
1. GoFlow2 listens for network flow data: `goflow2 -listen "netflow://:2055,sflow://:6343" -format json`
2. Application receives JSON data containing network flow information (NETFLOW_V5, SFLOW, etc.)
3. Filters data to exclude LAN source addresses (private IP ranges)
4. Stores filtered data in InfluxDB for analysis and monitoring

## Development Commands

```bash
# Build the project
cargo build

# Build for release
cargo build --release

# Run the application
cargo run

# Check code without building
cargo check

# Format code
cargo fmt

# Run lints
cargo clippy

# Run tests
cargo test

# Run a specific test
cargo test test_name

# Clean build artifacts
cargo clean
```

## Project Structure

- `src/main.rs` - Entry point with basic "Hello, world!" implementation
- `Cargo.toml` - Project configuration using Rust 2024 edition
- No external dependencies currently defined

## Data Format

GoFlow2 outputs JSON data with the following structure:
```json
{
  "type": "NETFLOW_V5",
  "time_received_ns": 1755046784453621258,
  "sequence_num": 96,
  "sampling_rate": 0,
  "sampler_address": "192.168.1.1",
  "time_flow_start_ns": 1755046766677193800,
  "time_flow_end_ns": 1755046766677193800,
  "bytes": 118,
  "packets": 1,
  "src_addr": "192.168.1.1",
  "dst_addr": "192.168.1.33",
  "etype": "IPv4",
  "proto": "UDP",
  "src_port": 53,
  "dst_port": 63461,
  "in_if": 0,
  "out_if": 11
  // ... additional fields
}
```

## Filtering Requirements

- Only keep traffic between LAN and WAN (internal-external communication)
- Filter out:
  - LAN to LAN traffic (internal network communication)
  - WAN to WAN traffic (external traffic not involving local network)
- Keep:
  - LAN to WAN traffic (upload from internal devices)
  - WAN to LAN traffic (download to internal devices)
- Private IP ranges:
  - 10.0.0.0/8 (10.0.0.0 - 10.255.255.255)
  - 172.16.0.0/12 (172.16.0.0 - 172.31.255.255)  
  - 192.168.0.0/16 (192.168.0.0 - 192.168.255.255)

## Architecture Notes

The application architecture involves:
- JSON data ingestion from GoFlow2 
- IP address filtering (exclude private/LAN source addresses)
- InfluxDB client integration for time-series data storage
- Data transformation for optimal InfluxDB schema

Key dependencies needed:
- JSON parsing (serde_json)
- InfluxDB client library
- IP address parsing and validation
- Async runtime (tokio)
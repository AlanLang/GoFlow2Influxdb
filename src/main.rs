use anyhow::Result;
use dotenv::dotenv;
use futures::stream;
use influxdb2::{Client, models::DataPoint};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use std::{env, net::Ipv4Addr, str::FromStr, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::sleep;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize, Serialize)]
struct FlowData {
    #[serde(rename = "type")]
    flow_type: String,
    time_received_ns: u64,
    sequence_num: u32,
    sampling_rate: u32,
    sampler_address: String,
    time_flow_start_ns: u64,
    time_flow_end_ns: u64,
    bytes: u64,
    packets: u64,
    src_addr: String,
    dst_addr: String,
    etype: String,
    proto: String,
    src_port: u16,
    dst_port: u16,
    in_if: u32,
    out_if: u32,
    src_mac: Option<String>,
    dst_mac: Option<String>,
    src_vlan: Option<u16>,
    dst_vlan: Option<u16>,
    vlan_id: Option<u16>,
    ip_tos: Option<u8>,
    forwarding_status: Option<u8>,
    ip_ttl: Option<u8>,
    ip_flags: Option<u16>,
    tcp_flags: Option<u16>,
    icmp_type: Option<u8>,
    icmp_code: Option<u8>,
    ipv6_flow_label: Option<u32>,
    fragment_id: Option<u32>,
    fragment_offset: Option<u32>,
    src_as: Option<u32>,
    dst_as: Option<u32>,
    next_hop: Option<String>,
    next_hop_as: Option<u32>,
    src_net: Option<String>,
    dst_net: Option<String>,
    bgp_next_hop: Option<String>,
    bgp_communities: Option<Vec<String>>,
    as_path: Option<Vec<u32>>,
    mpls_ttl: Option<Vec<u8>>,
    mpls_label: Option<Vec<u32>>,
    mpls_ip: Option<Vec<String>>,
    observation_domain_id: Option<u32>,
    observation_point_id: Option<u32>,
}

#[derive(Debug)]
struct Config {
    influxdb_url: String,
    influxdb_token: String,
    influxdb_org: String,
    influxdb_bucket: String,
    goflow2_input_file: String,
    batch_size: usize,
    flush_interval_seconds: u64,
    retry_attempts: u32,
    retry_delay_ms: u64,
}

impl Config {
    fn from_env() -> Result<Self> {
        Ok(Config {
            influxdb_url: env::var("INFLUXDB_URL")?,
            influxdb_token: env::var("INFLUXDB_TOKEN")?,
            influxdb_org: env::var("INFLUXDB_ORG")?,
            influxdb_bucket: env::var("INFLUXDB_BUCKET")?,
            goflow2_input_file: env::var("GOFLOW2_INPUT_FILE")
                .unwrap_or_else(|_| "/dev/stdin".to_string()),
            batch_size: env::var("BATCH_SIZE")
                .unwrap_or_else(|_| "100".to_string())
                .parse()?,
            flush_interval_seconds: env::var("FLUSH_INTERVAL_SECONDS")
                .unwrap_or_else(|_| "10".to_string())
                .parse()?,
            retry_attempts: env::var("RETRY_ATTEMPTS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()?,
            retry_delay_ms: env::var("RETRY_DELAY_MS")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()?,
        })
    }
}

fn is_private_ip(ip_str: &str) -> bool {
    if let Ok(ip) = Ipv4Addr::from_str(ip_str) {
        let private_ranges = [
            Ipv4Net::from_str("10.0.0.0/8").unwrap(),
            Ipv4Net::from_str("172.16.0.0/12").unwrap(),
            Ipv4Net::from_str("192.168.0.0/16").unwrap(),
        ];

        private_ranges.iter().any(|range| range.contains(&ip))
    } else {
        false
    }
}

async fn write_batch_with_retry(
    client: &Client,
    bucket: &str,
    batch: Vec<DataPoint>,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<()> {
    for attempt in 1..=retry_attempts {
        match client.write(bucket, stream::iter(batch.clone())).await {
            Ok(_) => {
                info!("Successfully wrote batch of {} points to InfluxDB", batch.len());
                return Ok(());
            }
            Err(e) => {
                if attempt == retry_attempts {
                    return Err(anyhow::anyhow!("Failed to write batch after {} attempts: {}", retry_attempts, e));
                }
                warn!("Attempt {}/{} failed: {}. Retrying in {}ms...", attempt, retry_attempts, e, retry_delay_ms);
                sleep(Duration::from_millis(retry_delay_ms)).await;
            }
        }
    }
    unreachable!()
}

fn flow_to_datapoint(flow: &FlowData) -> DataPoint {
    let timestamp = flow.time_received_ns as i64;

    DataPoint::builder("netflow")
        .tag("flow_type", &flow.flow_type)
        .tag("src_addr", &flow.src_addr)
        .tag("dst_addr", &flow.dst_addr)
        .tag("proto", &flow.proto)
        .tag("sampler_address", &flow.sampler_address)
        .field("bytes", flow.bytes as i64)
        .field("packets", flow.packets as i64)
        .field("src_port", flow.src_port as i64)
        .field("dst_port", flow.dst_port as i64)
        .field("sequence_num", flow.sequence_num as i64)
        .field("sampling_rate", flow.sampling_rate as i64)
        .field("time_flow_start_ns", flow.time_flow_start_ns as i64)
        .field("time_flow_end_ns", flow.time_flow_end_ns as i64)
        .field("in_if", flow.in_if as i64)
        .field("out_if", flow.out_if as i64)
        .timestamp(timestamp)
        .build()
        .expect("Failed to build DataPoint")
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    tracing_subscriber::fmt::init();

    let config = Config::from_env()?;
    info!("Starting GoFlow2Influxdb with config: {:?}", config);

    let client = Client::new(
        &config.influxdb_url,
        &config.influxdb_org,
        &config.influxdb_token,
    );

    let input: Box<dyn tokio::io::AsyncRead + Unpin> = if config.goflow2_input_file == "/dev/stdin"
    {
        Box::new(tokio::io::stdin())
    } else {
        Box::new(tokio::fs::File::open(&config.goflow2_input_file).await?)
    };

    let reader = BufReader::new(input);
    let mut lines = reader.lines();
    let mut batch = Vec::new();
    let mut total_processed = 0u64;
    let mut filtered_out = 0u64;

    info!("Starting to process flow data...");

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<FlowData>(&line) {
            Ok(flow) => {
                total_processed += 1;

                if !is_private_ip(&flow.src_addr) {
                    filtered_out += 1;
                    continue;
                }

                let datapoint = flow_to_datapoint(&flow);
                batch.push(datapoint);

                if batch.len() >= config.batch_size {
                    let batch_to_write: Vec<_> = batch.drain(..).collect();

                    if let Err(e) = write_batch_with_retry(
                        &client,
                        &config.influxdb_bucket,
                        batch_to_write,
                        config.retry_attempts,
                        config.retry_delay_ms,
                    ).await {
                        error!("Failed to write batch to InfluxDB: {}", e);
                    }

                    // Add delay between batch writes to reduce load
                    sleep(Duration::from_millis(config.flush_interval_seconds * 1000)).await;
                }

                if total_processed % 1000 == 0 {
                    info!(
                        "Processed: {}, Filtered: {}, Pending: {}",
                        total_processed,
                        filtered_out,
                        batch.len()
                    );
                }
            }
            Err(e) => {
                warn!("Failed to parse JSON line: {} - Error: {}", line, e);
            }
        }
    }

    if !batch.is_empty() {
        if let Err(e) = write_batch_with_retry(
            &client,
            &config.influxdb_bucket,
            batch,
            config.retry_attempts,
            config.retry_delay_ms,
        ).await {
            error!("Failed to write final batch to InfluxDB: {}", e);
        }
    }

    info!(
        "Processing completed. Total: {}, Filtered: {}",
        total_processed, filtered_out
    );

    Ok(())
}

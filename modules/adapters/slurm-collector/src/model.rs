use chrono::{DateTime, Utc};
use common_grpc::middleware::client::LayeredApiClientService;
use metering_grpc::meteroid::metering::v1::events_service_client::EventsServiceClient;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Sacct command failed: {0}")]
    SacctError(String),

    #[error("Failed to parse sacct output: {0}")]
    InvalidSacctOutput(String),

    #[error("Date parsing error: {0}")]
    DateParseError(#[from] chrono::ParseError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Error setting up grpc connection : {0}")]
    TonicConnectionError(#[from] tonic::transport::Error),

    #[error("Tonic error: {0}")]
    TonicStatusError(#[from] tonic::Status),

    #[error("Ingestion was rejected by the server for some records")]
    IngestError,
}

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(clap::Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct AppConfig {
    #[clap(long, default_value = "300", help = "Polling interval in seconds")]
    pub poll_interval: u64,

    #[clap(long, env = "METEROID_INGEST_ENDPOINT", help = "API endpoint URL")]
    pub api_endpoint: String,

    #[clap(
        long,
        env = "METEROID_API_KEY",
        hide_env_values = true,
        help = "API key for authentication"
    )]
    pub api_key: String,

    #[clap(long, default_value = "checkpoint.db", help = "Path to the state file")]
    pub state_file: String,

    #[clap(
        long,
        default_value = "200",
        help = "Number of records to process in each batch"
    )]
    pub batch_size: usize,

    #[clap(
        long,
        default_value = "2020-01-01T00:00:00Z",
        help = "Initial checkpoint date in RFC 3339 format"
    )]
    pub initial_checkpoint: String,
}

pub type GrpcClient = EventsServiceClient<LayeredApiClientService>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SacctData {
    pub id: String,
    pub job_id: String,
    pub account: String,
    pub start_time: DateTime<Utc>,
    pub elapsed_seconds: i64,
    pub state: String,
    pub end_time: DateTime<Utc>,
    pub req_cpu: i64,
    pub req_mem: i64,
    pub partition: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub last_processed_time: DateTime<Utc>,
    pub processed_jobs: Vec<String>,
}

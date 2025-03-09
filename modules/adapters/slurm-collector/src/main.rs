mod model;

use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use common_grpc::middleware::client::{LayeredApiClientService, build_api_layered_client_service};
use futures::StreamExt;
use futures_util::stream::BoxStream;
use log::{error, info};
use metering_grpc::meteroid::metering::v1::events_service_client::EventsServiceClient;
use std::fs::File as FsFile;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tokio::time::{self, Duration};
use tonic::transport::Channel;

use model::{AppConfig, AppError, Checkpoint, GrpcClient, Result, SacctData};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    info!("Starting Slurm sacct processor");

    let config = AppConfig::parse();

    let mut client = build_client(&config).await?;
    let mut interval = time::interval(Duration::from_secs(config.poll_interval));

    let sacct_executor = SacctExecutorImpl;
    loop {
        interval.tick().await;
        if let Err(e) = process_sacct_data(&config, &mut client, &sacct_executor).await {
            error!("Error processing billing data: {:?}", e);
        }
    }
}

async fn build_client(config: &AppConfig) -> Result<EventsServiceClient<LayeredApiClientService>> {
    log::info!("Connecting to API endpoint: {}", config.api_endpoint);

    let channel = Channel::from_shared(config.api_endpoint.clone())
        .expect("Invalid ingest endpoint")
        .connect()
        .await
        .map_err(AppError::TonicConnectionError)?;

    let service = build_api_layered_client_service(channel, &config.api_key);

    let client = EventsServiceClient::new(service);

    Ok(client)
}

trait SacctExecutor {
    fn sacct(&self, since: DateTime<Utc>) -> Result<BoxStream<Result<SacctData>>>;
}

struct SacctExecutorImpl;

const SACCT_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

// TODO allow dynamically setting this in CLI
const SACCT_FIELDS: [&str; 9] = [
    "JobID",
    "Account",
    "Start",
    "ElapsedRaw",
    // optional
    "End",
    "State",
    "ReqCPUS",
    "ReqMem",
    "Partition",
];

impl SacctExecutor for SacctExecutorImpl {
    // todo remove clippy ignore and call child.wait() to avoid zombie processes
    #[allow(clippy::zombie_processes)]
    fn sacct(&self, since: DateTime<Utc>) -> Result<BoxStream<Result<SacctData>>> {
        let since_str = since.format(SACCT_DATETIME_FORMAT).to_string();

        log::info!("Fetching sacct data since {}", since_str);

        // with --state we need an endtime
        let end_time_str = Utc::now().format(SACCT_DATETIME_FORMAT).to_string();

        let child = Command::new("sacct")
            .args([
                "-n", // no header
                "-a", // all users
                "-P", // parseable, adds "|" delimiter
                "-X", //remove step details (JobID.batch , .ext etc)
                "--format",
                SACCT_FIELDS.join(",").as_str(),
                "--state",
                "COMPLETED,FAILED,TIMEOUT,PREEMPTED,OUT_OF_MEMORY,CANCELLED", // TODO make configurable https://slurm.schedmd.com/sacct.html#SECTION_JOB-STATE-CODES
                "--starttime",
                &since_str,
                "--endtime",
                &end_time_str,
            ])
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn sacct command");

        let stdout = child.stdout.ok_or(AppError::SacctError(
            "Failed to open sacct stdout".to_string(),
        ))?;
        let reader = BufReader::new(stdout);

        Ok(futures::stream::iter(reader.lines().map(move |line| {
            let line = line.map_err(AppError::IoError)?;
            parse_sacct_line(&line)
        }))
        .boxed())
    }
}

async fn process_sacct_data<T: SacctExecutor>(
    config: &AppConfig,
    client: &mut GrpcClient,
    sacct_executor: &T,
) -> Result<()> {
    let checkpoint = load_checkpoint(&config.state_file, &config.initial_checkpoint)?;

    let mut sacct_data_stream = sacct_executor.sacct(checkpoint.last_processed_time)?;
    let mut batch = Vec::with_capacity(config.batch_size);

    while let Some(data) = sacct_data_stream.next().await {
        let data = data?;
        // Skip already processed jobs for the current timestamp
        if checkpoint.last_processed_time == data.start_time
            && checkpoint.processed_jobs.contains(&data.job_id)
        {
            continue;
        }

        batch.push(data.clone());

        if batch.len() >= config.batch_size {
            send_batch_to_api(client, &batch).await?;
            update_and_save_checkpoint(&batch, &config.state_file)?;

            batch.clear();
        }
    }

    // Process any remaining data
    if !batch.is_empty() {
        send_batch_to_api(client, &batch).await?;
        update_and_save_checkpoint(&batch, &config.state_file)?;
    }

    Ok(())
}

fn update_and_save_checkpoint(batch: &[SacctData], file_path: &str) -> Result<()> {
    if let Some(last_job) = batch.last() {
        let new_checkpoint = Checkpoint {
            last_processed_time: last_job.start_time,
            processed_jobs: batch
                .iter()
                .filter(|job| job.start_time == last_job.start_time)
                .map(|job| job.job_id.clone())
                .collect(),
        };

        let file = FsFile::create(file_path)?;
        serde_json::to_writer(file, &new_checkpoint)?;
    };

    Ok(())
}

fn load_checkpoint(file_path: &str, initial_checkpoint: &str) -> Result<Checkpoint> {
    match FsFile::open(file_path) {
        Ok(file) if file.metadata()?.len() > 0 => {
            let checkpoint: Checkpoint = serde_json::from_reader(file)?;
            Ok(checkpoint)
        }
        _ => Ok(Checkpoint {
            last_processed_time: DateTime::parse_from_rfc3339(initial_checkpoint)
                .map_err(AppError::DateParseError)?
                .with_timezone(&Utc),
            processed_jobs: Vec::new(),
        }),
    }
}

fn parse_sacct_line(line: &str) -> Result<SacctData> {
    let fields: Vec<&str> = line.split('|').collect();
    if fields.len() != SACCT_FIELDS.iter().len() {
        return Err(AppError::InvalidSacctOutput(
            "Invalid number of fields".to_string(),
        ));
    }

    let job_id = fields[0];
    let account = fields[1];
    let start_time = NaiveDateTime::parse_from_str(fields[2], SACCT_DATETIME_FORMAT)
        .map_err(AppError::DateParseError)?
        .and_utc();
    let elapsed_seconds = fields[3]
        .parse::<i64>()
        .map_err(|_| AppError::InvalidSacctOutput("Could not parse elapsed seconds".to_string()))?;
    let end_time = NaiveDateTime::parse_from_str(fields[4], SACCT_DATETIME_FORMAT)
        .map_err(AppError::DateParseError)?
        .and_utc();
    let state = fields[5];
    let req_cpus = fields[6]
        .parse::<i64>()
        .map_err(|_| AppError::InvalidSacctOutput("Could not parse req CPUs".to_string()))?;

    // we need to parse reqMem based on the last char, if it's not a number (e.g. M, G, T) we need to convert it to bytes
    let req_mem = parse_req_mem(fields[7]).ok_or(AppError::InvalidSacctOutput(
        "Could not parse req mem".to_string(),
    ))?;
    let partition = fields[8];

    Ok(SacctData {
        id: format!("{}_{}", fields[0], fields[2]),
        job_id: job_id.to_string(),
        account: account.to_string(),
        start_time,
        elapsed_seconds,
        state: state.to_string(),
        end_time,
        req_cpu: req_cpus,
        req_mem,
        partition: partition.to_string(),
    })
}

async fn send_batch_to_api(client: &mut GrpcClient, batch: &[SacctData]) -> Result<()> {
    info!("Sending batch of {} records to API", batch.len());

    let res = client.ingest(tonic::Request::new(metering_grpc::meteroid::metering::v1::IngestRequest {
        allow_backfilling: false,
        events: batch.iter().map(|data| {
            let mut properties = std::collections::HashMap::new();
            properties.insert("job_id".to_string(), data.job_id.to_string());
            properties.insert("state".to_string(), data.state.clone());
            properties.insert("req_cpus".to_string(), data.req_cpu.to_string());
            properties.insert("req_mem".to_string(), data.req_mem.to_string());
            properties.insert("elapsed_seconds".to_string(), data.elapsed_seconds.to_string());
            properties.insert("partition".to_string(), data.partition.to_string());

            metering_grpc::meteroid::metering::v1::Event {
                event_id: data.id.clone(),
                event_name: "slurm_job".to_string(),
                customer_id: Some(
                    metering_grpc::meteroid::metering::v1::event::CustomerId::ExternalCustomerAlias(data.account.clone())
                ),
                timestamp: data.start_time.to_rfc3339(),
                properties,
            }
        }).collect(),
    }))

        .await?
        .into_inner();

    if !res.failures.is_empty() {
        error!("Failed to process {} records.", res.failures.len());
        error!("Errors {:?}", res.failures);
        return Err(AppError::IngestError);
    }

    log::info!("Ingested successfully.");

    Ok(())
}

fn parse_req_mem(req_mem_raw: &str) -> Option<i64> {
    let trimmed = req_mem_raw.trim();

    if let Some(last_char) = trimmed.chars().last() {
        // If the last character is a letter, it indicates the unit
        if last_char.is_alphabetic() {
            let (num_part, unit) = trimmed.split_at(trimmed.len() - 1);
            let value: i64 = num_part.trim().parse().ok()?;
            let bytes = match unit.to_uppercase().as_str() {
                "B" => value,
                "K" => value * 1024,
                "M" => value * 1024 * 1024,
                "G" => value * 1024 * 1024 * 1024,
                "T" => value * 1024 * 1024 * 1024 * 1024,
                _ => return None, // Unknown unit
            };

            Some(bytes)
        } else {
            // If no unit is appended, we assume it's already in bytes
            let value: i64 = trimmed.parse().ok()?;
            Some(value)
        }
    } else {
        None // empty strings or invalid cases
    }
}

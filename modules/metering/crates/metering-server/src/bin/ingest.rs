//! Ingest-only metering process: the Kafka -> ClickHouse consumer plus the
//! EventsService / InternalEventsService write plane. Query is served elsewhere.
//! The default `metering-api` bin runs ingest + query together.

use envconfig::Envconfig;
use tokio::signal;

use common_build_info::BuildInfo;
use common_logging::telemetry;
use metering::config::Config;
use metering_server::server::{ExtraRoutes, ServerMode, start_server_with};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }?;

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {build_info}");

    let config = Config::init_from_env()?;

    telemetry::init(&config.common.telemetry);

    let server = start_server_with(config, ServerMode::Ingest, ExtraRoutes::default());

    let exit = signal::ctrl_c();

    tokio::select! {
        result = server => result?,
        _ = exit => {
            log::info!("Interrupted");
        }
    }

    Ok(())
}

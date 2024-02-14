use common_logging::init::init_telemetry;
use envconfig::Envconfig;
use metering::config::Config;
use tokio::signal;
use common_build_info::BuildInfo;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }?;

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {:?}", build_info);

    let config = Config::init_from_env().map_err(|err| err)?;

    init_telemetry(&config.common.telemetry, env!("CARGO_BIN_NAME"));

    // TODO clickhouse migrations

    let private_server = metering::server::start_api_server(config);

    let exit = signal::ctrl_c();

    tokio::select! {
          result = private_server => {
            if let Err(e) = result {
                log::error!("Error starting API server: {}", e);
            }
        },
        _ = exit => {
              log::info!("Interrupted");
        }
    };

    Ok(())
}

use std::sync::Arc;

use tokio::signal;

use common_build_info::BuildInfo;
use common_logging::init::init_telemetry;
use meteroid::adapters::stripe::Stripe;
use meteroid::config::Config;
use meteroid::singletons::get_pool;
use meteroid::{eventbus, webhook_in_api};
use meteroid_repository::migrations;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }?;

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {}", build_info);

    let config = Config::get();

    init_telemetry(&config.common.telemetry, env!("CARGO_BIN_NAME"));

    let pool = get_pool();

    // this creates a new pool, as it is incompatible with the one for cornucopia.
    let store = meteroid_store::Store::new(
        config.database_url.clone(),
        config.secrets_crypt_key.clone(),
        Arc::new(eventbus::memory::InMemory::new()),
    )?;

    let private_server =
        meteroid::api::server::start_api_server(config.clone(), pool.clone(), store.clone());

    let exit = signal::ctrl_c();

    let mut conn = meteroid::db::get_connection(&pool).await?;
    migrations::run_migrations(&mut **conn).await?;

    let object_store_client =
        Arc::new(object_store::parse_url(&url::Url::parse(&config.object_store_uri)?)?.0);

    let stripe_adapter = Arc::new(Stripe {
        client: stripe_client::client::StripeClient::new(),
    });

    tokio::select! {
        _ = private_server => {},
        _ = webhook_in_api::serve(
            config.invoicing_webhook_addr,
            object_store_client,
            pool.clone(),
            stripe_adapter.clone(),
            store,
        ) => {},
        _ = exit => {
              log::info!("Interrupted");
        }
    }

    Ok(())
}

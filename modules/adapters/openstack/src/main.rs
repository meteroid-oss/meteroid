use crate::config::Config;
use crate::events::EventHandler;
use dotenvy::dotenv;
use envconfig::Envconfig;

mod config;
mod error;
mod events;
mod sink;
mod source;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    env_logger::init();

    let config = Config::init_from_env()?;

    let mut event_handler = EventHandler {
        source: source::RabbitSource::connect(&config).await?,
        sink: sink::MeteroidSink::new(&config),
        config: config,
    };

    event_handler.start().await?;

    Ok(())
}

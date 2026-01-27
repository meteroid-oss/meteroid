use common_logging::tracing::level_filters::LevelFilter;
use common_logging::tracing_subscriber::EnvFilter;
use common_logging::tracing_subscriber::layer::SubscriberExt;
use common_logging::tracing_subscriber::util::SubscriberInitExt;
use std::sync::OnceLock;

static LOG_INIT: OnceLock<()> = OnceLock::new();

pub fn logging() {
    LOG_INIT.get_or_init(|| {
        // Test-specific logging configuration that filters out noisy modules
        let filter = EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy()
            // Filter out noisy test infrastructure logs
            .add_directive("meteroid::migrations=warn".parse().unwrap())
            .add_directive("tokio_postgres::connection=warn".parse().unwrap())
            .add_directive("meteroid::api_rest::server=warn".parse().unwrap())
            .add_directive("meteroid::api::server=warn".parse().unwrap())
            .add_directive("meteroid::eventbus=warn".parse().unwrap())
            .add_directive("integration::meteroid_it::container=warn".parse().unwrap());

        common_logging::tracing_subscriber::registry()
            .with(common_logging::logging::formatting_layer())
            .with(filter)
            .init();
    });
}

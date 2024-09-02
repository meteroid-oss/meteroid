mod archiver;
mod cleaner;
mod config;
mod error;
mod metrics;

pub use archiver::start_archiver;
pub use cleaner::start_cleaner;
pub use config::FangExtConfig;
use meteroid_store::store::PgPool;

pub fn start_tasks(pool: PgPool, config: &FangExtConfig) {
    if config.archiver.enabled {
        start_archiver(pool.clone(), config.archiver.clone());
    } else {
        log::warn!("Fang archiver is disabled");
    }

    if config.cleaner.enabled {
        start_cleaner(pool.clone(), config.cleaner.clone());
    } else {
        log::warn!("Fang cleaner is disabled");
    }
}

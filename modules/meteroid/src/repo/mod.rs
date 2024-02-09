use crate::config::Config;
use deadpool_postgres::Pool;
use std::sync::OnceLock;

pub mod errors;
pub mod provider_config;

static POOL: OnceLock<Pool> = OnceLock::new();

pub fn get_pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let config = Config::get();
        common_repository::create_pool(&config.database_url)
    })
}

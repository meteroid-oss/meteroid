use crate::config::Config;
use crate::eventbus;
use deadpool_postgres::Pool;
use meteroid_store::Store;
use std::sync::{Arc, OnceLock};

static POOL: OnceLock<Pool> = OnceLock::new();

pub fn get_pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let config = Config::get();
        common_repository::create_pool(&config.database_url)
    })
}

static STORE: OnceLock<Store> = OnceLock::new();

pub fn get_store() -> &'static Store {
    STORE.get_or_init(|| {
        let config = Config::get();
        Store::new(
            config.database_url.clone(),
            config.secrets_crypt_key.clone(),
            Arc::new(eventbus::memory::InMemory::new()),
        )
        .expect("Failed to initialize store")
    })
}

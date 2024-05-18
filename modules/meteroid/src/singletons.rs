use std::sync::OnceLock;

use deadpool_postgres::Pool;

use meteroid_store::Store;

use crate::config::Config;
use crate::eventbus::{create_eventbus_memory, setup_store_eventbus};

static POOL: OnceLock<Pool> = OnceLock::new();

pub fn get_pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let config = Config::get();
        common_repository::create_pool(&config.database_url)
    })
}

static STORE: tokio::sync::OnceCell<Store> = tokio::sync::OnceCell::const_new();
pub async fn get_store() -> &'static Store {
    STORE
        .get_or_init(|| async {
            let config = Config::get();

            let store = Store::new(
                config.database_url.clone(),
                config.secrets_crypt_key.clone(),
                config.jwt_secret.clone(),
                create_eventbus_memory(),
            )
            .expect("Failed to initialize store");

            setup_store_eventbus(store.clone(), config.clone()).await;

            store
        })
        .await
}

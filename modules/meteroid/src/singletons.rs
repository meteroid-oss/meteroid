use std::sync::{Arc, OnceLock};

use deadpool_postgres::Pool;

use crate::clients::usage::MeteringUsageClient;
use meteroid_store::Store;

use crate::config::Config;
use crate::eventbus::{create_eventbus_memory, setup_eventbus_handlers};

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
                Arc::new(MeteringUsageClient::get().clone()),
            )
            .expect("Failed to initialize store");

            setup_eventbus_handlers(store.clone(), config.clone()).await;

            store
        })
        .await
}

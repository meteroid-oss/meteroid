use std::sync::OnceLock;

use deadpool_postgres::Pool;

use meteroid_store::Store;

use crate::config::Config;
use crate::eventbus::EventBusStatic;

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
            let eventbus = EventBusStatic::get().await;
            Store::new(
                config.database_url.clone(),
                config.secrets_crypt_key.clone(),
                config.jwt_secret.clone(),
                eventbus.clone(),
            )
            .expect("Failed to initialize store")
        })
        .await
}

use crate::clients::usage::MeteringUsageClient;
use crate::config::Config;
use crate::eventbus::{create_eventbus_memory, setup_eventbus_handlers};
use crate::svix::new_svix;
use meteroid_store::store::StoreConfig;
use meteroid_store::Store;
use std::sync::Arc;

static STORE: tokio::sync::OnceCell<Store> = tokio::sync::OnceCell::const_new();

pub async fn get_store() -> &'static Store {
    STORE
        .get_or_init(|| async {
            let config = Config::get();

            let svix = new_svix(config);

            let store = Store::new(StoreConfig {
                database_url: config.database_url.clone(),
                crypt_key: config.secrets_crypt_key.clone(),
                jwt_secret: config.jwt_secret.clone(),
                multi_organization_enabled: config.multi_organization_enabled,
                eventbus: create_eventbus_memory(),
                usage_client: Arc::new(MeteringUsageClient::get().clone()),
                svix,
            })
            .expect("Failed to initialize store");

            setup_eventbus_handlers(store.clone(), config.clone()).await;

            store
        })
        .await
}

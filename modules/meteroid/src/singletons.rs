use meteroid_store::external::invoice_rendering::noop_invoice_rendering_service;
use std::sync::Arc;

use crate::clients::usage::MeteringUsageClient;
use meteroid_store::Store;

use crate::config::Config;
use crate::eventbus::{create_eventbus_memory, setup_eventbus_handlers};

static STORE: tokio::sync::OnceCell<Store> = tokio::sync::OnceCell::const_new();

pub async fn get_store() -> &'static Store {
    STORE
        .get_or_init(|| async {
            let config = Config::get();

            let store = Store::new(
                config.database_url.clone(),
                config.secrets_crypt_key.clone(),
                config.jwt_secret.clone(),
                config.multi_organization_enabled,
                create_eventbus_memory(),
                Arc::new(MeteringUsageClient::get().clone()),
                noop_invoice_rendering_service(),
            )
            .expect("Failed to initialize store");

            setup_eventbus_handlers(store.clone(), config.clone()).await;

            store
        })
        .await
}

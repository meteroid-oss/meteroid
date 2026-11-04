use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::wallets::v1::wallets_service_server::WalletsServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod activity;
pub use meteroid_app::ledger_export;
pub mod mapping;
mod service;

pub use meteroid_grpc_mapping::wallets::WalletApiError;
pub(crate) use service::{grant_product_names, top_up_status};

pub struct WalletServiceComponents {
    pub store: Store,
    pub services: Services,
    /// Signs the CSV export links.
    pub jwt_secret: SecretString,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
    feature_resolver: Arc<dyn Entitlements>,
) -> WalletsServiceServer<WalletServiceComponents> {
    WalletsServiceServer::new(WalletServiceComponents {
        store,
        services,
        jwt_secret,
        feature_resolver,
    })
}

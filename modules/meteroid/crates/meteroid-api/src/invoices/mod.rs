use meteroid_app::features::Entitlements;
use meteroid_app::services::invoice_rendering::InvoicePreviewRenderingService;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

pub use meteroid_grpc_mapping::invoices::{error, mapping};
mod service;

pub struct InvoiceServiceComponents {
    pub store: Store,
    pub services: Services,
    pub preview_rendering: InvoicePreviewRenderingService,
    pub jwt_secret: SecretString,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
    preview_rendering: InvoicePreviewRenderingService,
    feature_resolver: Arc<dyn Entitlements>,
) -> InvoicesServiceServer<InvoiceServiceComponents> {
    let inner = InvoiceServiceComponents {
        store,
        services,
        preview_rendering,
        jwt_secret,
        feature_resolver,
    };

    InvoicesServiceServer::new(inner)
}

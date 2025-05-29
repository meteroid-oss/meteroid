use crate::services::invoice_rendering::InvoicePreviewRenderingService;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

mod error;
pub mod mapping;
mod service;

pub struct InvoiceServiceComponents {
    pub store: Store,
    pub services: Services,
    pub preview_rendering: InvoicePreviewRenderingService,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
    preview_rendering: InvoicePreviewRenderingService,
) -> InvoicesServiceServer<InvoiceServiceComponents> {
    let inner = InvoiceServiceComponents {
        store,
        services,
        preview_rendering,
        jwt_secret,
    };

    InvoicesServiceServer::new(inner)
}

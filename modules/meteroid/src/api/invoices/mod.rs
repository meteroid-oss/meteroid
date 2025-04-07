use crate::services::invoice_rendering::InvoicePreviewRenderingService;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;

mod error;
pub mod mapping;
mod service;

pub struct InvoiceServiceComponents {
    pub store: Store,
    pub preview_rendering: InvoicePreviewRenderingService,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    jwt_secret: SecretString,
    preview_rendering: InvoicePreviewRenderingService,
) -> InvoicesServiceServer<InvoiceServiceComponents> {
    let inner = InvoiceServiceComponents {
        store,
        preview_rendering,
        jwt_secret,
    };

    InvoicesServiceServer::new(inner)
}

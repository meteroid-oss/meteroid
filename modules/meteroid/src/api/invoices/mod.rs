use crate::services::invoice_rendering::HtmlRenderingService;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;

mod error;
pub mod mapping;
mod service;

pub struct InvoiceServiceComponents {
    pub store: Store,
    pub html_rendering: HtmlRenderingService,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    jwt_secret: SecretString,
) -> InvoicesServiceServer<InvoiceServiceComponents> {
    let html_rendering = HtmlRenderingService::new(Arc::new(store.clone()));

    let inner = InvoiceServiceComponents {
        store,
        html_rendering,
        jwt_secret,
    };

    InvoicesServiceServer::new(inner)
}

use std::sync::Arc;

use common_domain::ids::StoredDocumentId;
use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::quotes::v1::quotes_service_server::QuotesServiceServer;
use meteroid_store::hooks::QuoteDocuments;
use meteroid_store::{Services, Store};

mod error;
pub mod mapping;
mod service;
mod v2;

/// Links to a quote's files through the REST file routes, which every storage backend serves.
#[derive(Clone)]
pub struct QuoteFileLinks {
    pub rest_api_external_url: String,
}

impl QuoteFileLinks {
    pub fn logo(&self, id: StoredDocumentId) -> String {
        format!("{}/files/v1/logo/{}", self.rest_api_external_url, id)
    }
}

pub struct QuoteServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
    pub quote_documents: Arc<dyn QuoteDocuments>,
    pub links: QuoteFileLinks,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
    quote_documents: Arc<dyn QuoteDocuments>,
    links: QuoteFileLinks,
) -> QuotesServiceServer<QuoteServiceComponents> {
    let inner = QuoteServiceComponents {
        store,
        services,
        feature_resolver,
        quote_documents,
        links,
    };

    QuotesServiceServer::new(inner)
}

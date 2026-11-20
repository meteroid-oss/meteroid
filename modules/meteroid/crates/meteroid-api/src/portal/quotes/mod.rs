use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_server::PortalQuoteServiceServer;
use meteroid_store::hooks::QuoteDocuments;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod service;
pub(crate) use service::{origin_of, quote_portal_details};

pub struct PortalQuoteServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
    pub quote_documents: Arc<dyn QuoteDocuments>,
    pub links: crate::quotes::QuoteFileLinks,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
    quote_documents: Arc<dyn QuoteDocuments>,
    links: crate::quotes::QuoteFileLinks,
) -> PortalQuoteServiceServer<PortalQuoteServiceComponents> {
    let inner = PortalQuoteServiceComponents {
        store,
        services,
        feature_resolver,
        quote_documents,
        links,
    };
    PortalQuoteServiceServer::new(inner)
}

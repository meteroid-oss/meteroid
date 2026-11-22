use std::sync::Arc;

use common_domain::ids::{QuoteAttachmentId, QuoteId, StoredDocumentId, TenantId};
use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::quotes::v1::Quote;
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

/// Fills the download links of the quote's attachments, which the mapping leaves unset.
pub(crate) fn link_attachments(
    documents: &dyn QuoteDocuments,
    tenant_id: TenantId,
    quote_id: QuoteId,
    quote: &mut Quote,
) -> Result<(), tonic::Status> {
    for attachment in &mut quote.attachments {
        let id = QuoteAttachmentId::from_proto(&attachment.id)?;
        attachment.download_url = documents
            .attachment_url(tenant_id, quote_id, id)
            .map_err(|e| tonic::Status::internal(format!("{e:?}")))?;
    }
    Ok(())
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

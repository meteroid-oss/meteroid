use crate::services::credit_note_rendering::CreditNotePreviewRenderingService;
use meteroid_grpc::meteroid::api::creditnotes::v1::credit_notes_service_server::CreditNotesServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;

mod error;
pub mod mapping;
mod service;

pub struct CreditNoteServiceComponents {
    pub store: Store,
    pub preview_rendering: CreditNotePreviewRenderingService,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    preview_rendering: CreditNotePreviewRenderingService,
    jwt_secret: SecretString,
) -> CreditNotesServiceServer<CreditNoteServiceComponents> {
    let inner = CreditNoteServiceComponents {
        store,
        preview_rendering,
        jwt_secret,
    };

    CreditNotesServiceServer::new(inner)
}

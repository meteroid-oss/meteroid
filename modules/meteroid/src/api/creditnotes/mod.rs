use crate::services::credit_note_rendering::CreditNotePreviewRenderingService;
use meteroid_grpc::meteroid::api::creditnotes::v1::credit_notes_service_server::CreditNotesServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct CreditNoteServiceComponents {
    pub store: Store,
    pub preview_rendering: CreditNotePreviewRenderingService,
}

pub fn service(
    store: Store,
    preview_rendering: CreditNotePreviewRenderingService,
) -> CreditNotesServiceServer<CreditNoteServiceComponents> {
    let inner = CreditNoteServiceComponents {
        store,
        preview_rendering,
    };

    CreditNotesServiceServer::new(inner)
}

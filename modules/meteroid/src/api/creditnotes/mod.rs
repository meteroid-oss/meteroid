use meteroid_grpc::meteroid::api::creditnotes::v1::credit_notes_service_server::CreditNotesServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct CreditNoteServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> CreditNotesServiceServer<CreditNoteServiceComponents> {
    let inner = CreditNoteServiceComponents { store };

    CreditNotesServiceServer::new(inner)
}

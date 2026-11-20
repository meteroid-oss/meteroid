use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_app::errors::ObjectStoreError;
use meteroid_store::errors::StoreError;
use std::error::Error;
use thiserror::Error;

#[derive(Error, Debug, ErrorAsTonic)]
pub enum PortalQuoteApiError {
    #[error("{}", crate::portal::INTERNAL_ERROR)]
    #[code(Internal)]
    StoreError(#[source] Box<dyn Error>),

    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("{0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),

    #[error("Quote not found")]
    #[code(NotFound)]
    QuoteNotFound,

    #[error("Recipient not found")]
    #[code(NotFound)]
    RecipientNotFound,

    #[error("Quote already signed by this recipient")]
    #[code(AlreadyExists)]
    AlreadySigned,

    #[error("Quote expired")]
    #[code(FailedPrecondition)]
    QuoteExpired,

    #[error("Quote not in signable state")]
    #[code(FailedPrecondition)]
    NotSignable,

    #[error("Quote not in editable state")]
    #[code(FailedPrecondition)]
    NotEditable,

    #[error("This quote is being revised: you will receive the new version")]
    #[code(FailedPrecondition)]
    BeingRevised,

    #[error("{0}")]
    #[code(FailedPrecondition)]
    FailedPrecondition(String),

    #[error("{}", crate::portal::INTERNAL_ERROR)]
    #[code(Internal)]
    ObjectStoreError(#[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for PortalQuoteApiError {
    fn from(err: Report<StoreError>) -> Self {
        match err.current_context() {
            StoreError::InvalidArgument(msg) => PortalQuoteApiError::InvalidArgument(msg.clone()),
            StoreError::FailedPrecondition(msg) => {
                PortalQuoteApiError::FailedPrecondition(msg.clone())
            }
            _ => PortalQuoteApiError::StoreError(Box::new(err.into_error())),
        }
    }
}

impl From<Report<ObjectStoreError>> for PortalQuoteApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(err)
    }
}

use crate::errors::ObjectStoreError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use error_stack::Report;
use meteroid_store::errors::StoreError;
use std::error::Error;
use thiserror::Error;

#[derive(Error, Debug, ErrorAsTonic)]
pub enum PortalQuoteApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String),

    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),

    #[error("Missing argument: {0}")]
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

    #[error("Object store error: {0}")]
    #[code(Internal)]
    ObjectStoreError(String, #[source] Box<dyn Error>),
}

impl From<Report<StoreError>> for PortalQuoteApiError {
    fn from(err: Report<StoreError>) -> Self {
        PortalQuoteApiError::StoreError(err.to_string())
    }
}

impl From<Report<ObjectStoreError>> for PortalQuoteApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(
            "Object store error in portal checkout service".to_string(),
            err,
        )
    }
}

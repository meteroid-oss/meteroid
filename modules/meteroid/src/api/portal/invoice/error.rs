use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use crate::errors::ObjectStoreError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::adapters::payment_service_providers::PaymentProviderError;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalInvoiceApiError {
    #[error("Store error: {0}")]
    #[code(Internal)]
    StoreError(String, #[source] Box<dyn Error>),
    #[error("Object store error: {0}")]
    #[code(Internal)]
    ObjectStoreError(String, #[source] Box<dyn Error>),
    #[error("Subscription has no configured payment provider")]
    #[code(InvalidArgument)]
    MissingCustomerConnection,
    #[error("Failed to update customer")]
    #[code(Internal)]
    CustomerUpdateError,
    #[error("Missing argument: {0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),
    #[error("Invalid argument: {0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
    #[error("{0}")]
    #[code(Internal)]
    InternalError(String),
}

impl From<Report<StoreError>> for PortalInvoiceApiError {
    fn from(value: Report<StoreError>) -> Self {
        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::PaymentError(msg) => Self::InternalError(msg.clone()),
            StoreError::PaymentProviderError => {
                let provider_error = value
                    .frames()
                    .find_map(|f| f.downcast_ref::<PaymentProviderError>());
                match provider_error {
                    Some(e) => Self::InternalError(e.to_string()),
                    None => Self::InternalError(
                        "The payment provider rejected this action. Please contact support."
                            .to_string(),
                    ),
                }
            }
            StoreError::DuplicateValue { entity, key } => {
                let msg = match key {
                    Some(k) => format!("{} with key '{}' already exists", entity, k),
                    None => format!("{} already exists", entity),
                };
                Self::InvalidArgument(msg)
            }
            _ => Self::StoreError(
                "Error in invoice service".to_string(),
                Box::new(value.into_error()),
            ),
        }
    }
}

impl From<Report<ObjectStoreError>> for PortalInvoiceApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(
            "Object store error in portal invoice service".to_string(),
            err,
        )
    }
}

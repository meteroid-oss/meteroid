use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_app::errors::ObjectStoreError;
use meteroid_store::adapters::payment::error::CustomerFacingMessage;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalSharedApiError {
    #[error("{}", crate::portal::INTERNAL_ERROR)]
    #[code(Internal)]
    StoreError(#[source] Box<dyn Error>),
    #[error("{}", crate::portal::INTERNAL_ERROR)]
    #[code(Internal)]
    ObjectStoreError(#[source] Box<dyn Error>),
    #[error("Subscription has no configured payment provider")]
    #[code(InvalidArgument)]
    MissingCustomerConnection,
    #[error("Failed to update customer")]
    #[code(Internal)]
    CustomerUpdateError,
    #[error("Error occurred while interacting with the payment provider")]
    #[code(Internal)]
    PaymentProviderError,
    #[error("{0}")]
    #[code(InvalidArgument)]
    MissingArgument(String),
    #[error("{0}")]
    #[code(InvalidArgument)]
    InvalidArgument(String),
    #[error("{0}")]
    #[code(FailedPrecondition)]
    PaymentFailed(String),
    #[error("{0}")]
    #[code(FailedPrecondition)]
    PaymentUnavailable(String),
}

impl From<Report<StoreError>> for PortalSharedApiError {
    fn from(value: Report<StoreError>) -> Self {
        // Customer-facing provider message (Mollie), shown as is.
        if let Some(CustomerFacingMessage(msg)) = value
            .frames()
            .find_map(|f| f.downcast_ref::<CustomerFacingMessage>())
        {
            return Self::PaymentUnavailable(msg.clone());
        }

        // The merchant disabled or deleted the provider; don't name it to the customer.
        if matches!(value.current_context(), StoreError::ConnectorNotActive(_)) {
            return Self::PaymentUnavailable(
                "Online payment isn't available right now. Please contact the merchant."
                    .to_string(),
            );
        }

        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::PaymentError(msg) => {
                Self::PaymentFailed(crate::portal::customer_payment_message(msg))
            }
            // The provider's own text names the account or the SDK: it stays in the logs.
            StoreError::PaymentProviderError => Self::StoreError(Box::new(value.into_error())),
            StoreError::DuplicateValue { entity, key } => {
                let msg = match key {
                    Some(k) => format!("{} with key '{}' already exists", entity, k),
                    None => format!("{} already exists", entity),
                };
                Self::InvalidArgument(msg)
            }
            _ => Self::StoreError(Box::new(value.into_error())),
        }
    }
}

impl From<Report<ObjectStoreError>> for PortalSharedApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(err)
    }
}

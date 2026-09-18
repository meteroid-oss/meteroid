use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use crate::errors::ObjectStoreError;
use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_store::adapters::payment::ConnectorError;
use meteroid_store::adapters::payment::error::CustomerFacingMessage;
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
    #[error("{0}")]
    #[code(FailedPrecondition)]
    PaymentUnavailable(String),
}

impl From<Report<StoreError>> for PortalInvoiceApiError {
    fn from(value: Report<StoreError>) -> Self {
        // Customer-facing provider message (Mollie), shown as is.
        if let Some(CustomerFacingMessage(msg)) = value
            .frames()
            .find_map(|f| f.downcast_ref::<CustomerFacingMessage>())
        {
            return Self::PaymentUnavailable(msg.clone());
        }

        let err = value.current_context();

        match err {
            StoreError::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::ValueNotFound(msg) => Self::InvalidArgument(msg.clone()),
            StoreError::PaymentError(msg) => Self::InternalError(msg.clone()),
            StoreError::PaymentProviderError => {
                let provider_error = value
                    .frames()
                    .find_map(|f| f.downcast_ref::<ConnectorError>());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_report(err: Report<ConnectorError>) -> Report<StoreError> {
        err.change_context(StoreError::PaymentProviderError)
    }

    #[test]
    fn customer_facing_provider_message_is_shown_verbatim() {
        let msg = "Direct debit is not available right now. Please pay by card.";
        let marked = provider_report(
            Report::new(ConnectorError::Configuration(msg.to_string()))
                .attach_opaque(CustomerFacingMessage(msg.to_string())),
        );
        match PortalInvoiceApiError::from(marked) {
            PortalInvoiceApiError::PaymentUnavailable(m) => assert_eq!(m, msg),
            other => panic!("expected PaymentUnavailable, got {other:?}"),
        }

        let unmarked = provider_report(Report::new(ConnectorError::Configuration(
            "stripe sdk: boom".to_string(),
        )));
        match PortalInvoiceApiError::from(unmarked) {
            PortalInvoiceApiError::InternalError(m) => {
                assert_eq!(m, "Connector configuration error: stripe sdk: boom")
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }
}

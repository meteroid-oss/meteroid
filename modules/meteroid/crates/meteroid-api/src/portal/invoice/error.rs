use error_stack::Report;
use std::error::Error;
use thiserror::Error;

use common_grpc_error_as_tonic_macros_impl::ErrorAsTonic;
use meteroid_app::errors::ObjectStoreError;
use meteroid_store::adapters::payment::error::CustomerFacingMessage;
use meteroid_store::errors::StoreError;

#[derive(Debug, Error, ErrorAsTonic)]
pub enum PortalInvoiceApiError {
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

impl From<Report<StoreError>> for PortalInvoiceApiError {
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

impl From<Report<ObjectStoreError>> for PortalInvoiceApiError {
    fn from(value: Report<ObjectStoreError>) -> Self {
        let err = Box::new(value.into_error());
        Self::ObjectStoreError(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meteroid_store::adapters::payment::ConnectorError;

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
        let status = tonic::Status::from(PortalInvoiceApiError::from(unmarked));
        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), crate::portal::INTERNAL_ERROR);
    }

    #[test]
    fn a_decline_is_a_customer_safe_precondition() {
        let declined = Report::new(StoreError::PaymentError(
            "Test gateway: payment declined (insufficient_funds)".to_string(),
        ));
        let status = tonic::Status::from(PortalInvoiceApiError::from(declined));
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status.message(),
            "Your card was declined for insufficient funds. Please try again or use a different payment method."
        );
        let invalid = Report::new(StoreError::InvalidArgument(
            "Coupon code 'X' not found".into(),
        ));
        assert_eq!(
            tonic::Status::from(PortalInvoiceApiError::from(invalid)).message(),
            "Coupon code 'X' not found"
        );
    }
}

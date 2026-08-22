use super::connector::PaymentConnector;
use super::error::ConnectorError;
use super::gocardless::GoCardlessConnector;
use super::mock::MockConnector;
use super::stripe::StripeConnector;
use crate::domain::connectors::Connector;
use crate::domain::enums::ConnectorProviderEnum;
use error_stack::Report;

/// Each call returns a freshly-boxed adapter, but the underlying HTTP client is
/// a process-wide singleton, so connection pooling is preserved.
pub fn initialize_payment_connector(
    config: &Connector,
) -> Result<Box<dyn PaymentConnector>, Report<ConnectorError>> {
    match config.provider {
        ConnectorProviderEnum::Stripe => Ok(Box::new(StripeConnector::new())),
        ConnectorProviderEnum::Gocardless => Ok(Box::new(GoCardlessConnector::new())),
        ConnectorProviderEnum::Mock => Ok(Box::new(MockConnector::from_connector(config))),
        ConnectorProviderEnum::Hubspot
        | ConnectorProviderEnum::Pennylane
        | ConnectorProviderEnum::Kintsugi => Err(Report::new(ConnectorError::Unsupported {
            provider: config.provider.clone(),
            capability: "payment operations",
        })),
    }
}

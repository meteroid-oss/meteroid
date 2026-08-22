use super::connector::{ConnectorCapabilities, PaymentConnector};
use super::error::ConnectorError;
use super::gocardless::GoCardlessConnector;
use super::mock::MockConnector;
use super::stancer::StancerConnector;
use super::stripe::StripeConnector;
use crate::domain::connectors::Connector;
use crate::domain::enums::ConnectorProviderEnum;
use error_stack::Report;

/// Static capability matrix for a payment provider, without needing a
/// configured [`Connector`]. `None` for non-payment providers.
pub fn provider_capabilities(
    provider: &ConnectorProviderEnum,
) -> Option<&'static ConnectorCapabilities> {
    match provider {
        ConnectorProviderEnum::Stripe => Some(&super::stripe::STRIPE_CAPABILITIES),
        ConnectorProviderEnum::Gocardless => Some(&super::gocardless::GOCARDLESS_CAPABILITIES),
        ConnectorProviderEnum::Stancer => Some(&super::stancer::STANCER_CAPABILITIES),
        ConnectorProviderEnum::Mock => Some(&super::mock::MOCK_CAPABILITIES),
        ConnectorProviderEnum::Hubspot | ConnectorProviderEnum::Pennylane => None,
    }
}

/// Each call returns a freshly-boxed adapter, but the underlying HTTP client is
/// a process-wide singleton, so connection pooling is preserved.
pub fn initialize_payment_connector(
    config: &Connector,
) -> Result<Box<dyn PaymentConnector>, Report<ConnectorError>> {
    match config.provider {
        ConnectorProviderEnum::Stripe => Ok(Box::new(StripeConnector::new())),
        ConnectorProviderEnum::Gocardless => Ok(Box::new(GoCardlessConnector::new())),
        ConnectorProviderEnum::Stancer => Ok(Box::new(StancerConnector::new())),
        ConnectorProviderEnum::Mock => Ok(Box::new(MockConnector::from_connector(config))),
        ConnectorProviderEnum::Hubspot | ConnectorProviderEnum::Pennylane => {
            Err(Report::new(ConnectorError::Unsupported {
                provider: config.provider.clone(),
                capability: "payment operations",
            }))
        }
    }
}

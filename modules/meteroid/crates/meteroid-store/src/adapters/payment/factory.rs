use super::connector::{ConnectorCapabilities, PaymentConnector};
use super::error::ConnectorError;
use super::gocardless::GoCardlessConnector;
use super::mock::MockConnector;
use super::mollie::MollieConnector;
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
        ConnectorProviderEnum::Mollie => Some(&super::mollie::MOLLIE_CAPABILITIES),
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
        ConnectorProviderEnum::Mollie => Ok(Box::new(MollieConnector::new())),
        ConnectorProviderEnum::Mock => Ok(Box::new(MockConnector::from_connector(config))),
        ConnectorProviderEnum::Hubspot | ConnectorProviderEnum::Pennylane => {
            Err(Report::new(ConnectorError::Unsupported {
                provider: config.provider.clone(),
                capability: "payment operations",
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::provider_capabilities;
    use crate::adapters::payment::contract::assert_capabilities_consistent;
    use crate::domain::enums::ConnectorProviderEnum;

    /// Exhaustive: a new variant won't compile until it is classified here.
    fn every_provider() -> Vec<ConnectorProviderEnum> {
        use ConnectorProviderEnum::*;
        [
            Stripe, Hubspot, Pennylane, Gocardless, Stancer, Mollie, Mock,
        ]
        .into_iter()
        .filter(|p| match p {
            Stripe | Gocardless | Stancer | Mollie | Mock => true,
            Hubspot | Pennylane => false,
        })
        .collect()
    }

    /// Pins the per-provider capability matrix so a mistake in one adapter fails here.
    #[test]
    fn provider_capability_matrix_is_pinned() {
        use crate::adapters::payment::HostedSetupCompletion::*;
        use ConnectorProviderEnum::*;
        let row = |provider: ConnectorProviderEnum| {
            let c = provider_capabilities(&provider).unwrap();
            (
                c.pending_charge_accepted,
                c.supports_hosted_checkout,
                c.supports_hosted_invoice_payment,
                c.is_hosted_redirect(),
                c.hosted_setup_completion,
                c.completes_pending_hosted_intents(),
            )
        };
        assert_eq!(
            row(Stripe),
            (false, false, false, false, WebhookBacked, false)
        );
        assert_eq!(
            row(Gocardless),
            (true, true, false, true, WebhookBacked, false)
        );
        assert_eq!(
            row(Stancer),
            (true, true, true, true, PollingRequired, true)
        );
        assert_eq!(row(Mollie), (true, true, true, true, WebhookBacked, true));
        assert_eq!(row(Mock), (false, true, false, false, WebhookBacked, false));
    }

    #[test]
    fn every_payment_provider_declares_consistent_capabilities() {
        for provider in every_provider() {
            let caps = provider_capabilities(&provider).unwrap_or_else(|| {
                panic!("{provider:?} is a payment provider without capabilities")
            });
            assert_capabilities_consistent(caps);
        }
    }
}

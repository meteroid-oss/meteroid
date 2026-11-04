use super::connector::{ConnectorRef, PaymentCapabilities, PaymentConnector};
use super::error::ConnectorError;
use super::gocardless::GoCardlessConnector;
use super::mock::MockConnector;
use super::mollie::MollieConnector;
use super::stancer::StancerConnector;
use super::stripe::StripeConnector;
use crate::domain::connectors::{Connector, PaymentProviderConfig};
use crate::domain::enums::{ConnectorProviderEnum, PaymentProvider};
use error_stack::Report;

/// Static capability matrix for a payment provider, without needing a
/// configured [`Connector`].
pub fn provider_capabilities(provider: PaymentProvider) -> &'static PaymentCapabilities {
    match provider {
        PaymentProvider::Stripe => &super::stripe::STRIPE_CAPABILITIES,
        PaymentProvider::Gocardless => &super::gocardless::GOCARDLESS_CAPABILITIES,
        PaymentProvider::Stancer => &super::stancer::STANCER_CAPABILITIES,
        PaymentProvider::Mollie => &super::mollie::MOLLIE_CAPABILITIES,
        PaymentProvider::Mock => &super::mock::MOCK_CAPABILITIES,
    }
}

/// Like [`provider_capabilities`], for a provider that may take no payments at all.
pub fn payment_capabilities(
    provider: &ConnectorProviderEnum,
) -> Option<&'static PaymentCapabilities> {
    provider.as_payment().map(provider_capabilities)
}

/// Binds an adapter to one connector: its identity and its typed credentials are read once, here,
/// so no later call can be handed the wrong provider's data. Each call returns a freshly-boxed
/// adapter, but the underlying HTTP client is a process-wide singleton, so connection pooling is
/// preserved.
pub fn initialize_payment_connector(
    connector: &Connector,
) -> Result<Box<dyn PaymentConnector>, Report<ConnectorError>> {
    let Some(payment) = connector.config.payment() else {
        return Err(Report::new(ConnectorError::Unsupported {
            provider: connector.provider(),
            capability: "payment operations",
        }));
    };
    let reference = ConnectorRef::from(connector);
    let config = payment.credentials().cloned().ok_or_else(|| {
        ConnectorError::Configuration(format!("connector {} has no credentials", connector.id))
    })?;
    Ok(match config {
        PaymentProviderConfig::Stripe(public, sensitive) => {
            Box::new(StripeConnector::new(reference, public, sensitive))
        }
        PaymentProviderConfig::Gocardless(public, sensitive) => {
            Box::new(GoCardlessConnector::new(reference, public, sensitive))
        }
        PaymentProviderConfig::Stancer(sensitive) => {
            Box::new(StancerConnector::new(reference, sensitive))
        }
        PaymentProviderConfig::Mollie(sensitive) => {
            Box::new(MollieConnector::new(reference, sensitive))
        }
        PaymentProviderConfig::Mock(public, _) => Box::new(MockConnector::new(reference, public)),
    })
}

#[cfg(test)]
mod tests {
    use super::provider_capabilities;
    use crate::adapters::payment::contract::assert_capabilities_consistent;
    use crate::domain::enums::PaymentProvider;
    use strum::IntoEnumIterator;

    /// Pins the per-provider capability matrix so a mistake in one adapter fails here.
    #[test]
    fn provider_capability_matrix_is_pinned() {
        use crate::adapters::payment::HostedSetupCompletion::*;
        use PaymentProvider::*;
        let row = |provider: PaymentProvider| {
            let c = provider_capabilities(provider);
            (
                c.pending_charge_accepted,
                c.supports_hosted_checkout,
                c.supports_hosted_invoice_payment(),
                c.is_hosted_redirect(),
                c.hosted_setup_completion(),
                c.completes_pending_hosted_intents(),
            )
        };
        // Stripe finishes setup client-side: no hosted redirect, so no hosted completion.
        assert_eq!(row(Stripe), (false, false, false, false, None, false));
        assert_eq!(
            row(Gocardless),
            (true, true, false, true, Some(WebhookBacked), false)
        );
        assert_eq!(
            row(Stancer),
            (true, true, true, true, Some(PollingRequired), true)
        );
        assert_eq!(
            row(Mollie),
            (true, true, true, true, Some(WebhookBacked), true)
        );
        assert_eq!(row(Mock), (false, true, false, false, None, false));
        assert_eq!(
            PaymentProvider::iter().count(),
            5,
            "a new payment provider needs its own row here"
        );
    }

    #[test]
    fn only_signed_webhook_providers_require_a_secret() {
        use PaymentProvider::*;
        let requires = |p| provider_capabilities(p).requires_webhook_secret();
        assert!(requires(Stripe));
        assert!(requires(Gocardless));
        assert!(
            !requires(Mollie),
            "Mollie authenticates with a generated URL token"
        );
        assert!(!requires(Stancer), "Stancer sends no webhooks");
        assert!(!requires(Mock));
    }

    #[test]
    fn every_payment_provider_declares_consistent_capabilities() {
        for provider in PaymentProvider::iter() {
            assert_capabilities_consistent(provider_capabilities(provider));
        }
    }
}

//! What a connector can do right now: its lifecycle status and the payment methods the
//! provider account has enabled, on top of the adapter's static [`PaymentCapabilities`].

use super::factory::payment_capabilities;
use crate::StoreResult;
use crate::domain::connectors::{
    Connector, MethodState, PaymentConnectorPublicData, PaymentMethodAvailability, ProviderAccount,
};
use crate::domain::enums::{ConnectorStatusEnum, PaymentMethodTypeEnum};
use crate::errors::StoreError;

/// Result of a successful credential check.
#[derive(Debug, Clone)]
pub struct ValidatedCredentials {
    /// Public data to persist (e.g. the Stripe account id).
    pub public: PaymentConnectorPublicData,
    pub account: ProviderAccount,
}

pub fn is_direct_debit(method: &PaymentMethodTypeEnum) -> bool {
    matches!(
        method,
        PaymentMethodTypeEnum::DirectDebitSepa
            | PaymentMethodTypeEnum::DirectDebitAch
            | PaymentMethodTypeEnum::DirectDebitBacs
    )
}

/// Key prefixes that name the kind of key, never part of the secret itself.
const KEY_PREFIXES: &[&str] = &[
    // Stripe
    "sk_live_", "sk_test_", "rk_live_", "rk_test_", // Stancer
    "sprod_", "stest_",   // GoCardless
    "sandbox_", // Mollie, GoCardless
    "live_", "test_",
];

/// `sk_live_abcd…wxyz` → `sk_live_…wxyz`: the known prefix says which kind of key, the last four
/// characters which one. An unknown shape shows only the tail, so no part of the secret body
/// ever passes for a prefix.
pub fn mask_secret(secret: &str) -> String {
    let secret = secret.trim();
    let prefix = KEY_PREFIXES
        .iter()
        .filter(|p| secret.starts_with(**p) && secret.len() > p.len() + 4)
        .max_by_key(|p| p.len())
        .copied()
        .unwrap_or("");
    let tail: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}…{tail}")
}

/// Whether the provider's capabilities require a webhook signing secret before going live.
pub fn requires_webhook_secret(provider: crate::domain::enums::PaymentProvider) -> bool {
    super::factory::provider_capabilities(provider).requires_webhook_secret()
}

/// Every rail the adapter supports, in the given state. For providers whose account can't be
/// queried for enabled methods.
pub fn static_methods(
    methods: &[PaymentMethodTypeEnum],
    state: MethodState,
) -> Vec<PaymentMethodAvailability> {
    methods
        .iter()
        .map(|method| PaymentMethodAvailability {
            method: method.clone(),
            state,
            reason: None,
        })
        .collect()
}

impl Connector {
    pub fn is_active(&self) -> bool {
        self.status == ConnectorStatusEnum::Active
    }

    /// Guards every place that starts something new (charge, setup, checkout). Webhooks,
    /// reconciliation and refunds deliberately don't call it.
    pub fn ensure_active(&self) -> StoreResult<()> {
        let state = match self.status {
            ConnectorStatusEnum::Active => return Ok(()),
            ConnectorStatusEnum::Draft => "isn't set up yet",
            ConnectorStatusEnum::Disabled => "is disabled",
            ConnectorStatusEnum::Deleted => "was deleted",
        };
        Err(error_stack::Report::new(StoreError::ConnectorNotActive(
            format!("The payment connection '{}' {state}", self.alias),
        )))
    }

    /// [`Self::ensure_active`] on a freshly locked row, for paths that start a payment inside a
    /// transaction: a concurrent delete or disable waits for that transaction to commit.
    pub async fn ensure_active_locked(&self, conn: &mut crate::store::PgConn) -> StoreResult<()> {
        let row = diesel_models::connectors::ConnectorRow::lock_key_shared_by_id(
            conn,
            self.id,
            self.tenant_id,
        )
        .await
        .map_err(|e| StoreError::DatabaseError(e.error))?;
        Connector {
            status: row.status.into(),
            ..self.clone()
        }
        .ensure_active()
    }

    /// The adapter must support the method and, once the account is known, the account must have
    /// it enabled. Connectors connected before account detection count their static methods as
    /// enabled until the first refresh.
    pub fn method_state(&self, method: &PaymentMethodTypeEnum) -> MethodState {
        let supported = payment_capabilities(&self.provider())
            .is_some_and(|caps| caps.supported_payment_methods.contains(method));
        if !supported {
            return MethodState::Unavailable;
        }
        match self.account() {
            None => MethodState::Enabled,
            Some(account) => account
                .methods
                .iter()
                .find(|m| &m.method == method)
                .map(|m| m.state)
                .unwrap_or(MethodState::Unavailable),
        }
    }

    pub fn is_method_enabled(&self, method: &PaymentMethodTypeEnum) -> bool {
        self.method_state(method) == MethodState::Enabled
    }

    pub fn supports_card(&self) -> bool {
        self.is_method_enabled(&PaymentMethodTypeEnum::Card)
    }

    pub fn supports_direct_debit(&self) -> bool {
        payment_capabilities(&self.provider()).is_some_and(|caps| {
            caps.supported_payment_methods
                .iter()
                .any(|m| is_direct_debit(m) && self.is_method_enabled(m))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_keep_the_kind_and_the_tail() {
        assert_eq!(mask_secret("sk_live_51Nx4abcdef4f2a"), "sk_live_…4f2a");
        assert_eq!(mask_secret("rk_test_51Nx4abcdef9z9z"), "rk_test_…9z9z");
        assert_eq!(
            mask_secret("test_dHar4XY7LxsDOtmnkVtjNVWXLSlXsM"),
            "test_…lXsM"
        );
        assert_eq!(mask_secret("sprod_abcdefghij1234"), "sprod_…1234");
        assert_eq!(mask_secret("sandbox_abcdefgh"), "sandbox_…efgh");
        // GoCardless token bodies can contain `_`: none of it may pass for a prefix.
        assert_eq!(mask_secret("live_Ypoc0_Ah6-kQ2_xYz9"), "live_…xYz9");
        // Unknown shape: tail only.
        assert_eq!(mask_secret("whatever_secret_value"), "…alue");
        assert_eq!(mask_secret("ab"), "…ab");
        // Too short to show a prefix and a tail without overlapping.
        assert_eq!(mask_secret("test_ab"), "…t_ab");
    }
}

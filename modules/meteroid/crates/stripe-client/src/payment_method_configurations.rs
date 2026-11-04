use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;

/// A set of payment-method switches: the Dashboard's "Payment methods" settings. Direct
/// (non-Connect) accounts report no capabilities; this is where their toggles live.
#[derive(Clone, Debug, Deserialize)]
pub struct PaymentMethodConfiguration {
    pub id: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub is_default: bool,
    /// Set on configurations a Connect platform manages for this account.
    #[serde(default)]
    pub parent: Option<String>,
    /// Keyed by payment method type (`card`, `sepa_debit`, `us_bank_account`, `bacs_debit`, …).
    #[serde(flatten)]
    pub methods: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MethodSetting {
    /// Whether the account can use the method at all (country, currency, onboarding).
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub display_preference: Option<DisplayPreference>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DisplayPreference {
    /// Effective setting: `on` or `off`.
    pub value: Option<String>,
}

impl PaymentMethodConfiguration {
    /// The switch for one payment method type, if this configuration lists it.
    pub fn method(&self, method_type: &str) -> Option<MethodSetting> {
        self.methods
            .get(method_type)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

impl MethodSetting {
    pub fn is_on(&self) -> bool {
        self.display_preference
            .as_ref()
            .and_then(|p| p.value.as_deref())
            == Some("on")
    }
}

#[derive(Debug, Deserialize)]
struct ConfigurationList {
    #[serde(default)]
    data: Vec<PaymentMethodConfiguration>,
}

#[async_trait::async_trait]
pub trait PaymentMethodConfigurationsApi {
    /// The account's own default configuration, the one checkout and setups use.
    async fn default_payment_method_configuration(
        &self,
        secret_key: &SecretString,
    ) -> Result<Option<PaymentMethodConfiguration>, StripeError>;
}

#[async_trait::async_trait]
impl PaymentMethodConfigurationsApi for StripeClient {
    async fn default_payment_method_configuration(
        &self,
        secret_key: &SecretString,
    ) -> Result<Option<PaymentMethodConfiguration>, StripeError> {
        let list: ConfigurationList = self
            .get(
                "/payment_method_configurations",
                secret_key,
                RetryStrategy::default(),
            )
            .await?;
        Ok(list
            .data
            .into_iter()
            .find(|c| c.is_default && c.active && c.parent.is_none()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_availability_and_preference_per_method() {
        let list: ConfigurationList = serde_json::from_str(
            r#"{"data":[{"id":"pmc_1","active":true,"is_default":true,"parent":null,
                "card":{"available":true,"display_preference":{"overridable":null,"preference":"on","value":"on"}},
                "us_bank_account":{"available":false,"display_preference":{"preference":"off","value":"off"}}}]}"#,
        )
        .unwrap();
        let config = &list.data[0];
        assert!(
            config
                .method("card")
                .is_some_and(|m| m.available && m.is_on())
        );
        let ach = config.method("us_bank_account").unwrap();
        assert!(!ach.available && !ach.is_on());
        assert!(config.method("bacs_debit").is_none());
    }
}

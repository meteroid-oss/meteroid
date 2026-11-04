use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Account {
    pub id: String,
    /// Payment capabilities by name (`card_payments`, `sepa_debit_payments`, …) with their
    /// status: `active`, `pending` or `inactive`.
    #[serde(default)]
    pub capabilities: HashMap<String, String>,
    #[serde(default)]
    pub business_profile: Option<BusinessProfile>,
    #[serde(default)]
    pub settings: Option<AccountSettings>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BusinessProfile {
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccountSettings {
    pub dashboard: Option<DashboardSettings>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DashboardSettings {
    pub display_name: Option<String>,
}

impl Account {
    pub fn display_name(&self) -> Option<String> {
        self.settings
            .as_ref()
            .and_then(|s| s.dashboard.as_ref())
            .and_then(|d| d.display_name.clone())
            .or_else(|| self.business_profile.as_ref().and_then(|b| b.name.clone()))
            .filter(|n| !n.trim().is_empty())
    }
}

#[async_trait::async_trait]
pub trait AccountsApi {
    async fn get_account(&self, secret_key: &SecretString) -> Result<Account, StripeError>;
}

#[async_trait::async_trait]
impl AccountsApi for StripeClient {
    async fn get_account(&self, secret_key: &SecretString) -> Result<Account, StripeError> {
        self.get("/account", secret_key, RetryStrategy::default())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capabilities_and_prefers_the_dashboard_name() {
        let account: Account = serde_json::from_str(
            r#"{"id":"acct_1","capabilities":{"card_payments":"active","sepa_debit_payments":"pending"},
                "business_profile":{"name":"Acme Legal"},"settings":{"dashboard":{"display_name":"Acme"}}}"#,
        )
        .unwrap();
        assert_eq!(account.capabilities["sepa_debit_payments"], "pending");
        assert_eq!(account.display_name().as_deref(), Some("Acme"));

        let bare: Account = serde_json::from_str(r#"{"id":"acct_2"}"#).unwrap();
        assert!(bare.capabilities.is_empty());
        assert_eq!(bare.display_name(), None);
    }
}

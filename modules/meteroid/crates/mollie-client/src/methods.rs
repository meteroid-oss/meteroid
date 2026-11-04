use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// Every payment method Mollie offers, with its status on the API key's profile.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MethodList {
    #[serde(rename = "_embedded", default)]
    pub embedded: Option<MethodListEmbedded>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MethodListEmbedded {
    #[serde(default)]
    pub methods: Vec<Method>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Method {
    /// e.g. `creditcard`, `ideal`, `directdebit`.
    pub id: String,
    /// `activated`, `pending-boarding`, `pending-review`, `pending-external` or `rejected`;
    /// absent when the method isn't enabled on the profile.
    #[serde(default)]
    pub status: Option<String>,
}

/// A method's state on the profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodStatus {
    Activated,
    Pending,
    NotEnabled,
}

impl MethodList {
    pub fn status(&self, method_id: &str) -> MethodStatus {
        let status = self
            .embedded
            .as_ref()
            .and_then(|e| e.methods.iter().find(|m| m.id == method_id))
            .and_then(|m| m.status.as_deref());
        match status {
            Some("activated") => MethodStatus::Activated,
            Some(s) if s.starts_with("pending") => MethodStatus::Pending,
            _ => MethodStatus::NotEnabled,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    /// `pfl_…`
    pub id: String,
    pub name: Option<String>,
    /// `live` or `test`.
    pub mode: Option<String>,
}

impl MollieClient {
    /// One call for every method's status. Also the credential check: it needs a valid key.
    pub async fn list_all_methods(
        &self,
        api_key: &SecretString,
    ) -> Result<MethodList, MollieError> {
        self.get("/methods/all", api_key, RetryStrategy::NoRetry)
            .await
    }

    /// The profile the API key belongs to.
    pub async fn current_profile(&self, api_key: &SecretString) -> Result<Profile, MollieError> {
        self.get("/profiles/me", api_key, RetryStrategy::NoRetry)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_statuses() {
        let list: MethodList = serde_json::from_str(
            r#"{"count":4,"_embedded":{"methods":[
                {"id":"creditcard","status":"activated"},
                {"id":"directdebit","status":"pending-review"},
                {"id":"paypal","status":null},
                {"id":"klarna","status":"rejected"}]}}"#,
        )
        .unwrap();
        assert_eq!(list.status("creditcard"), MethodStatus::Activated);
        assert_eq!(list.status("directdebit"), MethodStatus::Pending);
        assert_eq!(list.status("paypal"), MethodStatus::NotEnabled);
        assert_eq!(list.status("klarna"), MethodStatus::NotEnabled);
        assert_eq!(list.status("ideal"), MethodStatus::NotEnabled);
    }
}

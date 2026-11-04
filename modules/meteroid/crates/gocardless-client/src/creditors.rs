use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Creditor {
    pub id: String,
    pub name: Option<String>,
    /// One entry per scheme the creditor can collect on.
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub scheme_identifiers: Vec<SchemeIdentifier>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SchemeIdentifier {
    /// e.g. `sepa_core`, `bacs`, `ach`, `becs`.
    pub scheme: String,
}

#[derive(Debug, Deserialize)]
struct CreditorEnvelope {
    creditors: Creditor,
}

#[derive(Debug, Deserialize)]
struct CreditorListEnvelope {
    creditors: Vec<Creditor>,
}

#[async_trait::async_trait]
pub trait CreditorApi {
    async fn get_creditor(
        &self,
        creditor_id: &str,
        access_token: &SecretString,
    ) -> Result<Creditor, GoCardlessError>;

    /// Also a cheap authenticated GET with no side effects, so it doubles as the credential check.
    async fn list_creditors(
        &self,
        access_token: &SecretString,
    ) -> Result<Vec<Creditor>, GoCardlessError>;
}

#[async_trait::async_trait]
impl CreditorApi for GoCardlessClient {
    async fn get_creditor(
        &self,
        creditor_id: &str,
        access_token: &SecretString,
    ) -> Result<Creditor, GoCardlessError> {
        let resp: CreditorEnvelope = self
            .get(
                &format!("/creditors/{creditor_id}"),
                access_token,
                RetryStrategy::NoRetry,
            )
            .await?;
        Ok(resp.creditors)
    }

    async fn list_creditors(
        &self,
        access_token: &SecretString,
    ) -> Result<Vec<Creditor>, GoCardlessError> {
        let resp: CreditorListEnvelope = self
            .get("/creditors", access_token, RetryStrategy::NoRetry)
            .await?;
        Ok(resp.creditors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scheme_identifiers() {
        let env: CreditorEnvelope = serde_json::from_str(
            r#"{"creditors":{"id":"CR1","name":"Acme","scheme_identifiers":[{"scheme":"bacs","name":"Acme"},{"scheme":"sepa_core"}]}}"#,
        )
        .unwrap();
        let schemes: Vec<_> = env
            .creditors
            .scheme_identifiers
            .iter()
            .map(|s| s.scheme.as_str())
            .collect();
        assert_eq!(schemes, vec!["bacs", "sepa_core"]);
        let bare: CreditorEnvelope =
            serde_json::from_str(r#"{"creditors":{"id":"CR2","scheme_identifiers":null}}"#)
                .unwrap();
        assert!(bare.creditors.scheme_identifiers.is_empty());
    }
}

//! Refunds resource. Read-only for now: outbound refunds are not implemented,
//! but dashboard/API-initiated refunds surface via `refunds.*` webhooks whose
//! events carry no amounts — we fetch the resource to learn the amount and the
//! parent payment.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Refund {
    /// `RF...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub reference: Option<String>,
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: RefundLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RefundLinks {
    pub payment: Option<String>,
    pub mandate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefundEnvelope {
    refunds: Refund,
}

#[async_trait::async_trait]
pub trait RefundApi {
    async fn get_refund(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Refund, GoCardlessError>;
}

#[async_trait::async_trait]
impl RefundApi for GoCardlessClient {
    async fn get_refund(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Refund, GoCardlessError> {
        let resp: RefundEnvelope = self
            .get(
                &format!("/refunds/{id}"),
                access_token,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.refunds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refund_envelope_parses() {
        let body = r#"{
            "refunds": {
                "id": "RF123",
                "created_at": "2026-05-19T12:00:00.000Z",
                "amount": 500,
                "currency": "EUR",
                "reference": null,
                "metadata": null,
                "links": {"payment": "PM123", "mandate": "MD123"}
            }
        }"#;
        let env: RefundEnvelope = serde_json::from_str(body).unwrap();
        assert_eq!(env.refunds.id, "RF123");
        assert_eq!(env.refunds.amount, 500);
        assert_eq!(env.refunds.links.payment.as_deref(), Some("PM123"));
        assert!(env.refunds.metadata.is_empty());
    }
}

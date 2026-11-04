//! Refunds resource. `refunds.*` webhooks carry no amounts, so the resource is
//! fetched to learn the amount, status and parent payment.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct CreateRefund {
    pub amount: i64,
    /// The payment's refunded total including this refund. GoCardless rejects the refund with
    /// `total_amount_confirmation_invalid` when it disagrees, so two writers can't both refund.
    pub total_amount_confirmation: i64,
    pub reference: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub links: CreateRefundLinks,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateRefundLinks {
    pub payment: String,
}

#[derive(Debug, Serialize)]
struct CreateRefundEnvelope<'a> {
    refunds: &'a CreateRefund,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Refund {
    /// `RF...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub reference: Option<String>,
    #[serde(default)]
    pub status: RefundStatus,
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: RefundLinks,
}

/// <https://developer.gocardless.com/api-reference/#core-endpoints-refunds>
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    Created,
    PendingSubmission,
    Submitted,
    Paid,
    Cancelled,
    Bounced,
    FundsReturned,
    #[default]
    #[serde(other)]
    Unknown,
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

#[derive(Debug, Deserialize)]
struct RefundListEnvelope {
    refunds: Vec<Refund>,
}

#[async_trait::async_trait]
pub trait RefundApi {
    async fn create_refund(
        &self,
        params: CreateRefund,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Refund, GoCardlessError>;

    async fn get_refund(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Refund, GoCardlessError>;

    /// First page (GoCardless' default of 50) of the refunds made against a payment.
    async fn list_payment_refunds(
        &self,
        payment_id: &str,
        access_token: &SecretString,
    ) -> Result<Vec<Refund>, GoCardlessError>;
}

#[async_trait::async_trait]
impl RefundApi for GoCardlessClient {
    async fn create_refund(
        &self,
        params: CreateRefund,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Refund, GoCardlessError> {
        crate::validate_metadata(params.metadata.as_ref())?;
        let resp: RefundEnvelope = self
            .post_create(
                "/refunds",
                CreateRefundEnvelope { refunds: &params },
                access_token,
                idempotency_key,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.refunds)
    }

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

    async fn list_payment_refunds(
        &self,
        payment_id: &str,
        access_token: &SecretString,
    ) -> Result<Vec<Refund>, GoCardlessError> {
        let resp: RefundListEnvelope = self
            .get(
                &format!("/refunds?payment={payment_id}"),
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
                "status": "pending_submission",
                "metadata": null,
                "links": {"payment": "PM123", "mandate": "MD123"}
            }
        }"#;
        let env: RefundEnvelope = serde_json::from_str(body).unwrap();
        assert_eq!(env.refunds.id, "RF123");
        assert_eq!(env.refunds.amount, 500);
        assert_eq!(env.refunds.links.payment.as_deref(), Some("PM123"));
        assert!(env.refunds.metadata.is_empty());
        assert_eq!(env.refunds.status, RefundStatus::PendingSubmission);
    }

    #[test]
    fn refund_list_parses() {
        let env: RefundListEnvelope = serde_json::from_str(
            r#"{"refunds":[{"id":"RF1","amount":5,"currency":"EUR","reference":null,
                "status":"paid","metadata":{"meteroid.refund_id":"r1"},"links":{"payment":"PM1"}}],
                "meta":{"cursors":{"before":null,"after":null},"limit":50}}"#,
        )
        .unwrap();
        assert_eq!(env.refunds.len(), 1);
        assert_eq!(env.refunds[0].metadata["meteroid.refund_id"], "r1");
    }

    #[test]
    fn unknown_or_missing_refund_status_parses() {
        let unknown: Refund = serde_json::from_str(
            r#"{"id":"RF1","amount":1,"currency":"EUR","reference":null,"status":"brand_new"}"#,
        )
        .unwrap();
        assert_eq!(unknown.status, RefundStatus::Unknown);
        let missing: Refund =
            serde_json::from_str(r#"{"id":"RF1","amount":1,"currency":"EUR","reference":null}"#)
                .unwrap();
        assert_eq!(missing.status, RefundStatus::Unknown);
    }

    #[test]
    fn create_refund_body_is_wrapped_and_confirms_the_total() {
        let params = CreateRefund {
            amount: 500,
            total_amount_confirmation: 800,
            reference: None,
            metadata: None,
            links: CreateRefundLinks {
                payment: "PM123".into(),
            },
        };
        let body = serde_json::to_value(CreateRefundEnvelope { refunds: &params }).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"refunds": {
                "amount": 500,
                "total_amount_confirmation": 800,
                "links": {"payment": "PM123"}
            }})
        );
    }
}

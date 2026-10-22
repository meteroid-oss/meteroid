use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// `amount` omitted refunds the full remaining payment amount.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
pub struct RefundCreate {
    pub payment: String,
    pub amount: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerRefund {
    pub id: String,
    pub payment: String,
    pub amount: i64,
    pub status: RefundStatus,
}

/// Mirrors Stancer's `RefundStatus` schema (OpenAPI-verified: `to_refund,
/// refund_sent, refunded, not_honored, payment_canceled, failed,
/// awaiting_approval`).
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    ToRefund,
    RefundSent,
    Refunded,
    NotHonored,
    PaymentCanceled,
    Failed,
    AwaitingApproval,
    /// Any unmodeled status. Must never fail the response parse.
    #[serde(other)]
    Unknown,
}

impl StancerClient {
    /// `POST /v2/refunds/`. `RefundCreate` carries no idempotency field
    /// (OpenAPI-verified), so a client-side retry of a timed-out call could
    /// double-refund: use `NoRetry` and let the caller decide, rather than
    /// silently retrying underneath it.
    pub async fn create_refund(
        &self,
        params: RefundCreate,
        secret_key: &SecretString,
    ) -> Result<StancerRefund, StancerError> {
        self.post_json("/refunds/", params, secret_key, RetryStrategy::NoRetry)
            .await
    }

    /// `GET /v2/payments/{id}/refunds` — a bare array (unlike `/v2/refunds/`'s
    /// paginated envelope). Lets a caller check for an already-accepted refund
    /// before creating a new one, since no idempotency key exists provider-side.
    pub async fn list_payment_refunds(
        &self,
        payment_id: &str,
        secret_key: &SecretString,
    ) -> Result<Vec<StancerRefund>, StancerError> {
        self.get(
            &format!("/payments/{payment_id}/refunds"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::RefundStatus;

    fn parse(status: &str) -> RefundStatus {
        serde_json::from_value(serde_json::Value::String(status.to_string()))
            .expect("status must deserialize")
    }

    #[test]
    fn all_spec_statuses_deserialize() {
        assert_eq!(parse("to_refund"), RefundStatus::ToRefund);
        assert_eq!(parse("refund_sent"), RefundStatus::RefundSent);
        assert_eq!(parse("refunded"), RefundStatus::Refunded);
        assert_eq!(parse("not_honored"), RefundStatus::NotHonored);
        assert_eq!(parse("payment_canceled"), RefundStatus::PaymentCanceled);
        assert_eq!(parse("failed"), RefundStatus::Failed);
        assert_eq!(parse("awaiting_approval"), RefundStatus::AwaitingApproval);
    }

    #[test]
    fn unknown_status_falls_back_instead_of_erroring() {
        assert_eq!(parse("totally_new_status"), RefundStatus::Unknown);
    }
}

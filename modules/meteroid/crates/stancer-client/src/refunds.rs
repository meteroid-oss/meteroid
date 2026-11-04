use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// `amount` omitted refunds whatever is left on the payment.
#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct CreateRefund {
    pub payment: String,
    pub amount: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerRefund {
    pub id: String,
    pub payment: String,
    pub amount: i64,
    pub currency: String,
    pub status: RefundStatus,
}

/// source: https://docs.stancer.com/api/openapi.json (`RefundStatus`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    ToRefund,
    RefundSent,
    Refunded,
    NotHonored,
    PaymentCanceled,
    Failed,
    AwaitingApproval,
    #[serde(other)]
    Unknown,
}

impl StancerClient {
    /// `POST /v2/refunds/`. Never retried: the body has no idempotency field, so a retry after a
    /// lost response would refund twice.
    pub async fn create_refund(
        &self,
        params: CreateRefund,
        secret_key: &SecretString,
    ) -> Result<StancerRefund, StancerError> {
        self.post_json("/refunds/", params, secret_key, RetryStrategy::NoRetry)
            .await
    }

    pub async fn get_refund(
        &self,
        refund_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerRefund, StancerError> {
        self.get(
            &format!("/refunds/{refund_id}"),
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
        serde_json::from_value(serde_json::Value::String(status.to_string())).unwrap()
    }

    #[test]
    fn spec_statuses_deserialize_and_new_ones_fall_back() {
        assert_eq!(parse("to_refund"), RefundStatus::ToRefund);
        assert_eq!(parse("refund_sent"), RefundStatus::RefundSent);
        assert_eq!(parse("refunded"), RefundStatus::Refunded);
        assert_eq!(parse("not_honored"), RefundStatus::NotHonored);
        assert_eq!(parse("payment_canceled"), RefundStatus::PaymentCanceled);
        assert_eq!(parse("failed"), RefundStatus::Failed);
        assert_eq!(parse("awaiting_approval"), RefundStatus::AwaitingApproval);
        assert_eq!(parse("something_new"), RefundStatus::Unknown);
    }
}

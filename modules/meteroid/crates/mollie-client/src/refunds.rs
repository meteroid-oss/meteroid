use crate::amount::Amount;
use crate::client::MollieClient;
use crate::error::MollieError;
use crate::payments::deserialize_metadata;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// `POST /v2/payments/{paymentId}/refunds`. `description` may be shown to the customer.
#[skip_serializing_none]
#[derive(Clone, Debug, Serialize)]
pub struct CreateRefund {
    pub amount: Amount,
    pub description: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

/// `refund-status` schema; `refunded`, `failed` and `canceled` are terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefundStatus {
    Queued,
    Pending,
    Processing,
    Refunded,
    Failed,
    Canceled,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MollieRefund {
    pub id: String,
    pub amount: Amount,
    pub status: RefundStatus,
    pub payment_id: Option<String>,
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "deserialize_metadata")]
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RefundList {
    #[serde(rename = "_embedded", default)]
    pub embedded: Option<RefundListEmbedded>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RefundListEmbedded {
    #[serde(default)]
    pub refunds: Vec<MollieRefund>,
}

impl RefundList {
    pub fn into_refunds(self) -> Vec<MollieRefund> {
        self.embedded.map(|e| e.refunds).unwrap_or_default()
    }
}

impl MollieClient {
    pub async fn create_payment_refund(
        &self,
        payment_id: &str,
        params: CreateRefund,
        api_key: &SecretString,
        idempotency_key: &str,
    ) -> Result<MollieRefund, MollieError> {
        self.post_json(
            &format!("/payments/{payment_id}/refunds"),
            params,
            api_key,
            Some(idempotency_key),
            RetryStrategy::default(),
        )
        .await
    }

    /// Mollie has no refund lookup by id alone: a refund is always read through its payment.
    pub async fn get_payment_refund(
        &self,
        payment_id: &str,
        refund_id: &str,
        api_key: &SecretString,
    ) -> Result<MollieRefund, MollieError> {
        self.get(
            &format!("/payments/{payment_id}/refunds/{refund_id}"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }

    /// First page only (Mollie's default of 50): a payment refunded more often than that is
    /// covered by the cumulative `amountRefunded`.
    pub async fn list_payment_refunds(
        &self,
        payment_id: &str,
        api_key: &SecretString,
    ) -> Result<RefundList, MollieError> {
        self.get(
            &format!("/payments/{payment_id}/refunds"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refund_list_parses() {
        let list: RefundList = serde_json::from_value(serde_json::json!({
            "count": 2,
            "_embedded": {"refunds": [
                {"resource": "refund", "id": "re_1", "amount": {"currency": "EUR", "value": "4.00"},
                 "status": "refunded", "paymentId": "tr_1", "createdAt": "2026-09-01T10:00:00+00:00",
                 "metadata": {"meteroid.refund_id": "abc"}},
                {"resource": "refund", "id": "re_2", "amount": {"currency": "EUR", "value": "1.50"},
                 "status": "something_new", "paymentId": "tr_1", "metadata": null}
            ]}
        }))
        .unwrap();
        let refunds = list.into_refunds();
        assert_eq!(refunds.len(), 2);
        assert_eq!(refunds[0].status, RefundStatus::Refunded);
        assert_eq!(refunds[0].amount.to_minor(2).unwrap(), 400);
        assert_eq!(refunds[0].metadata["meteroid.refund_id"], "abc");
        assert_eq!(refunds[1].status, RefundStatus::Unknown);
    }

    #[test]
    fn create_refund_serializes_amount_as_decimal_string() {
        let body = serde_json::to_value(CreateRefund {
            amount: Amount::from_minor(1050, "eur", 2),
            description: None,
            metadata: None,
        })
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({"amount": {"currency": "EUR", "value": "10.50"}})
        );
    }
}

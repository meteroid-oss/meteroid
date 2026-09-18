use crate::amount::Amount;
use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// `sequence-type` schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SequenceType {
    Oneoff,
    First,
    Recurring,
}

/// `payment-status` schema; `paid` / `failed` / `canceled` / `expired` are terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentStatus {
    Open,
    Pending,
    Authorized,
    Paid,
    Canceled,
    Expired,
    Failed,
    /// Any other status; parsed without error and treated as non-terminal.
    #[serde(other)]
    Unknown,
}

/// `POST /v2/payments`. `redirectUrl` is required except for `recurring`, which needs `customerId`.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePayment {
    pub amount: Amount,
    pub description: String,
    pub redirect_url: Option<String>,
    pub cancel_url: Option<String>,
    pub webhook_url: Option<String>,
    /// Sent as a string for one value, an array for several; omit to let the customer choose.
    #[serde(serialize_with = "serialize_methods")]
    pub method: Option<Vec<String>>,
    pub sequence_type: Option<SequenceType>,
    pub customer_id: Option<String>,
    pub mandate_id: Option<String>,
    pub locale: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

/// `PATCH /v2/payments/{id}` — only while the payment is `open`.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePayment {
    pub redirect_url: Option<String>,
    pub cancel_url: Option<String>,
    pub webhook_url: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

/// Method-specific `details`; only the card and SEPA fields we use.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentDetails {
    pub card_number: Option<String>,
    pub card_holder: Option<String>,
    pub card_label: Option<String>,
    /// `MM/YY` on payments (mandates use `YYYY-MM-DD`).
    pub card_expiry_date: Option<String>,
    /// `payment-details-failure-reason-response` enum, e.g. `insufficient_funds`.
    pub failure_reason: Option<String>,
    pub failure_message: Option<String>,
    pub bank_reason_code: Option<String>,
    pub bank_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentLinks {
    pub checkout: Option<Link>,
    pub refunds: Option<Link>,
    pub chargebacks: Option<Link>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Link {
    pub href: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MolliePayment {
    pub id: String,
    pub mode: Option<String>,
    pub status: PaymentStatus,
    pub amount: Amount,
    /// Cumulative refunded total; present only once a refund exists.
    pub amount_refunded: Option<Amount>,
    /// Cumulative charged-back total; present only when non-zero.
    pub amount_charged_back: Option<Amount>,
    pub description: Option<String>,
    pub method: Option<String>,
    pub sequence_type: Option<SequenceType>,
    pub customer_id: Option<String>,
    pub mandate_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_metadata")]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub details: Option<PaymentDetails>,
    pub created_at: Option<String>,
    pub paid_at: Option<String>,
    pub failed_at: Option<String>,
    pub canceled_at: Option<String>,
    pub expired_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_cancelable: Option<bool>,
    #[serde(rename = "_links")]
    pub links: Option<PaymentLinks>,
}

impl MolliePayment {
    pub fn checkout_url(&self) -> Option<&str> {
        self.links
            .as_ref()
            .and_then(|l| l.checkout.as_ref())
            .map(|l| l.href.as_str())
    }
}

fn serialize_methods<S: serde::Serializer>(
    methods: &Option<Vec<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match methods.as_deref() {
        Some([single]) => serializer.serialize_str(single),
        Some(many) => many.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

/// Mollie `metadata` is free-form; we only write a flat string map, so anything else is dropped
/// instead of failing the parse.
pub fn deserialize_metadata<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Object(map)) => map
            .into_iter()
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) => Some((k, s)),
                serde_json::Value::Number(n) => Some((k, n.to_string())),
                serde_json::Value::Bool(b) => Some((k, b.to_string())),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    })
}

impl MollieClient {
    pub async fn create_payment(
        &self,
        params: CreatePayment,
        api_key: &SecretString,
        idempotency_key: &str,
    ) -> Result<MolliePayment, MollieError> {
        self.post_json(
            "/payments",
            params,
            api_key,
            Some(idempotency_key),
            RetryStrategy::default(),
        )
        .await
    }

    pub async fn get_payment(
        &self,
        payment_id: &str,
        api_key: &SecretString,
    ) -> Result<MolliePayment, MollieError> {
        self.get(
            &format!("/payments/{payment_id}"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }

    /// Cancels a payment while Mollie reports it `isCancelable`; 422 otherwise.
    pub async fn cancel_payment(
        &self,
        payment_id: &str,
        api_key: &SecretString,
    ) -> Result<MolliePayment, MollieError> {
        self.delete(
            &format!("/payments/{payment_id}"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }

    pub async fn update_payment(
        &self,
        payment_id: &str,
        params: UpdatePayment,
        api_key: &SecretString,
    ) -> Result<MolliePayment, MollieError> {
        self.patch_json(
            &format!("/payments/{payment_id}"),
            params,
            api_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(status: &str) -> PaymentStatus {
        serde_json::from_value(serde_json::Value::String(status.to_string()))
            .expect("status must deserialize")
    }

    #[test]
    fn all_spec_statuses_deserialize() {
        assert_eq!(parse("open"), PaymentStatus::Open);
        assert_eq!(parse("pending"), PaymentStatus::Pending);
        assert_eq!(parse("authorized"), PaymentStatus::Authorized);
        assert_eq!(parse("paid"), PaymentStatus::Paid);
        assert_eq!(parse("canceled"), PaymentStatus::Canceled);
        assert_eq!(parse("expired"), PaymentStatus::Expired);
        assert_eq!(parse("failed"), PaymentStatus::Failed);
        assert_eq!(parse("brand_new"), PaymentStatus::Unknown);
    }

    #[test]
    fn first_payment_response_deserializes() {
        let payment: MolliePayment = serde_json::from_str(
            r#"{"resource":"payment","id":"tr_7UhSN1zuXS","mode":"test","status":"open",
                "amount":{"currency":"EUR","value":"0.00"},"description":"First payment",
                "method":"creditcard","metadata":{"meteroid.customer_id":"abc","nested":{"x":1},"n":3},
                "sequenceType":"first","customerId":"cst_kEn1PlbGa","mandateId":"mdt_h3gAaD5zP",
                "expiresAt":"2024-03-20T09:44:56+00:00","isCancelable":false,"details":null,
                "_links":{"checkout":{"href":"https://www.mollie.com/checkout/test-mode?method=creditcard&token=3.ivicl6","type":"text/html"}}}"#,
        )
        .expect("payment must deserialize");
        assert_eq!(payment.status, PaymentStatus::Open);
        assert_eq!(payment.sequence_type, Some(SequenceType::First));
        assert_eq!(payment.amount.to_minor(2), Ok(0));
        assert_eq!(
            payment
                .metadata
                .get("meteroid.customer_id")
                .map(String::as_str),
            Some("abc")
        );
        assert_eq!(payment.metadata.get("n").map(String::as_str), Some("3"));
        assert!(!payment.metadata.contains_key("nested"));
        assert!(
            payment
                .checkout_url()
                .unwrap()
                .contains("mollie.com/checkout")
        );
    }

    #[test]
    fn string_metadata_and_failure_details_parse() {
        let payment: MolliePayment = serde_json::from_str(
            r#"{"id":"tr_x","status":"failed","amount":{"currency":"EUR","value":"10.00"},
                "metadata":"free text","details":{"failureReason":"insufficient_funds","failureMessage":"Not enough"}}"#,
        )
        .unwrap();
        assert!(payment.metadata.is_empty());
        assert_eq!(
            payment.details.unwrap().failure_reason.as_deref(),
            Some("insufficient_funds")
        );
    }

    #[test]
    fn create_payment_serializes_camel_case_and_omits_none() {
        let body = serde_json::to_value(CreatePayment {
            amount: Amount::from_minor(1999, "eur", 2),
            description: "Renewal".into(),
            sequence_type: Some(SequenceType::Recurring),
            customer_id: Some("cst_1".into()),
            mandate_id: Some("mdt_1".into()),
            webhook_url: Some("https://x.invalid/hook".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(body["amount"]["value"], "19.99");
        assert_eq!(body["sequenceType"], "recurring");
        assert_eq!(body["customerId"], "cst_1");
        assert!(body.get("redirectUrl").is_none());
        assert!(body.get("method").is_none());
    }

    #[test]
    fn method_serializes_as_string_or_array() {
        let single = serde_json::to_value(CreatePayment {
            amount: Amount::from_minor(0, "eur", 2),
            description: "x".into(),
            method: Some(vec!["creditcard".into()]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(single["method"], "creditcard");
        let many = serde_json::to_value(CreatePayment {
            amount: Amount::from_minor(1, "eur", 2),
            description: "x".into(),
            method: Some(vec!["ideal".into(), "bancontact".into()]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(many["method"], serde_json::json!(["ideal", "bancontact"]));
    }
}

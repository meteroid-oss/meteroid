use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Charges an already-tokenized card (`card_xxx`, obtained via the
/// payment_intents hosted-page flow — see `payment_intents.rs`). Meteroid
/// never submits raw card numbers itself, so `card` is always an existing
/// card id, never inline card data.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
pub struct CreatePayment {
    pub amount: i64,
    pub currency: String,
    pub customer: Option<String>,
    pub card: Option<String>,
    pub description: Option<String>,
    /// Non-unique correlation id (≤36 chars).
    pub order_id: Option<String>,
    /// Unique id (≤36 chars) — Stancer's only idempotency mechanism for
    /// this endpoint.
    pub unique_id: Option<String>,
    pub capture: bool,
    // Deliberately no `auth` field: omitting it entirely is what skips the
    // 3DS challenge for an off-session charge (verified live). Sending
    // `auth: false` explicitly is rejected by Stancer's API validation
    // ("Auth can't be False").
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerPayment {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: Option<StancerPaymentStatus>,
    /// ISO-8583-style network response code; `"00"` means approved.
    pub response: Option<String>,
    pub card: Option<StancerPaymentCard>,
    pub order_id: Option<String>,
    pub unique_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerPaymentCard {
    pub id: String,
    pub brand: Option<String>,
    pub last4: Option<String>,
    pub exp_month: Option<u32>,
    pub exp_year: Option<u32>,
}

/// Mirrors Stancer's `StatusCode` schema (11 values incl. the transient
/// `authorize`/`capture`). Settlement is asynchronous with no webhook push
/// (confirmed live): a successful charge starts at `ToCapture`/`CaptureSent`
/// and only later resolves to `Captured` — callers must poll `get_payment`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StancerPaymentStatus {
    Refused,
    Authorize,
    Authorized,
    Expired,
    ToCapture,
    Capture,
    CaptureSent,
    Captured,
    Disputed,
    Canceled,
    Failed,
    /// Any status this client doesn't model. Must never fail the response
    /// parse (that would turn a successful charge into an error and wedge
    /// reconciliation); callers treat it as non-terminal.
    #[serde(other)]
    Unknown,
}

/// Paginated envelope of `GET /v2/payments/` (spec `PaymentOutList`). Only
/// the entries are consumed; the `range` block is ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct StancerPaymentList {
    pub payments: Vec<StancerPayment>,
}

impl StancerClient {
    pub async fn create_payment(
        &self,
        params: CreatePayment,
        secret_key: &SecretString,
    ) -> Result<StancerPayment, StancerError> {
        self.post_json("/payments/", params, secret_key, RetryStrategy::default())
            .await
    }

    pub async fn get_payment(
        &self,
        payment_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerPayment, StancerError> {
        self.get(
            &format!("/payments/{payment_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }

    /// `GET /v2/payments/?unique_id=…` — resolve the payment created with a
    /// given (unicity-checked) `unique_id`. Used to recover a charge whose
    /// create call raced a crash/rollback before the returned `paym_…` id was
    /// recorded locally: instead of re-charging (the duplicate `unique_id`
    /// would be rejected anyway), adopt the existing payment.
    pub async fn list_payments_by_unique_id(
        &self,
        unique_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerPaymentList, StancerError> {
        self.get_with_query(
            "/payments/",
            &[("unique_id", unique_id)],
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::StancerPaymentStatus;

    fn parse(status: &str) -> StancerPaymentStatus {
        serde_json::from_value(serde_json::Value::String(status.to_string()))
            .expect("status must deserialize")
    }

    /// Every spec `StatusCode` value must deserialize — an unmodeled status
    /// would fail the WHOLE payment parse, turning a successful charge into
    /// an error and wedging reconciliation (Stancer's only settlement path).
    #[test]
    fn all_spec_statuses_deserialize() {
        assert_eq!(parse("refused"), StancerPaymentStatus::Refused);
        assert_eq!(parse("authorize"), StancerPaymentStatus::Authorize);
        assert_eq!(parse("authorized"), StancerPaymentStatus::Authorized);
        assert_eq!(parse("expired"), StancerPaymentStatus::Expired);
        assert_eq!(parse("to_capture"), StancerPaymentStatus::ToCapture);
        assert_eq!(parse("capture"), StancerPaymentStatus::Capture);
        assert_eq!(parse("capture_sent"), StancerPaymentStatus::CaptureSent);
        assert_eq!(parse("captured"), StancerPaymentStatus::Captured);
        assert_eq!(parse("disputed"), StancerPaymentStatus::Disputed);
        assert_eq!(parse("canceled"), StancerPaymentStatus::Canceled);
        assert_eq!(parse("failed"), StancerPaymentStatus::Failed);
    }

    /// A status Stancer introduces tomorrow must parse as `Unknown`, never
    /// error out of the full response deserialization.
    #[test]
    fn unknown_status_falls_back_instead_of_erroring() {
        assert_eq!(parse("totally_new_status"), StancerPaymentStatus::Unknown);
    }

    /// Full payment payload with an unmodeled status still parses.
    #[test]
    fn payment_with_unknown_status_deserializes() {
        let payment: super::StancerPayment = serde_json::from_str(
            r#"{"id":"paym_x","amount":100,"currency":"eur","status":"totally_new_status"}"#,
        )
        .expect("payment must deserialize despite unknown status");
        assert_eq!(payment.status, Some(StancerPaymentStatus::Unknown));
    }
}

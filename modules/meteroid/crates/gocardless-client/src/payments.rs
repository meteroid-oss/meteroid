//! Payments resource. A payment debits a mandate (id in `links.mandate`).
//! Settlement is async over multiple business days; status transitions arrive
//! via webhooks and can be polled via `GET /payments/:id` for reconciliation.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct CreatePayment {
    pub amount: i64,
    pub currency: String,
    pub description: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub charge_date: Option<String>,
    pub reference: Option<String>,
    pub links: CreatePaymentLinks,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatePaymentLinks {
    pub mandate: String,
}

#[derive(Debug, Serialize)]
struct CreatePaymentEnvelope<'a> {
    payments: &'a CreatePayment,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Payment {
    /// `PM...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub status: PaymentStatus,
    pub charge_date: Option<String>,
    pub reference: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: PaymentLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PaymentLinks {
    pub mandate: Option<String>,
    pub creditor: Option<String>,
}

/// Full payment lifecycle. See
/// <https://developer.gocardless.com/api-reference/#payments-payments>.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    PendingCustomerApproval,
    PendingSubmission,
    Submitted,
    Confirmed,
    PaidOut,
    Cancelled,
    CustomerApprovalDenied,
    Failed,
    ChargedBack,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct PaymentEnvelope {
    payments: Payment,
}

#[async_trait::async_trait]
pub trait PaymentApi {
    async fn create_payment(
        &self,
        params: CreatePayment,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Payment, GoCardlessError>;

    async fn get_payment(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Payment, GoCardlessError>;
}

#[async_trait::async_trait]
impl PaymentApi for GoCardlessClient {
    async fn create_payment(
        &self,
        params: CreatePayment,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Payment, GoCardlessError> {
        let resp: PaymentEnvelope = self
            .post(
                "/payments",
                CreatePaymentEnvelope { payments: &params },
                access_token,
                Some(idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.payments)
    }

    async fn get_payment(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Payment, GoCardlessError> {
        let resp: PaymentEnvelope = self
            .get(
                &format!("/payments/{id}"),
                access_token,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.payments)
    }
}

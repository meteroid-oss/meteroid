//! Billing Requests + Billing Request Flows.
//!
//! Create a billing request, mint an authorisation_url via a flow to redirect the
//! customer, then call the complete action on return; the completed request carries
//! `links.mandate` (and customer / bank-account links when GC creates them).
//! See <https://developer.gocardless.com/billing-requests>.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

// ── Create Billing Request ─────────────────────────────────────────

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct CreateBillingRequest {
    pub mandate_request: Option<MandateRequest>,
    pub payment_request: Option<PaymentRequest>,
    pub metadata: Option<HashMap<String, String>>,
    pub links: Option<BillingRequestLinks>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct MandateRequest {
    /// ISO 4217. EUR → SEPA, GBP → BACS, USD → ACH, AUD → BECS, etc.
    pub currency: String,
    /// Optional; GoCardless infers from currency if not set.
    pub scheme: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRequest {
    pub amount: i64,
    pub currency: String,
    pub description: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct BillingRequestLinks {
    pub customer: Option<String>,
    pub creditor: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateBillingRequestEnvelope<'a> {
    billing_requests: &'a CreateBillingRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BillingRequest {
    /// `BRQ...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub status: BillingRequestStatus,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: BillingRequestResponseLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BillingRequestResponseLinks {
    pub mandate: Option<String>,
    pub customer: Option<String>,
    pub customer_bank_account: Option<String>,
    pub creditor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingRequestStatus {
    Pending,
    ReadyToFulfil,
    Fulfilling,
    Fulfilled,
    Cancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct BillingRequestEnvelope {
    billing_requests: BillingRequest,
}

// ── Create Billing Request Flow ────────────────────────────────────

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct CreateBillingRequestFlow {
    pub redirect_uri: Option<String>,
    pub exit_uri: Option<String>,
    pub lock_currency: Option<bool>,
    pub lock_amount: Option<bool>,
    pub lock_bank_account: Option<bool>,
    pub auto_fulfil: Option<bool>,
    pub links: BillingRequestFlowLinks,
}

#[derive(Debug, Clone, Serialize)]
pub struct BillingRequestFlowLinks {
    pub billing_request: String,
}

#[derive(Debug, Serialize)]
struct CreateBillingRequestFlowEnvelope<'a> {
    billing_request_flows: &'a CreateBillingRequestFlow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BillingRequestFlow {
    /// `BRF...` prefix.
    pub id: String,
    /// Redirect target where the customer gives mandate consent on GC-hosted UI.
    pub authorisation_url: String,
    /// ISO 8601; typically 24h after creation.
    pub expires_at: Option<String>,
    #[serde(default)]
    pub links: BillingRequestFlowResponseLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BillingRequestFlowResponseLinks {
    pub billing_request: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BillingRequestFlowEnvelope {
    billing_request_flows: BillingRequestFlow,
}

// ── Complete action ────────────────────────────────────────────────

/// Finalises the billing request on return; the mandate / customer / bank
/// account can't be used until this succeeds.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CompleteBillingRequest {
    // Empty body — the action is implicit in the path.
}

#[derive(Debug, Serialize)]
struct CompleteBillingRequestEnvelope<'a> {
    data: &'a CompleteBillingRequest,
}

// ── API trait ──────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait BillingRequestApi {
    async fn create_billing_request(
        &self,
        params: CreateBillingRequest,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<BillingRequest, GoCardlessError>;

    async fn create_billing_request_flow(
        &self,
        params: CreateBillingRequestFlow,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<BillingRequestFlow, GoCardlessError>;

    async fn complete_billing_request(
        &self,
        billing_request_id: &str,
        access_token: &SecretString,
    ) -> Result<BillingRequest, GoCardlessError>;
}

#[async_trait::async_trait]
impl BillingRequestApi for GoCardlessClient {
    async fn create_billing_request(
        &self,
        params: CreateBillingRequest,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<BillingRequest, GoCardlessError> {
        let resp: BillingRequestEnvelope = self
            .post(
                "/billing_requests",
                CreateBillingRequestEnvelope {
                    billing_requests: &params,
                },
                access_token,
                Some(idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.billing_requests)
    }

    async fn create_billing_request_flow(
        &self,
        params: CreateBillingRequestFlow,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<BillingRequestFlow, GoCardlessError> {
        let resp: BillingRequestFlowEnvelope = self
            .post(
                "/billing_request_flows",
                CreateBillingRequestFlowEnvelope {
                    billing_request_flows: &params,
                },
                access_token,
                Some(idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.billing_request_flows)
    }

    async fn complete_billing_request(
        &self,
        billing_request_id: &str,
        access_token: &SecretString,
    ) -> Result<BillingRequest, GoCardlessError> {
        // Key derived from the BR id so duplicate return-URL hits dedup on GC:
        // during the fulfilling window a keyless retry can race into a 409.
        let idempotency_key = format!("brf-complete:{billing_request_id}");
        let resp: BillingRequestEnvelope = self
            .post(
                &format!("/billing_requests/{billing_request_id}/actions/complete"),
                CompleteBillingRequestEnvelope {
                    data: &CompleteBillingRequest::default(),
                },
                access_token,
                Some(&idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.billing_requests)
    }
}

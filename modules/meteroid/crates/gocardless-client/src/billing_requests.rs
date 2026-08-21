//! Billing Requests + Billing Request Flows.
//!
//! Create a billing request, mint an authorisation_url via a flow to redirect the
//! customer; once fulfilled the request carries the created mandate under
//! `links.mandate_request_mandate` (plus customer / bank-account links).
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
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: BillingRequestResponseLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BillingRequestResponseLinks {
    /// The mandate CREATED from `mandate_request`, populated once the BR is
    /// fulfilled. Note: a billing request has NO plain `mandate` link — the
    /// created mandate lives under this `mandate_request_mandate` field.
    pub mandate_request_mandate: Option<String>,
    /// The payment CREATED from `payment_request` (instant-first-payment BRs).
    pub payment_request_payment: Option<String>,
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

// ── Fulfil action ──────────────────────────────────────────────────

/// Finalises the billing request on return; the mandate / customer / bank
/// account can't be used until this succeeds.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FulfilBillingRequest {
    // Empty body — the action is implicit in the path.
}

#[derive(Debug, Serialize)]
struct FulfilBillingRequestEnvelope<'a> {
    data: &'a FulfilBillingRequest,
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

    async fn fulfil_billing_request(
        &self,
        billing_request_id: &str,
        access_token: &SecretString,
    ) -> Result<BillingRequest, GoCardlessError>;

    /// Fetch a Billing Request. Unlike `fulfil`, this is a read: safe to call
    /// on an already-fulfilled BR (the hosted flow auto-fulfils asynchronously,
    /// so completion must be observed, not driven). Returns our own `metadata`
    /// and `links.mandate_request_mandate` for the created mandate.
    async fn get_billing_request(
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
        crate::validate_metadata(params.metadata.as_ref())?;
        if let Some(mr) = &params.mandate_request {
            crate::validate_metadata(mr.metadata.as_ref())?;
        }
        if let Some(pr) = &params.payment_request {
            crate::validate_metadata(pr.metadata.as_ref())?;
        }
        let resp: BillingRequestEnvelope = self
            .post_create(
                "/billing_requests",
                CreateBillingRequestEnvelope {
                    billing_requests: &params,
                },
                access_token,
                idempotency_key,
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
        // Plain `post`, not `post_create`: GoCardless exposes no GET for a
        // billing request flow, so the 409 idempotent-conflict recovery
        // (which would GET `{path}/{id}`) has nothing to fetch and 404s.
        // Surface the 409 as-is; a fresh attempt mints its own key.
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

    async fn fulfil_billing_request(
        &self,
        billing_request_id: &str,
        access_token: &SecretString,
    ) -> Result<BillingRequest, GoCardlessError> {
        // Key derived from the BR id so duplicate return-URL hits dedup on GC:
        // during the fulfilling window a keyless retry can race into a 409.
        let idempotency_key = format!("brf-fulfil:{billing_request_id}");
        let resp: BillingRequestEnvelope = self
            .post(
                &format!("/billing_requests/{billing_request_id}/actions/fulfil"),
                FulfilBillingRequestEnvelope {
                    data: &FulfilBillingRequest::default(),
                },
                access_token,
                Some(&idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.billing_requests)
    }

    async fn get_billing_request(
        &self,
        billing_request_id: &str,
        access_token: &SecretString,
    ) -> Result<BillingRequest, GoCardlessError> {
        let resp: BillingRequestEnvelope = self
            .get(
                &format!("/billing_requests/{billing_request_id}"),
                access_token,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.billing_requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fulfilled BR links the created mandate as `mandate_request_mandate`,
    /// NOT `mandate` (which is not a billing-request link at all). Regression
    /// guard: reading the wrong field made every mandate setup fail with
    /// "has no mandate yet (not fulfilled?)".
    #[test]
    fn deserializes_fulfilled_mandate_link() {
        let json = r#"{
            "billing_requests": {
                "id": "BRQ123",
                "status": "fulfilled",
                "links": {
                    "mandate_request": "MRQ123",
                    "mandate_request_mandate": "MD123",
                    "customer": "CU123"
                }
            }
        }"#;
        let env: BillingRequestEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.billing_requests.status, BillingRequestStatus::Fulfilled);
        assert_eq!(
            env.billing_requests
                .links
                .mandate_request_mandate
                .as_deref(),
            Some("MD123"),
        );
    }
}

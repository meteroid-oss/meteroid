//! Mandates resource. A mandate is the persistent authorisation we collected
//! via the Billing Request Flow; it's what we charge against via /payments.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Mandate {
    /// `MD...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub status: MandateStatus,
    pub reference: Option<String>,
    /// Underlying scheme: `bacs`, `sepa_core`, `ach`, `becs`, etc.
    pub scheme: Option<String>,
    pub next_possible_charge_date: Option<String>,
    #[serde(default, deserialize_with = "crate::null_as_default")]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub links: MandateLinks,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MandateLinks {
    pub customer: Option<String>,
    pub customer_bank_account: Option<String>,
    pub creditor: Option<String>,
}

/// Mandate lifecycle. `active` is the only status we'll charge against.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateStatus {
    PendingCustomerApproval,
    PendingSubmission,
    Submitted,
    Active,
    Failed,
    Cancelled,
    Expired,
    Consumed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct MandateEnvelope {
    mandates: Mandate,
}

#[async_trait::async_trait]
pub trait MandateApi {
    async fn get_mandate(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Mandate, GoCardlessError>;
}

#[async_trait::async_trait]
impl MandateApi for GoCardlessClient {
    async fn get_mandate(
        &self,
        id: &str,
        access_token: &SecretString,
    ) -> Result<Mandate, GoCardlessError> {
        let resp: MandateEnvelope = self
            .get(
                &format!("/mandates/{id}"),
                access_token,
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.mandates)
    }
}

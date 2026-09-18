use crate::amount::Amount;
use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// No status field: a reversal sets `reversedAt`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MollieChargeback {
    pub id: String,
    pub amount: Amount,
    pub payment_id: Option<String>,
    #[serde(default)]
    pub reason: Option<ChargebackReason>,
    pub created_at: Option<String>,
    pub reversed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChargebackReason {
    pub code: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChargebackList {
    #[serde(default)]
    pub count: u32,
    #[serde(rename = "_embedded", default)]
    pub embedded: Option<ChargebackListEmbedded>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChargebackListEmbedded {
    #[serde(default)]
    pub chargebacks: Vec<MollieChargeback>,
}

impl ChargebackList {
    pub fn into_chargebacks(self) -> Vec<MollieChargeback> {
        self.embedded.map(|e| e.chargebacks).unwrap_or_default()
    }
}

impl MollieClient {
    pub async fn list_payment_chargebacks(
        &self,
        payment_id: &str,
        api_key: &SecretString,
    ) -> Result<ChargebackList, MollieError> {
        self.get(
            &format!("/payments/{payment_id}/chargebacks"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }
}

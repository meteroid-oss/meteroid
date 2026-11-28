//! What a payment cost in fees: a PaymentIntent's latest charge and its balance transaction, in
//! the settlement currency. Read off the payment path, after settlement.
use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// A field Stripe returns as an id, or as the object when asked to `expand[]` it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StripeExpandable<T> {
    Id(String),
    Object(Box<T>),
}

impl<T> StripeExpandable<T> {
    pub fn object(&self) -> Option<&T> {
        match self {
            StripeExpandable::Object(object) => Some(object),
            StripeExpandable::Id(_) => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StripeFeeDetail {
    pub amount: i64,
    pub currency: String,
    /// `stripe_fee`, `tax` or `application_fee`.
    #[serde(rename = "type")]
    pub fee_type: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Amounts in the minor unit of `currency`, the account's settlement currency.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StripeBalanceTransaction {
    pub id: String,
    pub amount: i64,
    pub fee: i64,
    pub net: i64,
    pub currency: String,
    #[serde(default)]
    pub fee_details: Vec<StripeFeeDetail>,
    /// Charge currency to settlement currency; none when they are the same.
    #[serde(default)]
    pub exchange_rate: Option<f64>,
    #[serde(default)]
    pub status: Option<String>,
    /// Unix seconds: when the charge entered the balance, the fee's day.
    #[serde(default)]
    pub created: Option<i64>,
    /// Unix seconds: when the funds become available.
    #[serde(default)]
    pub available_on: Option<i64>,
    #[serde(rename = "type", default)]
    pub transaction_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StripeCharge {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    /// None until the funds settle on an asynchronous method (SEPA, ACH).
    #[serde(default)]
    pub balance_transaction: Option<StripeExpandable<StripeBalanceTransaction>>,
}

impl StripeCharge {
    pub fn settled_balance_transaction(&self) -> Option<&StripeBalanceTransaction> {
        self.balance_transaction.as_ref()?.object()
    }
}

/// Only what the fee read needs of a PaymentIntent.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StripePaymentIntentCharge {
    pub id: String,
    #[serde(default)]
    pub latest_charge: Option<StripeExpandable<StripeCharge>>,
}

impl StripePaymentIntentCharge {
    pub fn latest_charge_id(&self) -> Option<&str> {
        match self.latest_charge.as_ref()? {
            StripeExpandable::Id(id) => Some(id),
            StripeExpandable::Object(charge) => Some(&charge.id),
        }
    }

    pub fn balance_transaction(&self) -> Option<&StripeBalanceTransaction> {
        self.latest_charge
            .as_ref()?
            .object()?
            .settled_balance_transaction()
    }
}

#[async_trait::async_trait]
pub trait ChargeApi {
    /// The intent with its latest charge and that charge's balance transaction, in one call.
    async fn retrieve_payment_intent_charge(
        &self,
        payment_intent_id: &str,
        secret_key: &SecretString,
    ) -> Result<StripePaymentIntentCharge, StripeError>;
}

#[async_trait::async_trait]
impl ChargeApi for StripeClient {
    async fn retrieve_payment_intent_charge(
        &self,
        payment_intent_id: &str,
        secret_key: &SecretString,
    ) -> Result<StripePaymentIntentCharge, StripeError> {
        self.get_query(
            &format!("/payment_intents/{payment_intent_id}"),
            &[("expand[]", "latest_charge.balance_transaction")],
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expanded_charge_reads_its_fee_in_the_settlement_currency() {
        let intent: StripePaymentIntentCharge = serde_json::from_str(
            r#"{"id":"pi_1","object":"payment_intent","latest_charge":{"id":"ch_1","amount":10000,
            "currency":"eur","balance_transaction":{"id":"txn_1","amount":10870,"fee":345,
            "net":10525,"currency":"usd","exchange_rate":1.087,"status":"pending",
            "created":1764201600,"available_on":1764633600,"type":"charge","fee_details":[{"amount":345,
            "currency":"usd","type":"stripe_fee","description":"Stripe processing fees"}]}}}"#,
        )
        .unwrap();
        let txn = intent.balance_transaction().expect("settled");
        assert_eq!((txn.fee, txn.currency.as_str()), (345, "usd"));
        assert_eq!(txn.exchange_rate, Some(1.087));
        assert_eq!(
            (txn.created, txn.available_on),
            (Some(1764201600), Some(1764633600))
        );
        assert_eq!(txn.fee_details[0].fee_type, "stripe_fee");
        assert_eq!(intent.latest_charge_id(), Some("ch_1"));
    }

    #[test]
    fn a_charge_as_an_id_or_without_its_balance_transaction_parses_unsettled() {
        let id: StripePaymentIntentCharge =
            serde_json::from_str(r#"{"id":"pi_1","latest_charge":"ch_9"}"#).unwrap();
        assert_eq!(id.latest_charge_id(), Some("ch_9"));
        assert!(id.balance_transaction().is_none());
        let pending: StripePaymentIntentCharge = serde_json::from_str(
            r#"{"id":"pi_2","latest_charge":{"id":"ch_2","amount":500,"currency":"eur",
            "balance_transaction":null}}"#,
        )
        .unwrap();
        assert!(pending.balance_transaction().is_none());
        let none: StripePaymentIntentCharge =
            serde_json::from_str(r#"{"id":"pi_3","latest_charge":null}"#).unwrap();
        assert_eq!(none.latest_charge_id(), None);
    }
}

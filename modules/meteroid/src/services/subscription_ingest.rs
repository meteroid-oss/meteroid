use chrono::NaiveDate;
use common_domain::ids::{AliasOr, CustomerId, PlanId};
use meteroid_store::domain::PaymentMethodsConfig;
use meteroid_store::domain::enums::SubscriptionActivationCondition;
use serde::Deserialize;

use super::csv_ingest::{
    CsvString, optional_csv_string, optional_naive_date, optional_u16, optional_u32,
};

#[derive(Deserialize)]
pub struct NewSubscriptionCsv {
    #[serde(default, with = "optional_csv_string")]
    pub idempotency_key: Option<CsvString>,
    pub customer_id_or_alias: AliasOr<CustomerId>,
    pub plan_id: PlanId,
    #[serde(default, with = "optional_u32")]
    pub plan_version: Option<u32>,
    pub start_date: NaiveDate,
    pub activation_condition: ActivationConditionCsv,
    pub auto_advance_invoices: bool,
    #[serde(default, with = "optional_u16")]
    pub billing_day_anchor: Option<u16>,
    pub charge_automatically: bool,
    #[serde(default, with = "optional_naive_date")]
    pub end_date: Option<NaiveDate>,
    #[serde(default, with = "optional_u32")]
    pub net_terms: Option<u32>,
    #[serde(default)]
    pub payment_method: Option<PaymentMethodCsv>,
    #[serde(default, with = "optional_csv_string")]
    pub purchase_order: Option<CsvString>,
    pub skip_past_invoices: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivationConditionCsv {
    OnStartDate,
    OnCheckout,
    Manual,
}

impl From<ActivationConditionCsv> for SubscriptionActivationCondition {
    fn from(v: ActivationConditionCsv) -> Self {
        match v {
            ActivationConditionCsv::OnStartDate => SubscriptionActivationCondition::OnStart,
            ActivationConditionCsv::OnCheckout => SubscriptionActivationCondition::OnCheckout,
            ActivationConditionCsv::Manual => SubscriptionActivationCondition::Manual,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentMethodCsv {
    Online,
    BankTransfer,
    External,
}

impl From<PaymentMethodCsv> for PaymentMethodsConfig {
    fn from(v: PaymentMethodCsv) -> Self {
        match v {
            PaymentMethodCsv::Online => PaymentMethodsConfig::Online { config: None },
            PaymentMethodCsv::BankTransfer => {
                PaymentMethodsConfig::BankTransfer { account_id: None }
            }
            PaymentMethodCsv::External => PaymentMethodsConfig::External,
        }
    }
}

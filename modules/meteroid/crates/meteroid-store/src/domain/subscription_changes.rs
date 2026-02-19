use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_domain::ids::ProductId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangePreview {
    pub matched: Vec<MatchedComponent>,
    pub added: Vec<AddedComponent>,
    pub removed: Vec<RemovedComponent>,
    pub effective_date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedComponent {
    pub product_id: ProductId,
    pub current_name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
    pub new_name: String,
    pub new_fee: SubscriptionFee,
    pub new_period: SubscriptionFeeBillingPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedComponent {
    pub name: String,
    pub fee: SubscriptionFee,
    pub period: SubscriptionFeeBillingPeriod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedComponent {
    pub name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
}

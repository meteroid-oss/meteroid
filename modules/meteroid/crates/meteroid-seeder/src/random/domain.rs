use chrono::NaiveDate;
use meteroid_store::domain::PlanTrial;
use meteroid_store::domain::enums::PlanTypeEnum;

// pub enum PlanType {
//     Free,
//     Paid,
//     Custom,
// }
//
// impl From<>

#[derive(Clone)]
pub struct Tenant {
    pub name: String,
    pub slug: String,
    pub currency: String,
    pub country: String,
}

#[derive(Clone)]
pub struct PlanVersion {
    pub period_start_day: Option<i16>,
    pub currency: String,
    pub billing_cycles: Option<i32>,
    pub net_terms: i32,
    pub trial: Option<PlanTrial>,
}

// #[derive(Clone)]
// pub struct PlanComponent {
//     pub name: String,
//     pub fee: serde_json::Value,
// }
#[derive(Clone)]
pub struct Plan {
    pub name: String,
    pub weight: f64,
    pub code: String,
    pub description: Option<String>,
    pub plan_type: PlanTypeEnum,
    pub version_details: PlanVersion,
    pub components: Vec<meteroid_store::domain::PriceComponentNewInternal>,
    // for each price component parametrized, we can provide a growth curve
    //
    pub churn_rate: Option<f64>,
    // pub upgrade_rate: Option<f64>,
}

#[derive(Clone)]
pub struct CustomerBase {
    pub dataset_path: Option<String>,
    pub customer_count: Option<u64>,
    pub customer_growth_curve: Vec<f64>,
}

impl Default for CustomerBase {
    fn default() -> Self {
        CustomerBase {
            dataset_path: None,
            customer_count: Some(10),
            customer_growth_curve: vec![1.0],
        }
    }
}

#[derive(Clone)]
pub struct Randomness {
    pub seed: Option<u64>,
    pub randomness_factor: f64,
}

#[derive(Clone)]
pub struct Metric {
    pub name: String,
    pub code: String,
}

#[derive(Clone)]
pub struct Scenario {
    pub name: String,
    //
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    //
    pub plans: Vec<Plan>,
    pub tenant: Tenant,
    pub product_family: String,
    pub customer_base: CustomerBase,
    pub metrics: Vec<Metric>,
    pub randomness: Randomness,
}

// pub enum SubscriptionBillingType {
//     Advance,
//     Arrears,
// }

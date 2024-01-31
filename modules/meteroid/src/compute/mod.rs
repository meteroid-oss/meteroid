mod adjustments;
pub mod fees;
pub mod period;

pub mod clients;
pub mod invoice_engine;

pub use invoice_engine::InvoiceEngine;
use meteroid_grpc::meteroid::api::billablemetrics::v1::BillableMetric;
use meteroid_grpc::meteroid::api::components::v1::{fee, price_component};
use meteroid_grpc::meteroid::api::schedules::v1::Schedule;
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionParameters;

#[derive(Debug)]
pub struct SubscriptionDetails {
    pub id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub customer_id: uuid::Uuid,
    pub customer_external_id: Option<String>,
    pub billing_start_date: chrono::NaiveDate,
    pub billing_end_date: Option<chrono::NaiveDate>,
    pub billing_day: i16,
    // i16 ?
    pub effective_billing_period: BillingPeriod,
    pub invoice_date: chrono::NaiveDate,
    pub current_period_idx: i32,
    pub currency: String,
    pub net_terms: i32,
    pub parameters: SubscriptionParameters,
    pub schedule: Option<Schedule>,
    pub price_components: Vec<PriceComponent>,
}

#[derive(Debug)]
pub struct PriceComponent {
    pub id: String,
    pub name: String,
    pub fee: fee::Type,
    pub product_item: Option<price_component::ProductItem>,
    pub metric: Option<BillableMetric>,
}

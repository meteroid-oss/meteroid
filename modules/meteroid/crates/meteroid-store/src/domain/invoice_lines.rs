use common_domain::ids::{BillableMetricId, PriceComponentId, ProductId};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct LineItem {
    pub local_id: String,
    pub name: String,
    pub total: i64,
    // before discounts/minimum
    pub subtotal: i64,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>, // precision 8

    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate, // TODO check incl/excl

    pub sub_lines: Vec<SubLineItem>,

    pub is_prorated: bool,

    pub price_component_id: Option<PriceComponentId>, // local_id ?
    pub product_id: Option<ProductId>,
    pub metric_id: Option<BillableMetricId>,

    pub description: Option<String>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct SubLineItem {
    pub local_id: String,
    pub name: String,
    pub total: i64,
    pub quantity: Decimal,
    pub unit_price: Decimal, // precision 8
    pub attributes: Option<SubLineAttributes>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub enum SubLineAttributes {
    Package {
        raw_usage: Decimal,
    },
    Tiered {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Volume {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Matrix {
        dimension1_key: String,
        dimension1_value: String,
        dimension2_key: Option<String>,
        dimension2_value: Option<String>,
    },
}

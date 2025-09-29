use crate::domain::coupons::CouponDiscount;
use common_domain::ids::{
    AppliedCouponId, BillableMetricId, CouponId, PriceComponentId, ProductId, SubscriptionAddOnId,
    SubscriptionPriceComponentId,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::Zero;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(PartialEq, Eq, Debug, Deserialize, Serialize, Clone)]
pub struct CouponLineItem {
    pub coupon_id: CouponId,
    pub applied_coupon_id: AppliedCouponId,
    pub name: String,
    pub code: String,
    pub value: i64,
    pub discount: CouponDiscount,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct LineItem {
    pub local_id: String,
    pub name: String,

    #[serde(alias = "subtotal")]
    pub amount_subtotal: i64, // quantity * unit_price, before discounts and tax. Displayed on invoice
    #[serde(default = "Decimal::zero")]
    pub tax_rate: Decimal, // Displayed on invoice
    #[serde(default)]
    pub taxable_amount: i64, // amount_subtotal - any discount or credit applied. Not displayed
    #[serde(default)]
    pub tax_amount: i64, // Not displayed
    #[serde(alias = "total")]
    pub amount_total: i64, // taxable_amount + tax_amount. Not displayed

    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>, // precision 8

    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate, // TODO check incl/excl

    pub sub_lines: Vec<SubLineItem>,

    pub is_prorated: bool,
    // todo remove?
    pub price_component_id: Option<PriceComponentId>,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub product_id: Option<ProductId>,
    pub metric_id: Option<BillableMetricId>,

    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by_dimensions: Option<HashMap<String, String>>,
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

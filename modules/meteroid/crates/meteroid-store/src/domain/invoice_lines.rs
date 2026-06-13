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
pub struct TaxDetail {
    pub tax_rate: Decimal,
    pub tax_name: String,
    pub tax_amount: i64,
}

#[derive(PartialEq, Debug, Deserialize, Serialize, Eq, Clone)]
pub struct LineItem {
    // TODO id: LocalItemId & serde(alias = "local_id")
    pub local_id: String,
    pub name: String,

    #[serde(alias = "subtotal")]
    pub amount_subtotal: i64, // quantity * unit_price, before discounts and tax. Displayed on invoice
    #[serde(default = "Decimal::zero")]
    pub tax_rate: Decimal, // Displayed on invoice (computed from tax_details)
    #[serde(default)]
    pub taxable_amount: i64, // amount_subtotal - any discount or credit applied. Not displayed
    #[serde(default)]
    pub tax_amount: i64, // Not displayed (computed from tax_details)
    #[serde(alias = "total")]
    pub amount_total: i64, // taxable_amount + tax_amount. Not displayed

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tax_details: Vec<TaxDetail>,

    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>, // precision 8

    pub start_date: chrono::NaiveDate,
    // Stored as the exclusive upper bound of the billing window (the *next* period's
    // start, e.g. a January monthly line is `2021-01-01 .. 2021-02-01`). This matches
    // the half-open `[start, end)` model used throughout period/proration/usage math.
    // For customer-facing presentation use `display_end_date()` to get the inclusive
    // last covered day (`2021-01-31`).
    pub end_date: chrono::NaiveDate,

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

impl LineItem {
    /// Inclusive last day actually covered by this line, for customer-facing display.
    ///
    /// `end_date` is stored as the exclusive upper bound of the half-open billing
    /// window (the next period's start), so a January monthly line reads
    /// `2021-01-01 .. 2021-02-01` internally. On the invoice we want the inclusive
    /// French/EU convention "du 01/01 au 31/01", so we step back one day.
    ///
    /// One-time / point-in-time lines carry a degenerate window where `end_date`
    /// is not an exclusive bound (e.g. one-time charges set `start == end == invoice_date`,
    /// discount lines use `NaiveDate::MIN`). Stepping back there would make the line
    /// read backwards, so we only adjust when the window actually spans more than a
    /// day (`end_date > start_date`) — for everything else this is a no-op.
    pub fn display_end_date(&self) -> chrono::NaiveDate {
        if self.end_date > self.start_date {
            self.end_date
                .pred_opt()
                .unwrap_or(self.end_date)
        } else {
            self.end_date
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn line(start: NaiveDate, end: NaiveDate) -> LineItem {
        LineItem {
            local_id: "l".into(),
            name: "n".into(),
            amount_subtotal: 0,
            tax_rate: Decimal::zero(),
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: start,
            end_date: end,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn display_end_date_makes_recurring_period_inclusive() {
        // Half-open Jan window 2021-01-01..2021-02-01 displays as the inclusive 2021-01-31.
        let l = line(d("2021-01-01"), d("2021-02-01"));
        assert_eq!(l.display_end_date(), d("2021-01-31"));
    }

    #[test]
    fn display_end_date_one_day_period_collapses_to_start() {
        // A single-day prorated window (start..start+1) reads as that single inclusive day.
        let l = line(d("2021-01-01"), d("2021-01-02"));
        assert_eq!(l.display_end_date(), d("2021-01-01"));
    }

    #[test]
    fn display_end_date_one_time_line_is_unchanged() {
        // One-time charges set start == end == invoice_date; stepping back would read backwards.
        let l = line(d("2021-03-10"), d("2021-03-10"));
        assert_eq!(l.display_end_date(), d("2021-03-10"));
    }

    #[test]
    fn display_end_date_degenerate_min_date_is_unchanged() {
        // Discount lines use NaiveDate::MIN for both bounds; never step below MIN.
        let l = line(NaiveDate::MIN, NaiveDate::MIN);
        assert_eq!(l.display_end_date(), NaiveDate::MIN);
    }
}

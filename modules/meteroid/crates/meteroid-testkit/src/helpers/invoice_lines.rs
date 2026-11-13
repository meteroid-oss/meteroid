use meteroid_store::domain::{LineItem, LineItemKind};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::data::ids::METRIC_DATABASE_SIZE;

pub fn line(name: &str, cents: i64, metric: bool) -> LineItem {
    let today = common_domain::clock::now().date_naive();
    LineItem {
        local_id: Uuid::new_v4().to_string(),
        name: name.to_owned(),
        charge_type: None,
        amount_subtotal: cents,
        tax_rate: Decimal::ZERO,
        taxable_amount: cents,
        tax_amount: 0,
        amount_total: cents,
        tax_details: Vec::new(),
        quantity: None,
        unit_price: None,
        start_date: today,
        end_date: today.succ_opt().unwrap(),
        sub_lines: Vec::new(),
        is_prorated: false,
        price_component_id: None,
        sub_component_id: None,
        sub_add_on_id: None,
        product_id: None,
        metric_id: metric.then_some(METRIC_DATABASE_SIZE),
        description: None,
        group_by_dimensions: None,
        kind: LineItemKind::Regular,
        credit_grant_amount: None,
        credit_grant_expiry: None,
        credit_grant_included: false,
        taxed_at_purchase: false,
        tax_inclusive_quote: None,
        credit_unit: None,
        credit_grant_unit: None,
        credit_grant_rollover: None,
        promotional_credit: None,
        subscription_id: None,
        usage_ends_at: None,
    }
}

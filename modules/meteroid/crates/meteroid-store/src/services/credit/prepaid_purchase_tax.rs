//! Currency credit is a prepayment: the usage it later pays is taxed on its own invoice, so the
//! purchase itself carries no tax. A credit unit is no money prepayment: it is taxed when sold.
use crate::domain::LineItem;

/// Whether the tax pass leaves this line untaxed because its tax is due at redemption. The rule
/// lives on the line (`LineItem::credit_taxed_at_redemption`), shared with documents and analytics.
pub(in crate::services) fn taxed_at_redemption(line: &LineItem) -> bool {
    line.credit_taxed_at_redemption()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LineItemKind;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn credit_line() -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        LineItem {
            local_id: "c".to_owned(),
            name: "Purchase credits".to_owned(),
            charge_type: None,
            amount_subtotal: 50_00,
            tax_rate: Decimal::ZERO,
            taxable_amount: 50_00,
            tax_amount: 0,
            amount_total: 50_00,
            tax_details: Vec::new(),
            quantity: Some(Decimal::ONE),
            unit_price: Some(Decimal::new(50_00, 2)),
            start_date: day,
            end_date: day,
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: LineItemKind::Credit,
            credit_grant_amount: Some(50_00),
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

    #[test]
    fn a_currency_prepayment_is_taxed_at_redemption() {
        assert!(taxed_at_redemption(&credit_line()));
    }

    #[test]
    fn credit_taxed_when_sold_is_not_taxed_again_at_redemption() {
        let taxed = LineItem {
            taxed_at_purchase: true,
            ..credit_line()
        };
        assert!(taxed.credit_taxed_at_purchase());
        assert!(!taxed_at_redemption(&taxed));
    }

    #[test]
    fn included_credit_is_never_taxed() {
        let included = LineItem {
            taxed_at_purchase: true,
            credit_grant_included: true,
            ..credit_line()
        };
        assert!(taxed_at_redemption(&included));
        let regular = LineItem {
            kind: LineItemKind::Regular,
            ..credit_line()
        };
        assert!(!taxed_at_redemption(&regular));
    }
}

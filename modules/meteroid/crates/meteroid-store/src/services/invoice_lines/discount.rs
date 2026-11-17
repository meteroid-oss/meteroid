use std::collections::HashMap;

use chrono::NaiveDate;
use common_domain::ids::{AppliedCouponId, InvoiceId};
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;

use crate::StoreResult;
use crate::domain::coupons::{AppliedCouponsDiscount, CouponDiscount};
use crate::domain::{
    AppliedCouponDetailed, CouponLineItem, Invoice, LineItem, LineItemKind, SubscriptionDetails,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use billing_core::assembly::discount as core;

pub fn distribute_discount(line_items: Vec<LineItem>, discount: u64) -> Vec<LineItem> {
    let subtotals: Vec<i64> = line_items.iter().map(|item| item.amount_subtotal).collect();
    let taxable = core::distribute_discount(&subtotals, discount);
    line_items
        .into_iter()
        .zip(taxable)
        .map(|(mut item, taxable)| {
            if let Some(taxable) = taxable {
                item.taxable_amount = taxable;
            }
            item
        })
        .collect()
}

/// A coupon as `billing_core` reads it; a fixed amount in its currency's exponent (KW-02).
fn to_core(coupon: &AppliedCouponDetailed) -> core::Coupon {
    core::Coupon {
        coupon_id: coupon.coupon.id,
        value: match &coupon.coupon.discount {
            CouponDiscount::Percentage(percentage) => core::CouponValue::Percentage(*percentage),
            CouponDiscount::Fixed { amount, currency } => core::CouponValue::Fixed {
                amount: *amount,
                currency: currency.clone(),
                // todo domain should use Currency type instead of string
                exponent: meteroid_money::iso::find(currency)
                    .unwrap_or(meteroid_money::iso::USD)
                    .exponent as u8,
            },
            CouponDiscount::Credit { .. } => core::CouponValue::Credit,
        },
    }
}

/// The coupons that apply to the invoice being computed (R-089).
pub fn applicable_coupons(coupons: &[AppliedCouponDetailed]) -> Vec<&AppliedCouponDetailed> {
    coupons
        .iter()
        .filter(|x| x.is_invoice_applicable())
        .collect()
}

/// `applicable` as `billing_core` reads them.
pub fn coupon_inputs(applicable: &[&AppliedCouponDetailed]) -> Vec<core::Coupon> {
    applicable.iter().map(|c| to_core(c)).collect()
}

/// The coupon rows of what `applied` took off, `applied` indexing `applicable`.
pub fn coupon_rows(
    applicable: &[&AppliedCouponDetailed],
    applied: &[core::CouponDiscount],
) -> Vec<CouponLineItem> {
    applied
        .iter()
        .map(|applied| {
            let applicable_coupon = applicable[applied.coupon];
            CouponLineItem {
                coupon_id: applicable_coupon.coupon.id,
                applied_coupon_id: applicable_coupon.applied_coupon.id,
                name: format!("Coupon ({})", applicable_coupon.coupon.code), // TODO allow defining a name in coupon
                code: applicable_coupon.coupon.code.clone(),
                value: applied.value,
                quoted_value: None,
                discount: applicable_coupon.coupon.discount.clone(),
            }
        })
        .collect()
}

pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[AppliedCouponDetailed],
) -> AppliedCouponsDiscount {
    let applicable_coupons = applicable_coupons(coupons);
    let inputs = coupon_inputs(&applicable_coupons);
    let applied_coupons_items = coupon_rows(
        &applicable_coupons,
        &core::calculate_coupons_discount(subtotal, invoice_currency, &inputs),
    );

    AppliedCouponsDiscount {
        discount_subunit: applied_coupons_items.iter().map(|x| x.value).sum(),
        applied_coupons: applied_coupons_items,
    }
}

/// A subscription's open period invoices in period order, each with what its coupons discount,
/// and the place among them of the invoice being computed: a coupon's limited uses go to the
/// oldest period first, whatever order the drafts are issued in (R-089).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    /// `None` is the invoice being computed.
    entries: Vec<(Option<InvoiceId>, i64)>,
}

impl Queue {
    /// The subscription's recurring and final drafts but `invoice` (a consolidated one's are
    /// its parent's), oldest period first; `invoice`, or a new one of `period_start`, in its place.
    pub(in crate::services) async fn load(
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        invoice: Option<InvoiceId>,
        period_start: NaiveDate,
    ) -> StoreResult<Self> {
        let sub = &details.subscription;
        let rows = InvoiceRow::drafts_of_subscription(conn, sub.tenant_id, sub.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mut entries: Vec<(Option<InvoiceId>, i64)> = Vec::with_capacity(rows.len() + 1);
        let mut place: Option<usize> = None;
        let mut later: Option<usize> = None;
        for row in rows {
            let draft: Invoice = row.try_into()?;
            if Some(draft.id) == invoice {
                place = Some(entries.len());
                continue;
            }
            if !draft.invoice_type.bills_subscription_usage() || draft.is_consolidated_child() {
                continue;
            }
            if later.is_none() && draft.billing_period_start > period_start {
                later = Some(entries.len());
            }
            entries.push((Some(draft.id), discounted(&draft.line_items)));
        }
        let at = place.or(later).unwrap_or(entries.len());
        entries.insert(at, (None, 0));
        Ok(Self { entries })
    }

    /// `coupons` as the invoice being computed is owed them: each limited one's uses the drafts
    /// of earlier periods take counted as taken.
    pub fn owed(
        &self,
        coupons: &[AppliedCouponDetailed],
        currency: &str,
    ) -> Vec<AppliedCouponDetailed> {
        let (inputs, mut uses) = uses_of(coupons);
        let mut taken = vec![0; coupons.len()];
        for (id, subtotal) in &self.entries {
            if id.is_none() {
                break;
            }
            for i in uses.take(&inputs, *subtotal, currency) {
                taken[i] += 1;
            }
        }
        coupons
            .iter()
            .zip(taken)
            .map(|(coupon, taken)| {
                let mut coupon = coupon.clone();
                if taken > 0 {
                    let applied = &mut coupon.applied_coupon;
                    applied.applied_count = Some(applied.applied_count.unwrap_or(0) + taken);
                }
                coupon
            })
            .collect()
    }

    /// The coupons each draft takes once issued, the invoice being computed taking `consumed`.
    pub fn taken(
        &self,
        coupons: &[AppliedCouponDetailed],
        currency: &str,
        consumed: &[AppliedCouponId],
    ) -> HashMap<InvoiceId, Vec<AppliedCouponId>> {
        let (inputs, mut uses) = uses_of(coupons);
        let mut taken = HashMap::new();
        for (id, subtotal) in &self.entries {
            match id {
                Some(id) => {
                    let ids = uses
                        .take(&inputs, *subtotal, currency)
                        .into_iter()
                        .map(|i| coupons[i].applied_coupon.id)
                        .collect();
                    taken.insert(*id, ids);
                }
                None => uses.took(
                    &(0..coupons.len())
                        .filter(|i| consumed.contains(&coupons[*i].applied_coupon.id))
                        .collect::<Vec<_>>(),
                ),
            }
        }
        taken
    }
}

/// Whether `invoice`, a draft kept as it stands, shows the coupons its period is owed now: a
/// use another draft was owed, or one a void freed, makes it computed again (R-089).
pub(in crate::services) async fn shows_owed_coupons(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice: &Invoice,
) -> StoreResult<bool> {
    if !invoice.invoice_type.bills_subscription_usage() {
        return Ok(true);
    }
    let currency = &details.subscription.currency;
    let owed = Queue::load(
        conn,
        details,
        Some(invoice.id),
        invoice.billing_period_start,
    )
    .await?
    .owed(&details.applied_coupons, currency);
    let applicable = applicable_coupons(&owed);
    let takes: Vec<AppliedCouponId> = core::calculate_coupons_discount(
        discounted(&invoice.line_items),
        currency,
        &coupon_inputs(&applicable),
    )
    .iter()
    .map(|applied| applicable[applied.coupon].applied_coupon.id)
    .collect();
    Ok(takes.len() == invoice.coupons.len()
        && invoice
            .coupons
            .iter()
            .all(|coupon| takes.contains(&coupon.applied_coupon_id)))
}

/// What a draft's coupons discount: its lines but a true-up, which no coupon discounts (R-023).
fn discounted(lines: &[LineItem]) -> i64 {
    lines
        .iter()
        .filter(|line| line.kind != LineItemKind::MinimumShortfall)
        .map(|line| line.amount_subtotal)
        .sum()
}

/// `coupons` as `billing_core` hands out their uses: what each limited one has left, none for
/// one no invoice takes.
fn uses_of(coupons: &[AppliedCouponDetailed]) -> (Vec<core::Coupon>, core::Uses) {
    let left = coupons
        .iter()
        .map(|coupon| {
            if !coupon.is_invoice_applicable() {
                return Some(0);
            }
            coupon.coupon.recurring_value.map(|n| {
                u32::try_from(n - coupon.applied_coupon.applied_count.unwrap_or(0)).unwrap_or(0)
            })
        })
        .collect();
    (coupons.iter().map(to_core).collect(), core::Uses::new(left))
}

/// What `coupons` will take off `subtotal` on the invoice that follows one which consumed
/// `consumed`. Counts the application finalization records (see `refresh_applied_coupons`) so a
/// coupon that has reached its cycle limit is accounted for.
pub fn calculate_next_invoice_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[AppliedCouponDetailed],
    consumed: &[CouponLineItem],
) -> i64 {
    let next: Vec<AppliedCouponDetailed> = coupons
        .iter()
        .cloned()
        .map(|mut c| {
            if consumed
                .iter()
                .any(|l| l.applied_coupon_id == c.applied_coupon.id)
            {
                let applied = &mut c.applied_coupon;
                applied.applied_count = Some(applied.applied_count.unwrap_or(0) + 1);
            }
            c
        })
        .collect();

    calculate_coupons_discount(subtotal, invoice_currency, &next)
        .discount_subunit
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coupons::Coupon;
    use crate::domain::subscription_coupons::AppliedCoupon;
    use common_domain::ids::BaseId;
    use common_domain::ids::{AppliedCouponId, CouponId, CustomerId, SubscriptionId, TenantId};
    use rust_decimal::Decimal;

    fn applied_coupon(code: &str, discount: CouponDiscount) -> AppliedCouponDetailed {
        let now = common_clock::now().naive_utc();
        let coupon_id = CouponId::new();
        AppliedCouponDetailed {
            coupon: Coupon {
                id: coupon_id,
                code: code.to_string(),
                description: String::new(),
                tenant_id: TenantId::new(),
                discount,
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: true,
                disabled: false,
                created_at: now,
                updated_at: now,
                last_redemption_at: None,
                archived_at: None,
                redemption_count: 0,
                plan_ids: vec![],
            },
            applied_coupon: AppliedCoupon {
                id: AppliedCouponId::new(),
                coupon_id,
                customer_id: CustomerId::new(),
                subscription_id: SubscriptionId::new(),
                is_active: true,
                applied_amount: None,
                applied_count: Some(0),
                last_applied_at: None,
                created_at: now,
            },
        }
    }

    #[test]
    fn a_coupon_takes_the_last_minor_unit() {
        let full = applied_coupon("FULL", CouponDiscount::Percentage(Decimal::from(100)));
        let fixed = applied_coupon(
            "FIXED",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::new(5, 2),
            },
        );

        assert_eq!(
            calculate_coupons_discount(1, "USD", &[full]).discount_subunit,
            1
        );
        assert_eq!(
            calculate_coupons_discount(1, "USD", &[fixed]).discount_subunit,
            1
        );
    }

    /// A 10 % coupon for two invoices, January's and February's drafts open: March is owed none;
    /// February, issued first, the second, leaving January the first and March nothing.
    /// Rules: R-089, R-022
    #[test]
    fn an_invoice_is_owed_a_use_only_after_the_open_drafts_of_earlier_periods() {
        let mut ten = applied_coupon("TEN", CouponDiscount::Percentage(Decimal::from(10)));
        ten.coupon.recurring_value = Some(2);
        let coupons = std::slice::from_ref(&ten);
        let (january, february, march) = (InvoiceId::new(), InvoiceId::new(), InvoiceId::new());
        let last = Queue {
            entries: vec![(Some(january), 2_500), (Some(february), 49_597), (None, 0)],
        };
        assert!(applicable_coupons(&last.owed(coupons, "USD")).is_empty());

        let closing = Queue {
            entries: vec![(Some(january), 2_500), (None, 0), (Some(march), 50_000)],
        };
        assert_eq!(applicable_coupons(&closing.owed(coupons, "USD")).len(), 1);
        let taken = closing.taken(coupons, "USD", &[ten.applied_coupon.id]);
        assert_eq!(taken[&january], vec![ten.applied_coupon.id]);
        assert_eq!(taken[&march], Vec::<AppliedCouponId>::new());

        // January's draft bills nothing yet: it takes no use, so the second is March's.
        let empty = Queue {
            entries: vec![(Some(january), 0), (None, 0), (Some(march), 50_000)],
        };
        let taken = empty.taken(coupons, "USD", &[ten.applied_coupon.id]);
        assert_eq!(taken[&march], vec![ten.applied_coupon.id]);
    }

    #[test]
    fn a_credit_coupon_discounts_nothing() {
        let credit = applied_coupon(
            "WELCOME",
            CouponDiscount::Credit {
                currency: "USD".to_string(),
                amount: Decimal::from(25),
                expires_in_days: None,
                scope: Default::default(),
            },
        );
        let result = calculate_coupons_discount(10_000, "USD", &[credit]);
        assert_eq!(result.discount_subunit, 0);
        assert!(result.applied_coupons.is_empty());
    }

    #[test]
    fn next_invoice_discount_ignores_coupons_unused_today() {
        // Nothing billed today (usage in arrears): the coupon is untouched and fully applies to
        // the next invoice's 50.00 floor.
        let pct = applied_coupon("PCT", CouponDiscount::Percentage(Decimal::from(10)));
        assert_eq!(
            calculate_next_invoice_coupons_discount(5_000, "USD", &[pct], &[]),
            500
        );
    }

    #[test]
    fn next_invoice_discount_drops_a_coupon_spent_today() {
        // A one-cycle coupon consumed on today's invoice no longer applies next time.
        let mut once = applied_coupon("ONCE", CouponDiscount::Percentage(Decimal::from(10)));
        once.coupon.recurring_value = Some(1);
        let today = calculate_coupons_discount(2_000, "USD", std::slice::from_ref(&once));
        assert_eq!(today.discount_subunit, 200);

        assert_eq!(
            calculate_next_invoice_coupons_discount(5_000, "USD", &[once], &today.applied_coupons),
            0
        );
    }

    fn new_line_item(amount_subtotal: i64) -> LineItem {
        LineItem {
            local_id: "test".to_string(),
            name: "test".to_string(),
            charge_type: None,
            amount_subtotal,
            taxable_amount: amount_subtotal,
            tax_amount: 0,
            tax_rate: Decimal::ZERO,
            quantity: None,
            unit_price: None,
            start_date: chrono::NaiveDate::MIN,
            end_date: chrono::NaiveDate::MIN,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            amount_total: amount_subtotal,
            group_by_dimensions: None,
            tax_details: vec![],
            kind: crate::domain::LineItemKind::Regular,
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
            usage_breakdown: None,
        }
    }

    #[test]
    fn test_simple_distribution() {
        let items = vec![
            new_line_item(6000), // 60€
            new_line_item(4000), // 40€
        ];

        let items = distribute_discount(items, 1000); // 10€ so 10%

        assert_eq!(items[0].taxable_amount, 5400); // 60€ - 6€
        assert_eq!(items[1].taxable_amount, 3600); // 40€ - 4€
    }
    #[test]
    fn a_fixed_coupon_gives_its_whole_amount_on_each_invoice_it_lasts() {
        let mut fixed = applied_coupon(
            "TWICE",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(10),
            },
        );
        fixed.coupon.recurring_value = Some(2);

        let mut taken = vec![];
        for _ in 0..3 {
            let invoice = calculate_coupons_discount(3_000, "USD", std::slice::from_ref(&fixed));
            taken.push(invoice.discount_subunit);
            // What finalization records: one more invoice, and the amount given.
            if invoice.discount_subunit > 0 {
                let applied = &mut fixed.applied_coupon;
                applied.applied_count = Some(applied.applied_count.unwrap_or(0) + 1);
                applied.applied_amount = Some(
                    applied.applied_amount.unwrap_or_default()
                        + Decimal::new(invoice.discount_subunit, 2),
                );
            }
        }
        assert_eq!(taken, vec![1_000, 1_000, 0]);
    }

    #[test]
    fn next_invoice_discount_keeps_a_fixed_coupon_whole() {
        let mut fixed = applied_coupon(
            "TWICE",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(10),
            },
        );
        fixed.coupon.recurring_value = Some(2);
        let today = calculate_coupons_discount(3_000, "USD", std::slice::from_ref(&fixed));
        assert_eq!(today.discount_subunit, 1_000);

        assert_eq!(
            calculate_next_invoice_coupons_discount(
                3_000,
                "USD",
                std::slice::from_ref(&fixed),
                &today.applied_coupons
            ),
            1_000
        );
        fixed.applied_coupon.applied_count = Some(1);
        assert_eq!(
            calculate_next_invoice_coupons_discount(3_000, "USD", &[fixed], &today.applied_coupons),
            0
        );
    }
}

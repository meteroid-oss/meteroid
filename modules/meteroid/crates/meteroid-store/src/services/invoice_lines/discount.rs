use std::collections::HashMap;

use chrono::NaiveDate;
use common_domain::ids::{AddOnId, AppliedCouponId, InvoiceId, PlanId, SubscriptionAddOnId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
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

/// A coupon as `billing_core` reads it on an invoice in `currency` of a subscription to `plan`;
/// a fixed amount in its amount there, at its currency's exponent (KW-02).
fn to_core(coupon: &AppliedCouponDetailed, currency: &str, plan: Option<PlanId>) -> core::Coupon {
    core::Coupon {
        coupon_id: coupon.coupon.id,
        add_ons: coupon
            .coupon
            .discounted_add_ons(plan)
            .map(<[AddOnId]>::to_vec),
        until: coupon.lasts_until(),
        from: coupon.applied_coupon.discounts_from,
        value: match &coupon.coupon.discount_in(currency) {
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

/// `applicable` as `billing_core` reads them on an invoice in `currency` of a subscription to
/// `plan`.
pub fn coupon_inputs(
    applicable: &[&AppliedCouponDetailed],
    currency: &str,
    plan: Option<PlanId>,
) -> Vec<core::Coupon> {
    applicable
        .iter()
        .map(|c| to_core(c, currency, plan))
        .collect()
}

/// The add-on each of a subscription's add-on rows bills, the ended ones too: an add-on ending
/// on a renewal still has its arrears and usage on the renewal's invoice.
pub type AddOnIds = HashMap<SubscriptionAddOnId, AddOnId>;

/// [`AddOnIds`] of the subscription of `details`: its rows, and the add-ons `details` holds (a
/// preview's are in no row yet).
pub(in crate::services) async fn load_add_on_ids(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<AddOnIds> {
    let sub = &details.subscription;
    let rows = SubscriptionAddOnRow::list_all_by_subscriptions(conn, sub.tenant_id, &[sub.id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(details
        .add_ons
        .iter()
        .map(|add_on| (add_on.id, add_on.add_on_id))
        .chain(rows.into_iter().map(|row| (row.id, row.add_on_id)))
        .collect())
}

/// The add-on each of `lines` bills, among the subscription's: what a coupon discounting only
/// some add-ons reads of them, each invoice afresh.
pub fn add_ons_of<'a>(
    lines: impl IntoIterator<Item = &'a LineItem>,
    add_on_ids: &AddOnIds,
) -> Vec<Option<AddOnId>> {
    lines
        .into_iter()
        .map(|line| add_on_ids.get(&line.sub_add_on_id?).copied())
        .collect()
}

/// The coupon rows of what `applied` took off, `applied` indexing `applicable`.
pub fn coupon_rows(
    applicable: &[&AppliedCouponDetailed],
    applied: &[core::CouponDiscount],
    currency: &str,
) -> Vec<CouponLineItem> {
    applied
        .iter()
        .map(|applied| {
            let applicable_coupon = applicable[applied.coupon];
            CouponLineItem {
                coupon_id: applicable_coupon.coupon.id,
                applied_coupon_id: applicable_coupon.applied_coupon.id,
                name: if applicable_coupon.coupon.is_grant_definition() {
                    applicable_coupon.coupon.display_label().to_string()
                } else {
                    format!("Coupon ({})", applicable_coupon.coupon.code)
                },
                code: applicable_coupon.coupon.code.clone(),
                value: applied.value,
                quoted_value: None,
                discount: applicable_coupon.coupon.discount_in(currency),
            }
        })
        .collect()
}

/// What `coupons` take off `subtotal`, which bills no add-on, of a subscription to `plan`.
pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    plan: Option<PlanId>,
    coupons: &[AppliedCouponDetailed],
) -> AppliedCouponsDiscount {
    let applicable_coupons = applicable_coupons(coupons);
    let inputs = coupon_inputs(&applicable_coupons, invoice_currency, plan);
    let applied_coupons_items = coupon_rows(
        &applicable_coupons,
        &core::calculate_coupons_discount(subtotal, invoice_currency, &inputs),
        invoice_currency,
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
    /// `None` is the invoice being computed; each with the day its period starts, which orders
    /// them (a coupon lasting months reads its lines' own periods).
    entries: Vec<(Option<InvoiceId>, NaiveDate, Vec<core::Part>)>,
    /// The subscription's plan, which a coupon's scope reads.
    plan: Option<PlanId>,
    /// The add-on each line of the subscription bills, which a coupon's scope reads.
    add_on_ids: AddOnIds,
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
        let add_on_ids = load_add_on_ids(conn, details).await?;
        let rows = InvoiceRow::drafts_of_subscription(conn, sub.tenant_id, sub.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mut entries: Vec<(Option<InvoiceId>, NaiveDate, Vec<core::Part>)> =
            Vec::with_capacity(rows.len() + 1);
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
            entries.push((
                Some(draft.id),
                draft.billing_period_start,
                parts(&draft.line_items, &add_on_ids),
            ));
        }
        let at = place.or(later).unwrap_or(entries.len());
        entries.insert(at, (None, period_start, Vec::new()));
        Ok(Self {
            entries,
            plan: Some(sub.plan_id),
            add_on_ids,
        })
    }

    /// The add-on each of `lines` bills (see [`add_ons_of`]).
    pub fn add_ons_of<'a>(
        &self,
        lines: impl IntoIterator<Item = &'a LineItem>,
    ) -> Vec<Option<AddOnId>> {
        add_ons_of(lines, &self.add_on_ids)
    }

    /// `applicable` as `billing_core` reads them on an invoice of the subscription.
    pub fn inputs(
        &self,
        applicable: &[&AppliedCouponDetailed],
        currency: &str,
    ) -> Vec<core::Coupon> {
        coupon_inputs(applicable, currency, self.plan)
    }

    /// `coupons` as the invoice being computed is owed them: each limited one's uses the drafts
    /// of earlier periods take counted as taken. One lasting months discounts only the lines of
    /// service periods within them, whatever the invoice's date.
    pub fn owed(
        &self,
        coupons: &[AppliedCouponDetailed],
        currency: &str,
    ) -> Vec<AppliedCouponDetailed> {
        let (inputs, mut uses) = uses_of(coupons, currency, self.plan);
        let mut taken = vec![0; coupons.len()];
        for (id, _, parts) in &self.entries {
            if id.is_none() {
                break;
            }
            for i in uses.take(&inputs, parts, currency) {
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
        let (inputs, mut uses) = uses_of(coupons, currency, self.plan);
        let mut taken = HashMap::new();
        for (id, _, parts) in &self.entries {
            match id {
                Some(id) => {
                    let ids = uses
                        .take(&inputs, parts, currency)
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
    let queue = Queue::load(
        conn,
        details,
        Some(invoice.id),
        invoice.billing_period_start,
    )
    .await?;
    let owed = queue.owed(&details.applied_coupons, currency);
    let applicable = applicable_coupons(&owed);
    let takes: Vec<AppliedCouponId> = core::discount_parts(
        &parts(&invoice.line_items, &queue.add_on_ids),
        currency,
        &queue.inputs(&applicable, currency),
    )
    .0
    .iter()
    .map(|applied| applicable[applied.coupon].applied_coupon.id)
    .collect();
    Ok(takes.len() == invoice.coupons.len()
        && invoice
            .coupons
            .iter()
            .all(|coupon| takes.contains(&coupon.applied_coupon_id)))
}

/// What a draft's coupons discount: its lines but a true-up, which no coupon discounts (R-023),
/// each with the service period it charges, which a coupon lasting months reads.
fn parts(lines: &[LineItem], add_on_ids: &AddOnIds) -> Vec<core::Part> {
    let discounted: Vec<&LineItem> = lines
        .iter()
        .filter(|line| line.kind != LineItemKind::MinimumShortfall)
        .collect();
    let add_ons = add_ons_of(discounted.iter().copied(), add_on_ids);
    discounted
        .iter()
        .zip(add_ons)
        .map(|(line, add_on)| core::Part {
            subtotal: line.amount_subtotal,
            add_on,
            start: Some(line.start_date),
        })
        .collect()
}

/// `coupons` as `billing_core` hands out their uses: what each limited one has left, none for
/// one no invoice takes.
pub(in crate::services) fn uses_of(
    coupons: &[AppliedCouponDetailed],
    currency: &str,
    plan: Option<PlanId>,
) -> (Vec<core::Coupon>, core::Uses) {
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
    (
        coupons.iter().map(|c| to_core(c, currency, plan)).collect(),
        core::Uses::new(left),
    )
}

/// What `coupons` will take off `subtotal`, which bills no add-on, on the invoice that follows
/// one which consumed `consumed`, its period starting on `next_period_start`. Counts the
/// application finalization records (see `refresh_applied_coupons`) so a coupon that has reached
/// its cycle limit is accounted for, and drops one whose months end before (R-089).
pub fn calculate_next_invoice_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    plan: Option<PlanId>,
    coupons: &[AppliedCouponDetailed],
    consumed: &[CouponLineItem],
    next_period_start: Option<NaiveDate>,
) -> i64 {
    let next: Vec<AppliedCouponDetailed> = coupons
        .iter()
        .filter(|c| next_period_start.is_none_or(|start| c.covers(start)))
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

    calculate_coupons_discount(subtotal, invoice_currency, plan, &next)
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

    /// The first of `month` 2026; a month past December runs into the next years.
    fn day(month: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1)
            .and_then(|first| first.checked_add_months(chrono::Months::new(month - 1)))
            .expect("a day")
    }

    fn whole(subtotal: i64) -> Vec<core::Part> {
        vec![core::Part {
            subtotal,
            add_on: None,
            start: None,
        }]
    }

    /// One line of `subtotal` charging the service period starting on `start`.
    fn dated(start: NaiveDate, subtotal: i64) -> Vec<core::Part> {
        vec![core::Part {
            subtotal,
            add_on: None,
            start: Some(start),
        }]
    }

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
                duration_months: None,
                reusable: true,
                disabled: false,
                created_at: now,
                updated_at: now,
                last_redemption_at: None,
                archived_at: None,
                redemption_count: 0,
                plan_ids: vec![],
                add_on_ids: vec![],
                eligibility: None,
                valid_from: None,
                fixed_amounts: Default::default(),
                offer_id: None,
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
                effective_from: day(1),
                discounts_from: None,
            },
        }
    }

    /// The invoices of `periods` that `coupon` discounts, each issued in turn and charging the
    /// period starting on its date: one finalized counts a use, as `refresh_applied_coupons`
    /// records it.
    fn discounted(mut coupon: AppliedCouponDetailed, periods: &[NaiveDate]) -> usize {
        let mut discounted = 0;
        for start in periods {
            let queue = Queue {
                plan: None,
                add_on_ids: AddOnIds::new(),
                entries: vec![(None, *start, vec![])],
            };
            let owed = queue.owed(std::slice::from_ref(&coupon), "USD");
            let applicable = applicable_coupons(&owed);
            let applied = core::discount_parts(
                &dated(*start, 10_000),
                "USD",
                &queue.inputs(&applicable, "USD"),
            )
            .0;
            if !applied.is_empty() {
                discounted += 1;
                let used = &mut coupon.applied_coupon;
                used.applied_count = Some(used.applied_count.unwrap_or(0) + 1);
            }
        }
        discounted
    }

    /// Two years of invoices on a cadence of `months`, from the coupon's redemption.
    fn periods(months: u32) -> Vec<NaiveDate> {
        (0..24 / months).map(|n| day(1 + n * months)).collect()
    }

    /// "50% off for 12 months": twelve monthly invoices, one annual one; an invoice of a period
    /// starting on the twelfth month's end is past it.
    /// Rules: R-089
    #[test]
    fn a_coupon_lasting_months_discounts_the_periods_starting_within_them() {
        let mut year = applied_coupon("YC26", CouponDiscount::Percentage(Decimal::from(50)));
        year.coupon.duration_months = Some(12);
        assert_eq!(discounted(year.clone(), &periods(1)), 12);
        assert_eq!(discounted(year.clone(), &periods(12)), 1);
        assert_eq!(discounted(year.clone(), &periods(3)), 4);
        assert_eq!(year.lasts_until(), Some(day(13)));
    }

    /// "50% off for 12 months" on usage billed in arrears: each invoice, dated its period's end,
    /// is discounted while the period it charges starts within the months, twelve in all.
    /// Rules: R-089
    #[test]
    fn a_coupon_lasting_months_reads_the_charged_period_not_the_invoice_date() {
        let mut year = applied_coupon("YC26", CouponDiscount::Percentage(Decimal::from(50)));
        year.coupon.duration_months = Some(12);
        let discounted = (1..=24)
            .filter(|month| {
                let queue = Queue {
                    plan: None,
                    add_on_ids: AddOnIds::new(),
                    entries: vec![(None, day(month + 1), vec![])],
                };
                let owed = queue.owed(std::slice::from_ref(&year), "USD");
                let inputs = queue.inputs(&applicable_coupons(&owed), "USD");
                !core::discount_parts(&dated(day(*month), 10_000), "USD", &inputs)
                    .0
                    .is_empty()
            })
            .count();
        assert_eq!(discounted, 12);
    }

    /// A coupon made before durations in months still counts invoices: twelve annual ones.
    /// Rules: R-089
    #[test]
    fn a_coupon_counting_invoices_is_unchanged() {
        let mut twelve = applied_coupon("LEGACY12", CouponDiscount::Percentage(Decimal::from(10)));
        twelve.coupon.recurring_value = Some(12);
        assert_eq!(twelve.lasts_until(), None);
        assert_eq!(discounted(twelve.clone(), &periods(1)), 12);
        let decades: Vec<NaiveDate> = (0..15).map(|n| day(1 + n * 12)).collect();
        assert_eq!(discounted(twelve, &decades), 12);
    }

    /// Past its months, a coupon is owed to no draft either: a later period's takes nothing.
    /// Rules: R-089
    #[test]
    fn a_draft_past_a_coupons_months_takes_none_of_it() {
        let mut quarter = applied_coupon("Q", CouponDiscount::Percentage(Decimal::from(10)));
        quarter.coupon.duration_months = Some(3);
        let (march, april) = (InvoiceId::new(), InvoiceId::new());
        let queue = Queue {
            plan: None,
            add_on_ids: AddOnIds::new(),
            entries: vec![
                (None, day(2), vec![]),
                (Some(march), day(3), dated(day(3), 1_000)),
                (Some(april), day(4), dated(day(4), 1_000)),
            ],
        };
        let coupons = std::slice::from_ref(&quarter);
        let taken = queue.taken(coupons, "USD", &[quarter.applied_coupon.id]);
        assert_eq!(taken[&march], vec![quarter.applied_coupon.id]);
        assert_eq!(taken[&april], Vec::<AppliedCouponId>::new());
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
            calculate_coupons_discount(1, "USD", None, &[full]).discount_subunit,
            1
        );
        assert_eq!(
            calculate_coupons_discount(1, "USD", None, &[fixed]).discount_subunit,
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
            plan: None,
            add_on_ids: AddOnIds::new(),
            entries: vec![
                (Some(january), day(1), whole(2_500)),
                (Some(february), day(2), whole(49_597)),
                (None, day(3), vec![]),
            ],
        };
        assert!(applicable_coupons(&last.owed(coupons, "USD")).is_empty());

        let closing = Queue {
            plan: None,
            add_on_ids: AddOnIds::new(),
            entries: vec![
                (Some(january), day(1), whole(2_500)),
                (None, day(2), vec![]),
                (Some(march), day(3), whole(50_000)),
            ],
        };
        assert_eq!(applicable_coupons(&closing.owed(coupons, "USD")).len(), 1);
        let taken = closing.taken(coupons, "USD", &[ten.applied_coupon.id]);
        assert_eq!(taken[&january], vec![ten.applied_coupon.id]);
        assert_eq!(taken[&march], Vec::<AppliedCouponId>::new());

        // January's draft bills nothing yet: it takes no use, so the second is March's.
        let empty = Queue {
            plan: None,
            add_on_ids: AddOnIds::new(),
            entries: vec![
                (Some(january), day(1), whole(0)),
                (None, day(2), vec![]),
                (Some(march), day(3), whole(50_000)),
            ],
        };
        let taken = empty.taken(coupons, "USD", &[ten.applied_coupon.id]);
        assert_eq!(taken[&march], vec![ten.applied_coupon.id]);
    }

    /// "50% off Support" on Pro: 20.00 off Support's 40.00, the plan's 100.00 and the usage
    /// whole; once Support is removed, the next invoice takes nothing off, nor a use.
    #[test]
    fn a_coupon_on_an_add_on_discounts_its_lines_alone_while_billed() {
        let support = AddOnId::new();
        let mut half = applied_coupon("SUPPORT50", CouponDiscount::Percentage(Decimal::from(50)));
        half.coupon.add_on_ids = vec![support];
        half.coupon.recurring_value = Some(3);
        let pro = Some(PlanId::new());
        let applicable = applicable_coupons(std::slice::from_ref(&half));
        let inputs = coupon_inputs(&applicable, "USD", pro);
        let line = |subtotal, add_on| core::Part {
            subtotal,
            add_on,
            start: None,
        };

        let with_support = [
            line(10_000, None),
            line(1_000, None),
            line(4_000, Some(support)),
        ];
        let (applied, taxable) = core::discount_parts(&with_support, "USD", &inputs);
        assert_eq!(applied.iter().map(|a| a.value).sum::<i64>(), 2_000);
        assert_eq!(taxable, vec![None, None, Some(2_000)]);

        let removed = [line(10_000, None), line(1_000, None)];
        let (applied, _) = core::discount_parts(&removed, "USD", &inputs);
        assert!(applied.is_empty());
        let queue = Queue {
            plan: pro,
            add_on_ids: AddOnIds::new(),
            entries: vec![
                (Some(InvoiceId::new()), day(1), removed.to_vec()),
                (None, day(2), vec![]),
            ],
        };
        let owed = queue.owed(std::slice::from_ref(&half), "USD");
        assert_eq!(owed[0].applied_coupon.applied_count, Some(0));
    }

    /// A redemption granted 9.00 in EUR bills 9.00 though the coupon no longer has an amount in
    /// EUR.
    #[test]
    fn a_fixed_coupon_bills_the_amount_it_was_redeemed_with() {
        let mut fixed = applied_coupon(
            "NINE",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(10),
            },
        );
        assert_eq!(
            calculate_coupons_discount(5_000, "EUR", None, std::slice::from_ref(&fixed))
                .discount_subunit,
            0
        );
        fixed.coupon = fixed
            .coupon
            .as_redeemed(Some(("EUR".to_owned(), Decimal::from(9))), None);
        assert_eq!(
            calculate_coupons_discount(5_000, "EUR", None, &[fixed]).discount_subunit,
            900
        );
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
        let result = calculate_coupons_discount(10_000, "USD", None, &[credit]);
        assert_eq!(result.discount_subunit, 0);
        assert!(result.applied_coupons.is_empty());
    }

    #[test]
    fn next_invoice_discount_ignores_coupons_unused_today() {
        // Nothing billed today (usage in arrears): the coupon is untouched and fully applies to
        // the next invoice's 50.00 floor.
        let pct = applied_coupon("PCT", CouponDiscount::Percentage(Decimal::from(10)));
        assert_eq!(
            calculate_next_invoice_coupons_discount(5_000, "USD", None, &[pct], &[], None),
            500
        );
    }

    #[test]
    fn next_invoice_discount_drops_a_coupon_spent_today() {
        // A one-cycle coupon consumed on today's invoice no longer applies next time.
        let mut once = applied_coupon("ONCE", CouponDiscount::Percentage(Decimal::from(10)));
        once.coupon.recurring_value = Some(1);
        let today = calculate_coupons_discount(2_000, "USD", None, std::slice::from_ref(&once));
        assert_eq!(today.discount_subunit, 200);

        assert_eq!(
            calculate_next_invoice_coupons_discount(
                5_000,
                "USD",
                None,
                &[once],
                &today.applied_coupons,
                None
            ),
            0
        );
    }

    /// "10% off for 3 months" from January: an April period's invoice is past it, March's not.
    /// Rules: R-089
    #[test]
    fn next_invoice_discount_drops_a_coupon_whose_months_ended() {
        let mut quarter = applied_coupon("Q", CouponDiscount::Percentage(Decimal::from(10)));
        quarter.coupon.duration_months = Some(3);
        let coupons = std::slice::from_ref(&quarter);
        let next = |start: Option<NaiveDate>| {
            calculate_next_invoice_coupons_discount(5_000, "USD", None, coupons, &[], start)
        };
        assert_eq!(next(Some(day(3))), 500);
        assert_eq!(next(Some(day(4))), 0);
        assert_eq!(next(None), 500);
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
            let invoice =
                calculate_coupons_discount(3_000, "USD", None, std::slice::from_ref(&fixed));
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
        let today = calculate_coupons_discount(3_000, "USD", None, std::slice::from_ref(&fixed));
        assert_eq!(today.discount_subunit, 1_000);

        assert_eq!(
            calculate_next_invoice_coupons_discount(
                3_000,
                "USD",
                None,
                std::slice::from_ref(&fixed),
                &today.applied_coupons,
                None
            ),
            1_000
        );
        fixed.applied_coupon.applied_count = Some(1);
        assert_eq!(
            calculate_next_invoice_coupons_discount(
                3_000,
                "USD",
                None,
                &[fixed],
                &today.applied_coupons,
                None
            ),
            0
        );
    }
}

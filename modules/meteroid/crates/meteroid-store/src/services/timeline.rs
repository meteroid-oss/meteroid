//! A subscription's projection (ADR 0015): the invoices its facts bill over the coming months,
//! as the billing core evaluates them, phases, dated items, discounts, terms and coupons
//! included. Fixed fees only: the usage to come is not known.
use billing_core::assembly::discount as core;
use billing_core::charge::{ChargeKey, Fee};
use billing_core::evaluate::{Charge, Evaluation};
use billing_core::facts::FeeFact;
use billing_core::issued::IssuedLine;
use chrono::{Months, NaiveDate, NaiveTime};
use common_domain::ids::{AddOnId, SubscriptionAddOnId, SubscriptionId, TenantId};
use error_stack::Report;
use std::collections::{BTreeMap, HashMap};

use crate::StoreResult;
use crate::domain::AppliedCouponDetailed;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::evaluate::pricer::StoreFee;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedLine {
    pub name: String,
    pub from: NaiveDate,
    pub until: NaiveDate,
    /// Minor units, before coupons; a commitment's is its shortfall with no usage.
    pub amount: i64,
    /// A minimum commitment's shortfall: in `committed_total`, never in `total`.
    pub commitment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedInvoice {
    pub on: NaiveDate,
    pub lines: Vec<ProjectedLine>,
    /// What the coupons in force take off its fees (R-020, R-089).
    pub discount: i64,
    /// Its fees less `discount`, commitments aside.
    pub total: i64,
    /// `total` plus its commitment lines.
    pub committed_total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub until: NaiveDate,
    pub invoices: Vec<ProjectedInvoice>,
    /// The invoices' total over the projection: a contract value up to `until`.
    pub total: i64,
    /// `total` plus the commitments' shortfalls with no usage: the committed contract value.
    pub committed_total: i64,
}

impl Services {
    /// The invoices due from today up to `months` from now, before tax and credits; today's less
    /// what issued documents already bill (R-164).
    pub(in crate::services) async fn project_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        months: u32,
    ) -> StoreResult<Projection> {
        let mut conn = self.store.get_conn().await?;
        project_tx(&mut conn, tenant_id, subscription_id, months).await
    }
}

/// [`Services::project_subscription`] on `conn`: inside a transaction, what it holds so far,
/// a subscription with no document yet included (its start day is today's charges).
pub(crate) async fn project_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    months: u32,
) -> StoreResult<Projection> {
    project_from_tx(
        conn,
        tenant_id,
        subscription_id,
        months,
        common_clock::now().date_naive(),
    )
    .await
}

/// [`project_tx`] from `today` on: a later day leaves out what is due before it (an immediate
/// change's own documents, shown apart).
pub(crate) async fn project_from_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    months: u32,
    today: NaiveDate,
) -> StoreResult<Projection> {
    let until = common_clock::now()
        .date_naive()
        .checked_add_months(Months::new(months.clamp(1, 60)))
        .unwrap_or(today);
    let loaded = crate::services::evaluate::load::load_facts(
        conn,
        tenant_id,
        subscription_id,
        common_clock::now().date_naive(),
        None,
    )
    .await?;
    let facts = &loaded.facts;
    let evaluation = billing_core::evaluate::evaluate(
        facts,
        until.and_time(NaiveTime::MIN).and_utc(),
        None,
        &crate::services::evaluate::pricer::StorePricer,
    )
    .map_err(crate::services::invoice_lines::fees::pricing_error)?;
    let issued = issued_today(conn, tenant_id, subscription_id, &evaluation, today).await?;
    let coupons = Coupons::load(conn, tenant_id, subscription_id).await?;

    let mut by_day: BTreeMap<NaiveDate, Vec<Projected>> = BTreeMap::new();
    let mut keys_today: Vec<&ChargeKey> = Vec::new();
    for charge in &evaluation.charges {
        let on = charge.due_at.date_naive();
        if on < today || on > until {
            continue;
        }
        let amount = if on == today {
            if keys_today.contains(&&charge.key) {
                continue;
            }
            keys_today.push(&charge.key);
            match owed_today(&evaluation, &issued, &charge.key) {
                Some(owed) => owed,
                None => continue,
            }
        } else {
            charge.amount
        };
        let row = row_of(facts, charge);
        by_day.entry(on).or_default().push(Projected {
            line: ProjectedLine {
                name: row.map_or_else(
                    || {
                        if charge.is_commitment() {
                            "Minimum commitment".to_string()
                        } else {
                            "Charge".to_string()
                        }
                    },
                    |row| row.fee.name.clone(),
                ),
                from: charge.window.0,
                until: charge.window.1,
                amount,
                commitment: charge.is_commitment(),
            },
            charge,
            add_on: row
                .and_then(|row| row.fee.sub_add_on_id)
                .and_then(|id| coupons.add_on_ids.get(&id).copied()),
            net: amount,
        });
    }

    let mut uses = coupons.uses();
    let mut invoices: Vec<(NaiveDate, Vec<Projected>, i64)> = by_day
        .into_iter()
        .map(|(on, mut lines)| {
            let discount = coupons.apply(&mut uses, &mut lines);
            (on, lines, discount)
        })
        .collect();
    shortfalls(&evaluation, facts, &mut invoices);

    let invoices: Vec<ProjectedInvoice> = invoices
        .into_iter()
        .filter(|(_, lines, _)| !lines.is_empty())
        .map(|(on, lines, discount)| {
            let mut lines: Vec<ProjectedLine> = lines.into_iter().map(|p| p.line).collect();
            lines.sort_by(|a, b| {
                (a.commitment, &a.name, a.from).cmp(&(b.commitment, &b.name, b.from))
            });
            let fees: i64 = lines
                .iter()
                .filter(|line| !line.commitment)
                .map(|line| line.amount)
                .sum();
            let committed: i64 = lines
                .iter()
                .filter(|line| line.commitment)
                .map(|line| line.amount)
                .sum();
            ProjectedInvoice {
                on,
                lines,
                discount,
                total: fees - discount,
                committed_total: fees - discount + committed,
            }
        })
        .collect();
    Ok(Projection {
        until,
        total: invoices.iter().map(|invoice| invoice.total).sum(),
        committed_total: invoices.iter().map(|invoice| invoice.committed_total).sum(),
        invoices,
    })
}

/// A projected line and what it bills from: its charge, the add-on it bills (which a coupon's
/// scope reads), and what it bills once its coupons are taken off.
struct Projected<'a> {
    line: ProjectedLine,
    charge: &'a Charge,
    add_on: Option<AddOnId>,
    net: i64,
}

/// The fee row a charge bills, as in force at its window's start.
fn row_of<'a>(
    facts: &'a billing_core::facts::Facts<StoreFee>,
    charge: &Charge,
) -> Option<&'a FeeFact<StoreFee>> {
    facts.fees.iter().find(|fact| {
        fact.lineage == charge.key.fee
            && fact.from <= charge.window.0
            && fact.to.is_none_or(|to| to > charge.window.0)
    })
}

/// The lines issued for the charges due `today`: drafts included, they bill once issued.
async fn issued_today(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    evaluation: &Evaluation,
    today: NaiveDate,
) -> StoreResult<Vec<IssuedLine>> {
    let Some(from) = evaluation
        .charges
        .iter()
        .filter(|charge| charge.due_at.date_naive() == today)
        .map(|charge| charge.key.from)
        .min()
    else {
        return Ok(Vec::new());
    };
    Ok(crate::services::evaluate::issued::load_billed(
        conn,
        tenant_id,
        subscription_id,
        from,
        None,
        None,
    )
    .await?
    .lines
    .into_iter()
    .map(|on| on.line)
    .collect())
}

/// What the charge of `key` due today still owes once its issued lines are taken off, as
/// reconcile reads them (R-080); `None` when nothing. A commitment trued up already owes nothing.
fn owed_today(evaluation: &Evaluation, issued: &[IssuedLine], key: &ChargeKey) -> Option<i64> {
    use billing_core::reconcile::{Adjustment, Expected, TakeBack, reconcile};
    let lines: Vec<IssuedLine> = issued
        .iter()
        .filter(|line| line.bills(key))
        .cloned()
        .collect();
    let amount: i64 = evaluation
        .charges
        .iter()
        .filter(|charge| charge.key == *key)
        .map(|charge| charge.amount)
        .sum();
    if key.kind == billing_core::charge::ChargeKind::Shortfall {
        return lines.is_empty().then_some(amount);
    }
    if lines.is_empty() {
        return Some(amount);
    }
    let owed: i64 = reconcile(
        &[Expected {
            key: key.clone(),
            amount,
        }],
        &lines,
        TakeBack::LatestFirst,
    )
    .iter()
    .map(|adjustment| match adjustment {
        Adjustment::Bill { amount, .. } => *amount,
        _ => 0,
    })
    .sum();
    (owed > 0).then_some(owed)
}

/// Each commitment's shortfall with no usage: its floor less the fees in its scope it counts,
/// net of the projection's coupons (R-022); a fee of the period billed before today at its
/// evaluated amount.
fn shortfalls(
    evaluation: &Evaluation,
    facts: &billing_core::facts::Facts<StoreFee>,
    invoices: &mut [(NaiveDate, Vec<Projected>, i64)],
) {
    let projected: Vec<(&Charge, i64)> = invoices
        .iter()
        .flat_map(|(_, lines, _)| lines.iter())
        .filter(|p| !p.line.commitment)
        .map(|p| (p.charge, p.net))
        .collect();
    let in_scope = |scope: &Fee, charge: &Charge| match scope {
        Fee::Minimum { product: None } => true,
        Fee::Minimum { product: Some(id) } => {
            row_of(facts, charge).and_then(|row| row.fee.product_id) == Some(*id)
        }
        _ => false,
    };
    for (_, lines, _) in invoices.iter_mut() {
        for p in lines.iter_mut().filter(|p| p.line.commitment) {
            let (from, until) = p.charge.window;
            let counted: i64 = evaluation
                .charges
                .iter()
                .filter(|charge| {
                    !charge.is_commitment()
                        && charge.window.0 >= from
                        && charge.window.0 < until
                        && in_scope(&p.charge.key.fee, charge)
                })
                .map(|charge| {
                    projected
                        .iter()
                        .find(|(projected, _)| std::ptr::eq(*projected, charge))
                        .map_or(charge.amount, |(_, net)| *net)
                })
                .sum();
            p.line.amount = (p.line.amount - counted).max(0);
            p.net = p.line.amount;
        }
        lines.retain(|p| !p.line.commitment || p.line.amount > 0);
    }
}

/// The subscription's coupons as billing_core applies them to its period invoices, and the
/// add-on each of its add-on rows bills, which a coupon's scope reads.
struct Coupons {
    applied: Vec<AppliedCouponDetailed>,
    currency: String,
    plan: Option<common_domain::ids::PlanId>,
    add_on_ids: HashMap<SubscriptionAddOnId, AddOnId>,
    /// The uses the subscription's open period drafts take once issued, by coupon.
    drafted: Vec<usize>,
}

impl Coupons {
    async fn load(
        conn: &mut crate::store::PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Self> {
        let db = |error: diesel_models::errors::DatabaseErrorContainer| -> Report<StoreError> {
            error.into()
        };
        let sub = diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await
        .map_err(db)?;
        let applied =
            diesel_models::applied_coupons::AppliedCouponDetailedRow::list_by_subscription_id(
                conn,
                &subscription_id,
            )
            .await
            .map_err(db)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<StoreResult<Vec<AppliedCouponDetailed>>>()?;
        let add_on_ids = if applied.is_empty() {
            HashMap::new()
        } else {
            diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_all_by_subscriptions(
                conn,
                tenant_id,
                &[subscription_id],
            )
            .await
            .map_err(db)?
            .into_iter()
            .map(|row| (row.id, row.add_on_id))
            .collect()
        };
        let mut drafted = Vec::new();
        if !applied.is_empty() {
            for row in diesel_models::invoices::InvoiceRow::drafts_of_subscription(
                conn,
                tenant_id,
                subscription_id,
            )
            .await
            .map_err(db)?
            {
                let draft: crate::domain::Invoice = row.try_into()?;
                if !draft.invoice_type.bills_subscription_usage() || draft.is_consolidated_child() {
                    continue;
                }
                drafted.extend(draft.coupons.iter().filter_map(|line| {
                    applied
                        .iter()
                        .position(|c| c.applied_coupon.id == line.applied_coupon_id)
                }));
            }
        }
        Ok(Self {
            applied,
            currency: sub.subscription.currency,
            plan: Some(sub.plan_id),
            add_on_ids,
            drafted,
        })
    }

    fn uses(&self) -> (Vec<core::Coupon>, core::Uses) {
        let (inputs, mut uses) = crate::services::invoice_lines::discount::uses_of(
            &self.applied,
            &self.currency,
            self.plan,
        );
        uses.took(&self.drafted);
        (inputs, uses)
    }

    /// What the next invoice in period order takes off `lines` (commitments aside, R-023), each
    /// line's `net` set; the coupons' uses taken (R-089).
    fn apply(
        &self,
        (inputs, uses): &mut (Vec<core::Coupon>, core::Uses),
        lines: &mut [Projected],
    ) -> i64 {
        let fees: Vec<usize> = (0..lines.len())
            .filter(|i| !lines[*i].line.commitment)
            .collect();
        if inputs.is_empty() || fees.is_empty() {
            return 0;
        }
        let parts: Vec<core::Part> = fees
            .iter()
            .map(|i| core::Part {
                subtotal: lines[*i].line.amount,
                add_on: lines[*i].add_on,
                start: Some(lines[*i].line.from),
            })
            .collect();
        let open: Vec<usize> = (0..inputs.len())
            .filter(|i| uses.left(*i) != Some(0))
            .collect();
        let offered: Vec<core::Coupon> = open.iter().map(|i| inputs[*i].clone()).collect();
        let (applied, taxable) = core::discount_parts(&parts, &self.currency, &offered);
        uses.took(
            &applied
                .iter()
                .map(|applied| open[applied.coupon])
                .collect::<Vec<_>>(),
        );
        for (i, taxable) in fees.iter().zip(taxable) {
            if let Some(net) = taxable {
                lines[*i].net = net;
            }
        }
        applied.iter().map(|applied| applied.value).sum()
    }
}

//! What `evaluate` puts on an invoice, and the state it derives, checked against what the store
//! computed, in tests only (feature `facts-shadow`): each difference not in [`SKIPS`] fails the
//! test, as `timeline_shadow` does (Phase 7, PR 2).

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use billing_core::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use billing_core::evaluate::{DerivedState, DocumentSlot, Evaluation, Status, evaluate};
use billing_core::pricer::{UsageRead, UsageReads, UsageWindow};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::{Invoice, LineItem, LineItemKind, Subscription};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::clients::usage::UsageClient;
use crate::services::evaluate::load::{LoadedFacts, load_facts};
use crate::services::evaluate::pricer::StorePricer;
use crate::services::evaluate::usage::{plan_usage, read_one};
use crate::services::invoice_lines::charges::{Lineages, evaluated_charge_key};
use crate::services::invoice_lines::fees::pricing_error;
use crate::store::PgConn;

static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the facts found different from the invoices since the last call.
pub fn take_mismatches() -> Vec<String> {
    MISMATCHES
        .lock()
        .map(|mut found| std::mem::take(&mut *found))
        .unwrap_or_default()
}

/// A known divergence: a difference of `class` whose text holds `contains`.
struct Skip {
    class: &'static str,
    contains: &'static str,
    reason: &'static str,
}

/// Known, explained divergences; each entry says in one line why it is expected.
const SKIPS: &[Skip] = &[];

fn record(subscription: SubscriptionId, class: &'static str, detail: &str) {
    if let Some(skip) = SKIPS
        .iter()
        .find(|skip| skip.class == class && detail.contains(skip.contains))
    {
        log::debug!("facts shadow: skipped ({}): {detail}", skip.reason);
        return;
    }
    let difference = format!("{subscription} [{class}] {detail}");
    log::warn!("facts shadow: {difference}");
    if let Ok(mut found) = MISMATCHES.lock() {
        found.push(difference);
    }
}

/// The invoice whose computed lines are checked.
pub(crate) struct Shadowed<'a> {
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub invoice_type: InvoiceType,
    /// The period it settles: a recurring invoice's start, an adjustment's day.
    pub period_start: NaiveDate,
    /// The draft being refreshed, whose pinned unit prices its usage keeps.
    pub draft: Option<&'a [LineItem]>,
}

impl<'a> Shadowed<'a> {
    /// A subscription's draft being refreshed, unless an operator owns its lines.
    pub(crate) fn of_invoice(invoice: &'a Invoice) -> Option<Self> {
        if invoice.is_correction() || invoice.manually_edited_at.is_some() {
            return None;
        }
        Some(Self {
            tenant_id: invoice.tenant_id,
            subscription_id: invoice.subscription_id?,
            invoice_type: invoice.invoice_type.clone(),
            period_start: invoice.billing_period_start,
            draft: Some(&invoice.line_items),
        })
    }

    fn slot(&self) -> Option<DocumentSlot> {
        match self.invoice_type {
            InvoiceType::Recurring => Some(DocumentSlot::Recurring {
                period_start: self.period_start,
            }),
            InvoiceType::Final => Some(DocumentSlot::Final),
            InvoiceType::Adjustment => Some(DocumentSlot::Adjustment {
                on: self.period_start,
            }),
            InvoiceType::OneOff | InvoiceType::UsageThreshold => None,
        }
    }
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(NaiveTime::MIN).and_utc()
}

impl Services {
    /// Checks `lines`, computed for `invoice`, against the charges the facts put on it, and the
    /// state they derive against the stored row. Reads in a savepoint, reports and never fails.
    pub(crate) async fn shadow_facts(
        &self,
        conn: &mut PgConn,
        invoice: Shadowed<'_>,
        lines: &[LineItem],
    ) {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.shadow_facts_unboxed(conn, invoice, lines)).await;
    }

    async fn shadow_facts_unboxed(
        &self,
        conn: &mut PgConn,
        invoice: Shadowed<'_>,
        lines: &[LineItem],
    ) {
        let Some(slot) = invoice.slot() else {
            return;
        };
        let clock = common_domain::clock::now();
        // An invoice computed ahead of its date (the upcoming one) is evaluated on its date.
        let now = clock.max(midnight(invoice.period_start));
        let usage: Arc<dyn UsageClient> = match &self.served {
            Some(served) => match served.peek() {
                Some(peek) => peek,
                None => return,
            },
            None => self.usage_client.clone(),
        };
        let subscription = invoice.subscription_id;
        let checked = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    compare(
                        conn,
                        usage.as_ref(),
                        &invoice,
                        slot,
                        now,
                        now == clock,
                        lines,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await;
        match checked {
            Ok(differences) => {
                for (class, detail) in differences {
                    record(subscription, class, &detail);
                }
            }
            Err(error) => record(subscription, "load", &format!("{error:?}")),
        }
    }
}

type Differences = Vec<(&'static str, String)>;

async fn compare(
    conn: &mut PgConn,
    usage: &dyn UsageClient,
    invoice: &Shadowed<'_>,
    slot: DocumentSlot,
    now: DateTime<Utc>,
    at_clock: bool,
    lines: &[LineItem],
) -> StoreResult<Differences> {
    let loaded = load_facts(
        conn,
        invoice.tenant_id,
        invoice.subscription_id,
        now.date_naive(),
        invoice.draft,
    )
    .await?;
    let reads = read_for(usage, &loaded, now, lines).await?;
    let evaluation =
        evaluate(&loaded.facts, now, Some(&reads), &StorePricer).map_err(pricing_error)?;
    let mut differences = charge_differences(&loaded, &evaluation, slot, lines);
    if at_clock {
        differences.extend(
            state_differences(
                conn,
                invoice.tenant_id,
                invoice.subscription_id,
                &evaluation.state,
                now.date_naive(),
            )
            .await?,
        );
    }
    Ok(differences)
}

/// The usage of the windows evaluate needs that the computed lines read too: the shadow never
/// reads metering the computation did not, so it adds no read under a lock nor a pass. A window
/// the two disagree on stays unpriced, its line then billing no charge.
async fn read_for(
    usage: &dyn UsageClient,
    loaded: &LoadedFacts,
    now: DateTime<Utc>,
    lines: &[LineItem],
) -> StoreResult<UsageReads> {
    let computed: HashSet<UsageRead> = lines
        .iter()
        .filter_map(|line| {
            Some(UsageRead {
                metric: line.metric_id?.as_uuid(),
                window: UsageWindow {
                    from: midnight(line.start_date),
                    until: line
                        .usage_ends_at
                        .unwrap_or_else(|| midnight(line.end_date)),
                },
            })
        })
        .collect();
    let mut reads = UsageReads::new();
    for need in plan_usage(loaded, now)? {
        if !computed.contains(&need.read) || reads.contains(&need.read) {
            continue;
        }
        if let Ok(read) = read_one(usage, loaded, &need.read).await {
            reads.insert(need.read, read);
        }
    }
    Ok(reads)
}

/// Whether a line keyed `line` bills the charge `key`, as reconcile matches them (R-080).
fn bills(line: &ChargeKey, key: &ChargeKey) -> bool {
    line.subscription == key.subscription
        && line.fee == key.fee
        && line.kind == key.kind
        && line.denomination == key.denomination
        && key.from <= line.from
        && line.until <= key.until
        && (line.from < line.until || key.from == key.until)
}

fn describe(key: &ChargeKey) -> String {
    let fee = match &key.fee {
        Fee::Component(id) => format!("component {id}"),
        Fee::AddOn(id) => format!("add-on {id}"),
        Fee::Minimum { product } => format!("minimum {product:?}"),
        Fee::Unattached => "unattached".to_owned(),
    };
    let kind = match &key.kind {
        ChargeKind::Fee => "fee".to_owned(),
        ChargeKind::Usage { metric, dimensions } => format!("usage {metric} {dimensions:?}"),
        ChargeKind::Shortfall => "shortfall".to_owned(),
        ChargeKind::Credit => "credit".to_owned(),
    };
    let denomination = match key.denomination {
        Denomination::Money => String::new(),
        Denomination::CreditUnit(unit) => format!(" in credits {unit}"),
    };
    format!("{fee} {kind} {}..{}{denomination}", key.from, key.until)
}

#[derive(Default)]
struct Billed {
    amount: i64,
    quantity: Option<Decimal>,
}

impl Billed {
    fn add(&mut self, amount: i64, quantity: Option<Decimal>) {
        self.amount += amount;
        if let Some(quantity) = quantity {
            *self.quantity.get_or_insert(Decimal::ZERO) += quantity;
        }
    }
}

/// What a computed line bills in the figures prices are in: a tax-inclusive line at its quote
/// (R-081), a line priced in a credit unit the credits it consumed (R-039).
fn line_amount(line: &LineItem) -> i64 {
    match (&line.credit_unit, &line.tax_inclusive_quote) {
        (Some(unit), _) => unit.consumed,
        (None, Some(quote)) => quote.amount_subtotal,
        (None, None) => line.amount_subtotal,
    }
}

/// Per charge on `slot`, its amount (and a usage charge's quantity) against the computed lines
/// billing it; a line billing no such charge is a difference too. A minimum's commitment and
/// its true-up are reconcile's, never compared here (R-022).
fn charge_differences(
    loaded: &LoadedFacts,
    evaluation: &Evaluation,
    slot: DocumentSlot,
    lines: &[LineItem],
) -> Differences {
    let subscription = Some(loaded.facts.subscription);
    let mut expected: Vec<(ChargeKey, Billed)> = Vec::new();
    for charge in evaluation
        .charges
        .iter()
        .filter(|charge| charge.document == slot && !charge.is_commitment())
    {
        let quantity = matches!(charge.key.kind, ChargeKind::Usage { .. })
            .then_some(charge.quantity)
            .flatten();
        match expected.iter_mut().find(|(key, _)| *key == charge.key) {
            Some((_, billed)) => billed.add(charge.amount, quantity),
            None => {
                let mut billed = Billed::default();
                billed.add(charge.amount, quantity);
                expected.push((charge.key.clone(), billed));
            }
        }
    }

    let lineages = Lineages::of_roots(loaded.facts.fees.iter().filter_map(
        |row| match row.lineage {
            Fee::Component(root) | Fee::AddOn(root) => Some((row.id, root)),
            _ => None,
        },
    ));
    let mut computed: Vec<Billed> = expected.iter().map(|_| Billed::default()).collect();
    let mut unexpected: BTreeMap<ChargeKey, Billed> = BTreeMap::new();
    for line in lines
        .iter()
        .filter(|line| line.kind != LineItemKind::MinimumShortfall)
    {
        let mut key = evaluated_charge_key(line, &lineages);
        key.subscription = key.subscription.or(subscription);
        let quantity = matches!(key.kind, ChargeKind::Usage { .. })
            .then_some(line.quantity)
            .flatten();
        // The narrowest charge the line falls in, so an ending's stub never takes a period's line.
        let narrowest = expected
            .iter()
            .enumerate()
            .filter(|(_, (charge, _))| bills(&key, charge))
            .min_by_key(|(_, (charge, _))| charge.until - charge.from)
            .map(|(i, _)| i);
        match narrowest {
            Some(i) => computed[i].add(line_amount(line), quantity),
            None => unexpected
                .entry(key)
                .or_default()
                .add(line_amount(line), quantity),
        }
    }

    let mut differences = Differences::new();
    for ((key, charge), line) in expected.iter().zip(&computed) {
        if charge.amount != line.amount {
            differences.push((
                "amount",
                format!(
                    "{slot:?} {}: expected {} computed {}",
                    describe(key),
                    charge.amount,
                    line.amount
                ),
            ));
        }
        if let (Some(expected), Some(computed)) = (charge.quantity, line.quantity)
            && expected != computed
        {
            differences.push((
                "quantity",
                format!(
                    "{slot:?} {}: expected {expected} computed {computed}",
                    describe(key)
                ),
            ));
        }
    }
    for (key, line) in unexpected.iter().filter(|(_, line)| line.amount != 0) {
        differences.push((
            "amount",
            format!(
                "{slot:?} {}: expected no charge computed {}",
                describe(key),
                line.amount
            ),
        ));
    }
    differences
}

/// The stored statuses a derived one stands for; none for those no fact derives yet.
fn stored_as(status: &SubscriptionStatusEnum) -> Option<Status> {
    match status {
        SubscriptionStatusEnum::PendingActivation | SubscriptionStatusEnum::PendingCharge => {
            Some(Status::Pending)
        }
        SubscriptionStatusEnum::TrialActive => Some(Status::Trialing),
        SubscriptionStatusEnum::TrialExpired => Some(Status::TrialExpired),
        SubscriptionStatusEnum::Active => Some(Status::Active),
        SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Aborted => {
            Some(Status::Cancelled)
        }
        SubscriptionStatusEnum::Completed => Some(Status::Completed),
        SubscriptionStatusEnum::Paused
        | SubscriptionStatusEnum::Suspended
        | SubscriptionStatusEnum::Superseded
        | SubscriptionStatusEnum::Errored => None,
    }
}

/// The derived status, current period and plan version against the stored row, once the row
/// has caught up with `today` (a renewal still to run leaves it a period behind).
async fn state_differences(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    state: &DerivedState,
    today: NaiveDate,
) -> StoreResult<Differences> {
    let stored: Subscription =
        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;
    let mut differences = Differences::new();
    if stored.current_period_end.is_some_and(|end| end <= today) {
        return Ok(differences);
    }
    if let Some(status) = stored_as(&stored.status)
        && status != state.status
    {
        differences.push((
            "status",
            format!("derived {:?} stored {:?}", state.status, stored.status),
        ));
    }
    let period = Some((stored.current_period_start, stored.current_period_end));
    if state.current_period != period {
        differences.push((
            "current_period",
            format!("derived {:?} stored {period:?}", state.current_period),
        ));
    }
    if state.plan_version != Some(stored.plan_version_id.as_uuid()) {
        differences.push((
            "plan_version",
            format!(
                "derived {:?} stored {}",
                state.plan_version, stored.plan_version_id
            ),
        ));
    }
    Ok(differences)
}

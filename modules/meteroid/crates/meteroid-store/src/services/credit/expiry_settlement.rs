//! Lapses expired credit, after paying the usage it covered. Credit that expires mid-period pays
//! the usage of the open periods up to its expiry, priced exactly from the metering store, so
//! the value lapses only once it can no longer pay for anything the customer consumed. The
//! invoice of the period later takes those settlements over (`net_settlements_then_draw_tx`).
//!
//! Events reaching the metering store more than the settlement grace (`settlement_grace`: the
//! invoicing entity's) after the expiry are not covered: they are billed on the invoice and paid
//! from the credit still valid then. An invoice
//! finalizing before the sweep settles the expiries due in its own periods itself, under the same
//! grant locks, and first rolls over the expired credit whose rollover it could spend
//! (`settle_due_expiries_tx`). A settlement whose period can no longer be billed is given back to
//! its grants (`return_orphaned_settlements`).
use super::billing_windows::{
    PlannedWindow, customer_windows, price_window_groups, resolve_window_taxes, windows_at,
};
use crate::Store;
use crate::StoreResult;
use crate::domain::credit::GrantRollover;
use crate::domain::{LineItem, SubscriptionDetails, UsagePeriod};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::allocation::GrantPool;
use crate::repositories::credit::customer_credit::{
    LockedGrants, MIN_SETTLEMENT_GRACE, MovementTime, UsageOwed, UsageSettlementTerms,
    awaits_rollover_tx, balance_changed_tx, customer_settlement_grace_tx, expire_value_tx,
    lapse_unsettled_tx, list_drawable_tx, lock_customer_tx, lock_drawable_tx, lock_grants_tx,
    normalized_currency, return_settlement_tx, roll_over_tx, settle_usage_tx, settlement_grace,
    settlement_holders_tx,
};
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::clients::usage::{GroupedUsageData, UsageClient};
use crate::services::credit::invalid;
use crate::services::credit::metrics::{self, LapseKind, SettlementSource};
use crate::services::invoice_lines::component::{PinnedPrices, is_usage_based_line};
use crate::services::invoice_lines::invoice_lines::BilledSpan;
use crate::store::PgConn;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, CreditGrantId, CustomerId, SubscriptionId, TenantId,
    UsageSettlementId, WalletId,
};
use diesel_models::customer_credit::{
    CreditUsageSettlementRow, CustomerCreditAccountRow, CustomerCreditGrantRow, DueExpiryGroup,
    ScopedGrantRow,
};
use diesel_models::query::credit_usage::{ConsolidatedDrafts, list_draft_recurring_periods};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};

/// How long a settlement may wait for the invoice of its period before the period is judged
/// unbillable: the invoice is created when the period closes, well within this.
const RETURN_AFTER: Duration = Duration::days(1);

/// How long after its settlement grace credit lapses even though its usage could not be settled,
/// so a customer whose settlement keeps failing does not hold expired value forever.
const UNSETTLED_LAPSE_AFTER: Duration = Duration::days(7);

/// A metric's usage over a window, as one subscription's usage filter attributes it.
type UsageKey = (BillableMetricId, String, NaiveDateTime, NaiveDateTime);
type Periods<'a> = BTreeMap<(uuid::Uuid, NaiveDate), (SubscriptionId, Vec<&'a PlannedWindow>)>;

/// Gives back the settlements no invoice can take over, then settles and lapses every expired
/// grant that still holds value and expired at least its customer's settlement grace ago, `page`
/// customers at a time. A customer that fails is logged and retried by the next pass; it never
/// blocks the others. Returns how many grants lapsed.
pub(crate) async fn lapse_expired_credit(store: &Store, page: i64) -> StoreResult<usize> {
    let page = page.max(1);
    if let Err(error) =
        super::subscription_end::end_credit_of_ended_subscriptions(store, page).await
    {
        log::error!("credit of ended subscriptions was not ended: {error:?}");
    }
    let now = Utc::now();
    if let Err(error) = return_orphaned_settlements(store, page, now).await {
        log::error!("orphaned usage settlements were not returned: {error:?}");
    }
    let mut lapsed = 0_usize;
    let mut after = None;
    loop {
        let mut conn = store.get_conn().await?;
        let candidates = CustomerCreditGrantRow::list_due_groups(
            &mut conn,
            now - MIN_SETTLEMENT_GRACE,
            after,
            page,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        let full_page = i64::try_from(candidates.len()).is_ok_and(|len| len == page);
        after = candidates.last().map(|candidate| {
            let group = &candidate.group;
            (group.tenant_id, group.customer_id, group.currency.clone())
        });
        for candidate in candidates {
            let grace = settlement_grace(
                candidate.grace_period_hours,
                candidate.consolidate_recurring_invoices,
            );
            let expired_by = now - grace;
            if candidate.first_lapse > expired_by {
                continue;
            }
            let group = candidate.group;
            let settled = match settle_group(store, &group, expired_by, now).await {
                Ok(settled) => {
                    metrics::expiry_settled(
                        SettlementSource::Sweep,
                        &normalized_currency(&group.currency),
                        settled.settlements,
                        settled.settled_minor,
                    );
                    metrics::expiry_lapsed(LapseKind::Settled, settled.lapsed);
                    metrics::credit_rolled_over(
                        &normalized_currency(&group.currency),
                        settled.rolled,
                        settled.rolled_minor,
                    );
                    settled.lapsed
                }
                Err(error) => {
                    log::error!(
                        "expired credit of customer {} ({}) was not lapsed: {error:?}",
                        group.customer_id,
                        group.currency
                    );
                    0
                }
            };
            let count = if settled > 0 {
                settled
            } else {
                lapse_unsettled_group(store, &group, expired_by, now).await
            };
            lapsed += count;
        }
        if !full_page {
            return Ok(lapsed);
        }
    }
}

/// The sweep's settle-then-lapse of one customer, for credit that expired by `as_of` (no grace:
/// the caller states the moment). Each currency settles in its own transaction.
pub(crate) async fn lapse_expired_credit_for(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    as_of: DateTime<Utc>,
) -> StoreResult<usize> {
    if as_of > Utc::now() {
        return Err(invalid("credit cannot be lapsed at a future moment"));
    }
    let mut conn = store.get_conn().await?;
    let mut currencies =
        CustomerCreditAccountRow::list_for_customer(&mut conn, tenant_id, customer_id)
            .await?
            .into_iter()
            .map(|account| account.currency)
            .collect::<Vec<_>>();
    drop(conn);
    currencies.sort();
    currencies.dedup();
    let mut lapsed = 0_usize;
    for currency in currencies {
        let group = DueExpiryGroup {
            tenant_id,
            customer_id,
            currency,
        };
        let settled = settle_group(store, &group, as_of, as_of).await?;
        let currency = normalized_currency(&group.currency);
        metrics::expiry_settled(
            SettlementSource::Sweep,
            &currency,
            settled.settlements,
            settled.settled_minor,
        );
        metrics::expiry_lapsed(LapseKind::Settled, settled.lapsed);
        metrics::credit_rolled_over(&currency, settled.rolled, settled.rolled_minor);
        lapsed += settled.lapsed;
    }
    Ok(lapsed)
}

/// Lapses what expired at least `UNSETTLED_LAPSE_AFTER` before `expired_by` (the end of the
/// settlement grace) without settling its usage, once the settling itself has failed to lapse it
/// all that time.
async fn lapse_unsettled_group(
    store: &Store,
    group: &DueExpiryGroup,
    expired_by: DateTime<Utc>,
    now: DateTime<Utc>,
) -> usize {
    let cutoff = expired_by - UNSETTLED_LAPSE_AFTER;
    let currency = normalized_currency(&group.currency);
    let currency = &currency;
    let outcome = store
        .transaction(|conn| {
            async move {
                let lock = lock_customer_tx(conn, group.tenant_id, group.customer_id).await?;
                let grants = lock_drawable_tx(conn, &lock, currency).await?;
                let mut touched = Vec::new();
                let mut lapsed = 0_usize;
                for ScopedGrantRow { grant, .. } in &grants {
                    if grant.remaining <= 0 || grant.lapses_at.is_none_or(|at| at > cutoff) {
                        continue;
                    }
                    // What is left is unreliable without the usage settled: it never rolls over.
                    if grant.rollover.is_some() {
                        log::error!(
                            "credit grant {} lapsed unsettled without rolling over",
                            grant.id
                        );
                    }
                    let updated = lapse_unsettled_tx(conn, grant, grant.remaining).await?;
                    touched.push(updated.account_id);
                    lapsed += 1;
                }
                balance_changed_tx(conn, &*store.hooks.credit, group.tenant_id, &touched, now)
                    .await?;
                Ok::<_, Report<StoreError>>(lapsed)
            }
            .scope_boxed()
        })
        .await;
    match outcome {
        Ok(lapsed) => {
            metrics::expiry_lapsed(LapseKind::Unsettled, lapsed);
            if lapsed > 0 {
                log::error!(
                    "credit of customer {} ({currency}) lapsed without settling its usage",
                    group.customer_id
                );
            }
            lapsed
        }
        Err(error) => {
            log::error!(
                "credit of customer {} ({currency}) could not be lapsed: {error:?}",
                group.customer_id
            );
            0
        }
    }
}

/// Whether `settle_due_expiries_tx` has any expiry to settle at `expired_by`.
pub(crate) fn has_lapsed_expiries(grants: &LockedGrants, expired_by: DateTime<Utc>) -> bool {
    !lapsed_expiries(grants, expired_by).is_empty()
}

/// The distinct lapses (expiries or ends), oldest first, of the grants that lapsed by `expired_by`.
fn lapsed_expiries(grants: &LockedGrants, expired_by: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let mut expiries = grants
        .iter()
        .filter(|scoped| scoped.grant.remaining > 0)
        .filter_map(|scoped| scoped.grant.lapses_at)
        .filter(|at| *at <= expired_by)
        .collect::<Vec<_>>();
    expiries.sort();
    expiries.dedup();
    expiries
}

/// The periods whose usage is not on a finalized invoice: the windows the customer's
/// subscriptions bill next and those of their draft invoices not merged into a consolidated
/// invoice, the prices pinned on those drafts,
/// and every `(subscription, period start)` some invoice can still bill.
struct OpenPeriods {
    windows: Vec<PlannedWindow>,
    pinned: PinnedPrices,
    billable: HashSet<(SubscriptionId, NaiveDate)>,
}

async fn open_periods(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    now: DateTime<Utc>,
) -> StoreResult<OpenPeriods> {
    open_periods_of(store, conn, tenant_id, customer_id, None, now).await
}

/// `open_periods`, of the subscriptions in `only` when given.
async fn open_periods_of(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    only: Option<&HashSet<SubscriptionId>>,
    now: DateTime<Utc>,
) -> StoreResult<OpenPeriods> {
    let mut windows = customer_windows(store, conn, tenant_id, customer_id, only, now).await?;
    let planned = windows
        .iter()
        .map(|window| (window.subscription_id, window.period.start))
        .collect::<HashSet<_>>();
    let mut billable = planned.clone();
    let mut pinned = PinnedPrices::default();
    let mut details: HashMap<SubscriptionId, SubscriptionDetails> = HashMap::new();
    for draft in
        list_draft_recurring_periods(conn, tenant_id, customer_id, ConsolidatedDrafts::Exclude)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|draft| only.is_none_or(|only| only.contains(&draft.subscription_id)))
    {
        let lines = serde_json::from_value::<Vec<LineItem>>(draft.line_items).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize line_items".to_owned(),
                error,
            ))
        })?;
        pinned.add_lines(&lines, draft.tax_inclusive);
        billable.extend(
            lines
                .iter()
                .filter(|line| is_usage_based_line(line))
                .map(|line| (draft.subscription_id, line.start_date)),
        );
        let subscription = match details.entry(draft.subscription_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                store
                    .get_subscription_details_with_conn(conn, tenant_id, draft.subscription_id)
                    .await?,
            ),
        };
        let cycle_index = draft
            .cycle_index
            .and_then(|cycle| u32::try_from(cycle).ok())
            .or(subscription.subscription.cycle_index)
            .unwrap_or(0);
        let invoice_type: crate::domain::enums::InvoiceType = draft.invoice_type.clone().into();
        let span = BilledSpan::of(&invoice_type);
        let mut draft_windows = windows_at(conn, subscription, draft.anchor, cycle_index, span)
            .await?
            .into_iter()
            .filter(|window| !planned.contains(&(window.subscription_id, window.period.start)))
            .collect::<Vec<_>>();
        resolve_window_taxes(store, conn, subscription, &mut draft_windows, now).await?;
        billable.extend(
            draft_windows
                .iter()
                .map(|window| (window.subscription_id, window.period.start)),
        );
        windows.extend(draft_windows);
    }
    Ok(OpenPeriods {
        windows,
        pinned,
        billable,
    })
}

/// The open windows billing one currency (credit-priced ones by their money overage), with the
/// prices pinned on the drafts.
async fn open_windows(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    now: DateTime<Utc>,
) -> StoreResult<(Vec<PlannedWindow>, PinnedPrices)> {
    open_windows_of(store, conn, tenant_id, customer_id, currency, None, now).await
}

/// `open_windows`, of the subscriptions in `only` when given.
async fn open_windows_of(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    only: Option<&HashSet<SubscriptionId>>,
    now: DateTime<Utc>,
) -> StoreResult<(Vec<PlannedWindow>, PinnedPrices)> {
    let mut open = open_periods_of(store, conn, tenant_id, customer_id, only, now).await?;
    open.windows
        .retain(|window| window.bills_money_in(currency));
    Ok((open.windows, open.pinned))
}

fn by_period(windows: &[PlannedWindow]) -> Periods<'_> {
    let mut periods = Periods::new();
    for window in windows {
        periods
            .entry((window.subscription_id.as_uuid(), window.period.start))
            .or_insert_with(|| (window.subscription_id, Vec::new()))
            .1
            .push(window);
    }
    periods
}

/// The part of a window that ended, or was still running, at `expiry`; none when it had not begun.
fn window_until(window: &PlannedWindow, expiry: DateTime<Utc>) -> Option<NaiveDateTime> {
    let until = window.end.min(expiry.naive_utc());
    (until > window.start).then_some(until)
}

fn usage_keys(
    windows: &[PlannedWindow],
    expiries: &[DateTime<Utc>],
) -> Vec<(UsageKey, PlannedWindow)> {
    let mut keys = HashMap::new();
    for expiry in expiries {
        for window in windows {
            if let Some(until) = window_until(window, *expiry) {
                keys.entry((
                    window.metric.id,
                    window.metric.usage_scope.fingerprint(),
                    window.start,
                    until,
                ))
                .or_insert_with(|| window.clone());
            }
        }
    }
    keys.into_iter().collect()
}

async fn read_usage(
    usage_client: &dyn UsageClient,
    tenant_id: TenantId,
    customer_id: CustomerId,
    windows: &[PlannedWindow],
    expiries: &[DateTime<Utc>],
) -> StoreResult<HashMap<UsageKey, Vec<GroupedUsageData>>> {
    let mut usage = HashMap::new();
    for (key, window) in usage_keys(windows, expiries) {
        let read = usage_client
            .fetch_usage(
                &tenant_id,
                &customer_id,
                &window.metric,
                UsagePeriod {
                    start: key.2,
                    end: key.3,
                },
            )
            .await?;
        usage.insert(key, read.data);
    }
    Ok(usage)
}

#[derive(Debug, Default, Clone, Copy)]
struct GroupLapse {
    lapsed: usize,
    settlements: usize,
    settled_minor: i64,
    rolled: usize,
    rolled_minor: i64,
}

async fn settle_group(
    store: &Store,
    group: &DueExpiryGroup,
    expired_by: DateTime<Utc>,
    now: DateTime<Utc>,
) -> StoreResult<GroupLapse> {
    let (tenant_id, customer_id) = (group.tenant_id, group.customer_id);
    let currency = normalized_currency(&group.currency);

    // Usage is read before any lock is taken: a slow metering store never holds up a drawer.
    let mut conn = store.get_conn().await?;
    let grants = list_drawable_tx(&mut conn, tenant_id, customer_id, &currency).await?;
    let expiries = lapsed_expiries(&grants, expired_by);
    if expiries.is_empty() {
        return Ok(GroupLapse::default());
    }
    let (windows, _) =
        open_windows(store, &mut conn, tenant_id, customer_id, &currency, now).await?;
    drop(conn);

    let usage = read_usage(
        store.usage_client.as_ref(),
        tenant_id,
        customer_id,
        &windows,
        &expiries,
    )
    .await?;
    let usage = &usage;
    let currency = &currency;
    store
        .transaction(|conn| {
            async move {
                lapse_group_tx(store, conn, group, currency, expired_by, now, usage).await
            }
            .scope_boxed()
        })
        .await
}

/// Pays the open periods' usage up to each expiry, then lapses what is left of the grants that
/// expired then. Writes nothing when the usage read beforehand no longer covers the periods.
async fn lapse_group_tx(
    store: &Store,
    conn: &mut PgConn,
    group: &DueExpiryGroup,
    currency: &str,
    expired_by: DateTime<Utc>,
    now: DateTime<Utc>,
    usage: &HashMap<UsageKey, Vec<GroupedUsageData>>,
) -> StoreResult<GroupLapse> {
    let (tenant_id, customer_id) = (group.tenant_id, group.customer_id);
    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
    let mut grants: LockedGrants = lock_drawable_tx(conn, &lock, currency).await?;
    let expiries = lapsed_expiries(&grants, expired_by);
    if expiries.is_empty() {
        return Ok(GroupLapse::default());
    }
    let (windows, pinned) =
        open_windows(store, conn, tenant_id, customer_id, currency, now).await?;
    if usage_keys(&windows, &expiries)
        .iter()
        .any(|(key, _)| !usage.contains_key(key))
    {
        log::warn!(
            "credit of customer {customer_id} expired while its periods changed; retried by the next pass"
        );
        return Ok(GroupLapse::default());
    }
    settle_and_lapse_tx(
        conn,
        &*store.hooks.credit,
        tenant_id,
        customer_id,
        currency,
        &mut grants,
        &windows,
        &pinned,
        expiries,
        usage,
        now,
    )
    .await
}

/// Lapses, in the transaction of an invoice void, the value it gave back to credit whose expiry
/// is due: settled and lapsed exactly as the sweep would, so the ledger records the expiry now and
/// the sweep finds nothing left. The caller holds the customer's grant locks (`lock_grants_tx`).
pub(crate) async fn lapse_due_expiries_tx(
    store: &Store,
    usage_client: &dyn UsageClient,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let currency = normalized_currency(currency);
    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
    let mut grants: LockedGrants = lock_drawable_tx(conn, &lock, &currency).await?;
    let grace = customer_settlement_grace_tx(conn, tenant_id, customer_id).await?;
    let expiries = lapsed_expiries(&grants, now - grace);
    if expiries.is_empty() {
        return Ok(());
    }
    let (windows, pinned) =
        open_windows(store, conn, tenant_id, customer_id, &currency, now).await?;
    let usage = read_usage(usage_client, tenant_id, customer_id, &windows, &expiries).await?;
    let lapse = settle_and_lapse_tx(
        conn,
        &*store.hooks.credit,
        tenant_id,
        customer_id,
        &currency,
        &mut grants,
        &windows,
        &pinned,
        expiries,
        &usage,
        now,
    )
    .await?;
    metrics::expiry_settled(
        SettlementSource::Sweep,
        &currency,
        lapse.settlements,
        lapse.settled_minor,
    );
    metrics::expiry_lapsed(LapseKind::Settled, lapse.lapsed);
    metrics::credit_rolled_over(&currency, lapse.rolled, lapse.rolled_minor);
    Ok(())
}

/// Pays the open periods' usage up to each expiry, then lapses what is left of the grants that
/// expired then. The caller holds the grant locks and read `usage` for every window up to them.
#[allow(clippy::too_many_arguments)]
async fn settle_and_lapse_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    grants: &mut LockedGrants,
    windows: &[PlannedWindow],
    pinned: &PinnedPrices,
    expiries: Vec<DateTime<Utc>>,
    usage: &HashMap<UsageKey, Vec<GroupedUsageData>>,
    now: DateTime<Utc>,
) -> StoreResult<GroupLapse> {
    let mut touched = Vec::new();
    let mut lapsed = 0_usize;
    let mut settlements = 0_usize;
    let mut settled_minor = 0_i64;
    let mut rolled = 0_usize;
    let mut rolled_minor = 0_i64;
    for expiry in expiries {
        let (paid_by, settled) = settle_expiry_tx(
            conn,
            tenant_id,
            customer_id,
            currency,
            grants,
            windows,
            pinned,
            expiry,
            usage,
            &[],
            UnitCover::AtBound,
        )
        .await?;
        settlements += paid_by.len();
        settled_minor = settled_minor.saturating_add(settled);
        touched.extend(paid_by);
        let mut rolled_over = Vec::new();
        for scoped in grants.iter_mut() {
            if scoped.grant.lapses_at != Some(expiry) || scoped.grant.remaining <= 0 {
                continue;
            }
            if let Some(rolled_into) = lapse_grant_tx(conn, controls, scoped, currency, now).await?
            {
                rolled_minor = rolled_minor.saturating_add(rolled_into.grant.amount);
                rolled_over.push(rolled_into);
            }
            touched.push(scoped.grant.account_id);
            lapsed += 1;
        }
        // Rolled credit is spendable from this expiry on: a later expiry's usage burns it first.
        rolled += rolled_over.len();
        grants.extend(rolled_over);
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;
    Ok(GroupLapse {
        lapsed,
        settlements,
        settled_minor,
        rolled,
        rolled_minor,
    })
}

/// Lapses what is left of a lapsed grant, rolling it over first when it has a rollover; returns the
/// grant it rolled into. `scoped` is kept current.
async fn lapse_grant_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    scoped: &mut ScopedGrantRow,
    currency: &str,
    now: DateTime<Utc>,
) -> StoreResult<Option<ScopedGrantRow>> {
    let (updated, rolled_into) = match scoped.grant.rollover.as_ref().map(GrantRollover::from_json)
    {
        Some(rollover) => {
            let (updated, rolled_into) =
                roll_over_tx(conn, controls, scoped, currency, &rollover?, now).await?;
            (
                updated,
                rolled_into.map(|grant| ScopedGrantRow {
                    grant,
                    product_ids: scoped.product_ids.clone(),
                }),
            )
        }
        None => (
            expire_value_tx(conn, &scoped.grant, scoped.grant.remaining).await?,
            None,
        ),
    };
    scoped.grant = updated;
    Ok(rolled_into)
}

/// Pays, from the locked grants of `currency`, what each period of `windows` (priced in it)
/// consumed up to `expiry` and no settlement in that denomination paid yet. Returns the wallets
/// that paid and the value settled.
#[allow(clippy::too_many_arguments)]
async fn settle_expiry_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    grants: &mut LockedGrants,
    windows: &[PlannedWindow],
    pinned: &PinnedPrices,
    expiry: DateTime<Utc>,
    usage: &HashMap<UsageKey, Vec<GroupedUsageData>>,
    ending: &[CreditGrantId],
    cover: UnitCover<'_>,
) -> StoreResult<(Vec<WalletId>, i64)> {
    let mut touched = Vec::new();
    let mut total_settled = 0_i64;
    for (subscription_id, period_windows) in by_period(windows).values() {
        let period_start = period_windows[0].period.start;
        // Where its usage lines end: an immediate cancellation reads its last day to the instant.
        let dated_before = period_windows
            .iter()
            .map(|window| window.end.and_utc())
            .max()
            .unwrap_or_else(|| period_windows[0].end.and_utc());
        // Only usage the lapsing credit could pay settles now; the rest waits for billing order.
        if !lapsing_credit_pays(grants, expiry, *subscription_id, dated_before, ending) {
            continue;
        }
        let mut owed: Vec<UsageOwed> = Vec::new();
        let mut credits: Vec<(&PlannedWindow, i64)> = Vec::new();
        for window in period_windows {
            let Some(until) = window_until(window, expiry) else {
                continue;
            };
            let groups = usage[&(
                window.metric.id,
                window.metric.usage_scope.fingerprint(),
                window.start,
                until,
            )]
                .clone();
            let amount = price_window_groups(window, until, groups, pinned)?;
            if window.currency == currency {
                add_owed(&mut owed, window, amount, &window.taxes)?;
            } else {
                credits.push((window, amount));
            }
        }
        for (window, money) in overage_owed(
            conn,
            tenant_id,
            customer_id,
            *subscription_id,
            (period_start, dated_before),
            &credits,
            cover,
            expiry,
        )
        .await?
        {
            let taxes = window
                .overage
                .as_ref()
                .map_or(&window.taxes, |overage| &overage.taxes);
            add_owed(&mut owed, window, money, taxes)?;
        }
        // Earlier settlements of the period are placed on the products their grants pay for;
        // only what is left of each product's usage settles now.
        let earlier = CreditUsageSettlementRow::list_for_period(
            conn,
            tenant_id,
            *subscription_id,
            period_start,
            currency,
        )
        .await?
        .iter()
        .map(|settlement| settlement.id.as_uuid())
        .collect::<Vec<_>>();
        let held = settlement_holders_tx(conn, tenant_id, &earlier).await?;
        let (paid_by, settled_now) = settle_usage_tx(
            conn,
            &UsageSettlementTerms {
                tenant_id,
                customer_id,
                subscription_id: *subscription_id,
                period_start,
                dated_before,
                bound: expiry,
            },
            &owed,
            &held,
            GrantPool::sequence(currency),
            grants,
            ending,
        )
        .await?;
        touched.extend(paid_by);
        total_settled = total_settled.saturating_add(settled_now);
    }
    Ok((touched, total_settled))
}

/// How many credits a unit wallet covers of a period's credit-priced usage, to find the money
/// overage a lapsing currency grant settles: credits cover the earliest usage, so the overage up
/// to a bound is what was consumed by then beyond that cover.
#[derive(Clone, Copy)]
pub(crate) enum UnitCover<'a> {
    /// The unit's open settlements of the period, then what its grants spendable just before the
    /// bound would cover.
    AtBound,
    /// What the invoice taking the period over covered, by subscription, period start and unit.
    Invoiced(&'a HashMap<(SubscriptionId, NaiveDate, String), i64>),
}

fn add_owed(
    owed: &mut Vec<UsageOwed>,
    window: &PlannedWindow,
    amount: i64,
    taxes: &[crate::domain::TaxDetail],
) -> StoreResult<()> {
    match owed
        .iter_mut()
        .find(|owed| owed.product_id == window.product_id)
    {
        Some(owed) => {
            owed.base_minor = owed
                .base_minor
                .checked_add(amount)
                .ok_or_else(|| invalid("usage exceeds the supported range"))?
        }
        None => owed.push(UsageOwed {
            product_id: window.product_id,
            base_minor: amount,
            tax_inclusive: window.tax_inclusive,
            taxes: taxes.to_vec(),
        }),
    }
    Ok(())
}

/// The money each credit-priced window of one subscription's period owes for the credits
/// (`consumed` by the bound) its unit wallet does not cover, split and rounded as its invoice
/// lines are (`allocate_covered`, `overage_money`).
#[allow(clippy::too_many_arguments)]
async fn overage_owed<'w>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    (period_start, dated_before): (NaiveDate, DateTime<Utc>),
    consumed: &[(&'w PlannedWindow, i64)],
    cover: UnitCover<'_>,
    bound: DateTime<Utc>,
) -> StoreResult<Vec<(&'w PlannedWindow, i64)>> {
    let mut owed = Vec::new();
    let mut units = consumed
        .iter()
        .map(|(window, _)| window.currency.clone())
        .collect::<Vec<_>>();
    units.sort();
    units.dedup();
    for unit in units {
        let windows = consumed
            .iter()
            .filter(|(window, _)| window.currency == unit && window.overage.is_some())
            .collect::<Vec<_>>();
        let used = windows
            .iter()
            .map(|(_, amount)| (*amount).max(0))
            .collect::<Vec<_>>();
        let total = used
            .iter()
            .try_fold(0_i64, |sum, amount| sum.checked_add(*amount))
            .ok_or_else(|| invalid("credit usage exceeds the supported range"))?;
        if total <= 0 {
            continue;
        }
        let covered = match cover {
            UnitCover::Invoiced(covered) => covered
                .get(&(subscription_id, period_start, unit.clone()))
                .copied()
                .unwrap_or(0),
            UnitCover::AtBound => {
                let settled = CreditUsageSettlementRow::list_for_period(
                    conn,
                    tenant_id,
                    subscription_id,
                    period_start,
                    &unit,
                )
                .await?
                .iter()
                .filter(|settlement| settlement.invoice_id.is_none())
                .try_fold(0_i64, |sum, settlement| sum.checked_add(settlement.amount))
                .ok_or_else(|| invalid("settled credit exceeds the supported range"))?;
                let rest = total - settled.min(total);
                let fresh = if rest > 0 {
                    crate::repositories::credit::customer_credit::preview_tx(
                        conn,
                        tenant_id,
                        customer_id,
                        &unit,
                        GrantPool::All,
                        &crate::repositories::credit::allocation::CreditRequest::usage_for(
                            Some(subscription_id),
                            rest,
                            Some(dated_before),
                        ),
                        bound - Duration::microseconds(1),
                    )
                    .await?
                    .total()?
                } else {
                    0
                };
                settled.saturating_add(fresh)
            }
        };
        let shares = crate::services::invoice_lines::credit_units::allocate_covered(
            &used,
            covered.clamp(0, total),
        );
        for (((window, _), used), share) in windows.into_iter().zip(used).zip(shares) {
            let Some(overage) = window.overage.as_ref() else {
                continue;
            };
            let money = crate::domain::credit_units::beyond_balance_money_minor(
                used - share,
                window.precision,
                overage.rate,
                overage.precision,
            )?;
            owed.push((*window, money));
        }
    }
    Ok(owed)
}

/// Whether a grant lapsing at `expiry` (or, when given, one a plan change ends then), with value
/// spendable just before it, can pay `subscription_id`'s usage dated before `dated_before`: one
/// of that subscription, or a customer-wide one, whose usage start is before then.
fn lapsing_credit_pays(
    grants: &LockedGrants,
    expiry: DateTime<Utc>,
    subscription_id: SubscriptionId,
    dated_before: DateTime<Utc>,
    ending: &[CreditGrantId],
) -> bool {
    grants.iter().any(|scoped| {
        let grant = &scoped.grant;
        let lapsing = if ending.is_empty() {
            grant.lapses_at == Some(expiry)
        } else {
            ending.contains(&grant.id)
        };
        lapsing
            && grant.remaining > 0
            && grant.effective_at < expiry
            && grant.usage_start() < dated_before
            && grant
                .subscription_id
                .is_none_or(|own| own == subscription_id)
    })
}

/// Pays the usage of the invoice's periods that credit already expired covered, before the
/// invoice draws: the sweep would find those periods closed and lapse the credit unpaid. Expired
/// credit whose rollover could pay the invoice is rolled over first (`roll_over_reaching_tx`).
/// The caller holds the customer's grant locks (`lock_grants_tx`) and `pinned` holds the prices
/// the invoice bills.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn settle_due_expiries_tx(
    store: &Store,
    usage_client: &dyn UsageClient,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grants: &mut LockedGrants,
    windows: &[PlannedWindow],
    pinned: &PinnedPrices,
    currency: &str,
    cover: UnitCover<'_>,
    time: MovementTime,
) -> StoreResult<()> {
    if windows.is_empty() {
        return Ok(());
    }
    debug_assert!(windows.iter().all(|window| window.bills_money_in(currency)));
    // Boxed: the open periods it may read are a deep future for every invoice finalization.
    Box::pin(roll_over_reaching_tx(
        store,
        usage_client,
        conn,
        tenant_id,
        customer_id,
        grants,
        windows,
        pinned,
        currency,
        time,
    ))
    .await?;
    let expiries = lapsed_expiries(grants, time.at);
    if expiries.is_empty() {
        return Ok(());
    }
    let usage = read_usage(usage_client, tenant_id, customer_id, windows, &expiries).await?;
    let mut touched = Vec::new();
    let mut settled_minor = 0_i64;
    for expiry in expiries {
        let (paid_by, settled) = settle_expiry_tx(
            conn,
            tenant_id,
            customer_id,
            currency,
            grants,
            windows,
            pinned,
            expiry,
            &usage,
            &[],
            cover,
        )
        .await?;
        settled_minor = settled_minor.saturating_add(settled);
        touched.extend(paid_by);
    }
    balance_changed_tx(conn, &*store.hooks.credit, tenant_id, &touched, time.now).await?;
    metrics::expiry_settled(
        SettlementSource::Invoice,
        currency,
        touched.len(),
        settled_minor,
    );
    Ok(())
}

/// Whether `grant` expired by `at` holding value its rollover would carry into a grant spendable
/// during one of `windows` it may pay. Whether it already rolled is the caller's to check.
fn rollover_reaches(
    grant: &CustomerCreditGrantRow,
    windows: &[PlannedWindow],
    at: DateTime<Utc>,
) -> StoreResult<bool> {
    let (Some(rollover), Some(expired_at)) = (grant.rollover.as_ref(), grant.expires_at) else {
        return Ok(false);
    };
    if grant.remaining <= 0 || expired_at > at || grant.lapses_at != Some(expired_at) {
        return Ok(false);
    }
    let Some(rolled_until) = GrantRollover::from_json(rollover)?.rolled_expiry(expired_at) else {
        return Ok(false);
    };
    let (from, until) = (expired_at.naive_utc(), rolled_until.naive_utc());
    Ok(windows.iter().any(|window| {
        grant
            .subscription_id
            .is_none_or(|own| own == window.subscription_id)
            && window.end > from
            && window.start < until
    }))
}

/// Rolls over, before an invoice draws, the expired credit the sweep has not rolled yet whose
/// rollover could pay the invoice (catch-up or backdated invoicing, a lagging sweep, a period
/// shorter than the grace). Every lapse up to the last of them is settled in order and the credit
/// due to roll over then rolls, as the sweep would have done on time; lapsing the rest is the
/// sweep's.
#[allow(clippy::too_many_arguments)]
async fn roll_over_reaching_tx(
    store: &Store,
    usage_client: &dyn UsageClient,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grants: &mut LockedGrants,
    windows: &[PlannedWindow],
    pinned: &PinnedPrices,
    currency: &str,
    time: MovementTime,
) -> StoreResult<()> {
    let mut last = None;
    let mut subscriptions = HashSet::new();
    for scoped in grants.iter() {
        if rollover_reaches(&scoped.grant, windows, time.at)?
            && awaits_rollover_tx(conn, &scoped.grant).await?
        {
            last = last.max(scoped.grant.lapses_at);
            subscriptions.extend(scoped.grant.subscription_id);
        }
    }
    let Some(last) = last else {
        return Ok(());
    };
    // Customer-wide credit pays every subscription in the sweep's order: all their periods settle.
    let only = (!grants
        .iter()
        .any(|scoped| scoped.grant.subscription_id.is_none() && scoped.grant.remaining > 0))
    .then_some(subscriptions);
    let in_scope = |subscription_id: Option<SubscriptionId>| {
        only.as_ref().is_none_or(|only| {
            subscription_id.is_some_and(|subscription_id| only.contains(&subscription_id))
        })
    };

    // The invoice's own windows stand for its periods, whether or not they are still open.
    let invoiced = windows
        .iter()
        .map(|window| (window.subscription_id, window.period.start))
        .collect::<HashSet<_>>();
    let (open, mut prices) = open_windows_of(
        store,
        conn,
        tenant_id,
        customer_id,
        currency,
        only.as_ref(),
        time.now,
    )
    .await?;
    prices.extend(pinned);
    let paid = open
        .into_iter()
        .filter(|window| !invoiced.contains(&(window.subscription_id, window.period.start)))
        .chain(
            windows
                .iter()
                .filter(|window| in_scope(Some(window.subscription_id)))
                .cloned(),
        )
        .collect::<Vec<_>>();

    let mut usage = HashMap::new();
    let mut touched = Vec::new();
    let mut settlements = 0_usize;
    let mut settled_minor = 0_i64;
    let mut rolled = 0_usize;
    let mut rolled_minor = 0_i64;
    let mut settled_until = None;
    while let Some(expiry) = grants
        .iter()
        .filter(|scoped| scoped.grant.remaining > 0)
        .filter_map(|scoped| scoped.grant.lapses_at)
        .filter(|at| *at <= last && settled_until.is_none_or(|until| *at > until))
        .min()
    {
        settled_until = Some(expiry);
        for (key, window) in usage_keys(&paid, &[expiry]) {
            if let Entry::Vacant(entry) = usage.entry(key) {
                let read = usage_client
                    .fetch_usage(
                        &tenant_id,
                        &customer_id,
                        &window.metric,
                        UsagePeriod {
                            start: entry.key().2,
                            end: entry.key().3,
                        },
                    )
                    .await?;
                entry.insert(read.data);
            }
        }
        let (paid_by, settled) = settle_expiry_tx(
            conn,
            tenant_id,
            customer_id,
            currency,
            grants,
            &paid,
            &prices,
            expiry,
            &usage,
            &[],
            UnitCover::AtBound,
        )
        .await?;
        settlements += paid_by.len();
        settled_minor = settled_minor.saturating_add(settled);
        touched.extend(paid_by);
        let mut rolled_over = Vec::new();
        for scoped in grants.iter_mut() {
            let grant = &scoped.grant;
            if grant.lapses_at != Some(expiry)
                || grant.expires_at != Some(expiry)
                || grant.remaining <= 0
                || !in_scope(grant.subscription_id)
                || !awaits_rollover_tx(conn, grant).await?
            {
                continue;
            }
            if let Some(rolled_into) =
                lapse_grant_tx(conn, &*store.hooks.credit, scoped, currency, time.now).await?
            {
                rolled_minor = rolled_minor.saturating_add(rolled_into.grant.amount);
                rolled_over.push(rolled_into);
            }
            touched.push(scoped.grant.account_id);
        }
        rolled += rolled_over.len();
        grants.extend(rolled_over);
    }
    balance_changed_tx(conn, &*store.hooks.credit, tenant_id, &touched, time.now).await?;
    metrics::expiry_settled(
        SettlementSource::Invoice,
        currency,
        settlements,
        settled_minor,
    );
    metrics::credit_rolled_over(currency, rolled, rolled_minor);
    Ok(())
}

/// The usage of a customer's open periods up to `bound`, in `currency`, read before the locks of
/// the change that ends grants at `bound`.
pub(crate) struct EndingUsage(HashMap<UsageKey, Vec<GroupedUsageData>>);

pub(crate) async fn read_ending_usage(
    store: &Store,
    usage_client: &dyn UsageClient,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    bound: DateTime<Utc>,
    now: DateTime<Utc>,
) -> StoreResult<EndingUsage> {
    let currency = normalized_currency(currency);
    let (windows, _) = open_windows(store, conn, tenant_id, customer_id, &currency, now).await?;
    Ok(EndingUsage(
        read_usage(usage_client, tenant_id, customer_id, &windows, &[bound]).await?,
    ))
}

/// Pays from `ending`, grants a plan change ends at `bound` before their expiry, the usage of the
/// open periods up to `bound`, as their expiry would: what they leave is what the change removes.
/// The caller holds the grant locks of `currency` (`lock_grants_tx`). Usage `read` before the
/// locks is used; a window it misses is read now.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn settle_ending_grants_tx(
    store: &Store,
    usage_client: &dyn UsageClient,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    grants: &mut LockedGrants,
    ending: &[CreditGrantId],
    bound: DateTime<Utc>,
    now: DateTime<Utc>,
    read: Option<&EndingUsage>,
) -> StoreResult<()> {
    if ending.is_empty() {
        return Ok(());
    }
    let currency = normalized_currency(currency);
    let (windows, pinned) =
        open_windows(store, conn, tenant_id, customer_id, &currency, now).await?;
    if windows.is_empty() {
        return Ok(());
    }
    let read_before = read.map(|read| &read.0);
    let usage = match read_before {
        Some(usage)
            if usage_keys(&windows, &[bound])
                .iter()
                .all(|(key, _)| usage.contains_key(key)) =>
        {
            usage.clone()
        }
        _ => read_usage(usage_client, tenant_id, customer_id, &windows, &[bound]).await?,
    };
    let (touched, _) = settle_expiry_tx(
        conn,
        tenant_id,
        customer_id,
        &currency,
        grants,
        &windows,
        &pinned,
        bound,
        &usage,
        ending,
        UnitCover::AtBound,
    )
    .await?;
    balance_changed_tx(conn, &*store.hooks.credit, tenant_id, &touched, now).await
}

/// Gives back the settlements open for at least `RETURN_AFTER` whose period no invoice can bill
/// any more, `page` customers at a time. A customer that fails is logged and retried later.
async fn return_orphaned_settlements(
    store: &Store,
    page: i64,
    now: DateTime<Utc>,
) -> StoreResult<usize> {
    let created_before = now - RETURN_AFTER;
    let mut returned = 0_usize;
    let mut after = None;
    loop {
        let mut conn = store.get_conn().await?;
        let customers = CreditUsageSettlementRow::list_customers_with_open_before(
            &mut conn,
            created_before,
            after,
            page,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        let full_page = i64::try_from(customers.len()).is_ok_and(|len| len == page);
        after = customers.last().copied();
        for (tenant_id, customer_id) in customers {
            match return_customer_orphans(store, tenant_id, customer_id, created_before, now).await
            {
                Ok(count) => returned += count,
                Err(error) => log::error!(
                    "usage settlements of customer {customer_id} were not reconciled: {error:?}"
                ),
            }
        }
        if !full_page {
            return Ok(returned);
        }
    }
}

async fn return_customer_orphans(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    created_before: DateTime<Utc>,
    now: DateTime<Utc>,
) -> StoreResult<usize> {
    let mut conn = store.get_conn().await?;
    let open =
        CreditUsageSettlementRow::list_open_for_customers(&mut conn, tenant_id, &[customer_id])
            .await?
            .into_iter()
            .filter(|settlement| settlement.created_at < created_before)
            .collect::<Vec<_>>();
    if open.is_empty() {
        return Ok(0);
    }
    let periods = open_periods(store, &mut conn, tenant_id, customer_id, now).await?;
    let mut terminal: HashMap<SubscriptionId, bool> = HashMap::new();
    let mut orphaned = Vec::new();
    for settlement in open {
        let key = (settlement.subscription_id, settlement.period_start);
        if periods.billable.contains(&key) {
            continue;
        }
        let ended = match terminal.entry(settlement.subscription_id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => *entry.insert(
                SubscriptionRow::get_status(&mut conn, &tenant_id, settlement.subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .is_terminal(),
            ),
        };
        let moved_on = periods.windows.iter().any(|window| {
            window.subscription_id == settlement.subscription_id
                && window.period.start > settlement.period_start
        });
        if ended || moved_on {
            orphaned.push(settlement);
        }
    }
    if orphaned.is_empty() {
        return Ok(0);
    }
    let currencies =
        CustomerCreditAccountRow::list_for_customers(&mut conn, tenant_id, &[customer_id])
            .await?
            .into_iter()
            .map(|account| (account.id, account.currency))
            .collect::<HashMap<_, _>>();
    drop(conn);

    let mut by_currency: BTreeMap<String, Vec<UsageSettlementId>> = BTreeMap::new();
    for settlement in orphaned {
        if let Some(currency) = currencies.get(&settlement.account_id) {
            by_currency
                .entry(currency.clone())
                .or_default()
                .push(settlement.id);
        }
    }
    let mut returned = 0_usize;
    for (currency, ids) in by_currency {
        let (currency, ids) = (&currency, &ids);
        let (count, amount) = store
            .transaction(|conn| {
                async move {
                    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
                    lock_grants_tx(conn, &lock, currency).await?;
                    let mut touched = Vec::new();
                    let mut amount = 0_i64;
                    for id in ids {
                        if let Some((wallet, given_back)) =
                            return_settlement_tx(conn, &*store.hooks.credit, &lock, *id, now)
                                .await?
                        {
                            touched.push(wallet);
                            amount = amount.saturating_add(given_back);
                        }
                    }
                    balance_changed_tx(conn, &*store.hooks.credit, tenant_id, &touched, now)
                        .await?;
                    Ok::<_, Report<StoreError>>((touched.len(), amount))
                }
                .scope_boxed()
            })
            .await?;
        metrics::settlements_returned(&normalized_currency(currency), count, amount);
        returned += count;
    }
    Ok(returned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_domain::ids::CreditGrantId;

    fn lapsing(
        subscription_id: Option<SubscriptionId>,
        remaining: i64,
        effective_at: DateTime<Utc>,
        lapses_at: DateTime<Utc>,
    ) -> ScopedGrantRow {
        ScopedGrantRow {
            grant: CustomerCreditGrantRow {
                id: CreditGrantId::new(),
                tenant_id: TenantId::new(),
                customer_id: CustomerId::new(),
                account_id: WalletId::new(),
                amount: 1_000,
                remaining,
                priority: 100,
                classification: "PROMOTIONAL".to_owned(),
                effective_at,
                expires_at: Some(lapses_at),
                source_kind: "INCLUDED_CREDIT".to_owned(),
                source_id: uuid::Uuid::now_v7(),
                invoice_id: None,
                line_local_id: None,
                credit_note_id: None,
                reason: "included".to_owned(),
                reference: None,
                created_by: None,
                created_at: effective_at,
                cost_minor: Some(0),
                cost_currency: Some("USD".to_owned()),
                rollover: None,
                subscription_id,
                application: "USAGE".to_owned(),
                ends_at: None,
                end_reason: None,
                lapses_at: Some(lapses_at),
                pays_usage_from: None,
            },
            product_ids: Vec::new(),
        }
    }

    /// Only credit that still holds spendable value at the expiry settles a subscription's usage
    /// then: a spent or not yet effective grant lapsing at the same instant settles nothing early.
    #[test]
    fn only_spendable_lapsing_credit_settles_a_subscription() {
        let at = |d| Utc.with_ymd_and_hms(2026, 4, d, 0, 0, 0).unwrap();
        let (a, b) = (SubscriptionId::new(), SubscriptionId::new());
        let grants = vec![
            lapsing(Some(a), 1_000, at(1) - Duration::days(31), at(1)),
            lapsing(Some(b), 0, at(1) - Duration::days(31), at(1)),
            lapsing(Some(b), 1_000, at(1), at(1)),
        ];
        let end = at(1);
        assert!(lapsing_credit_pays(&grants, at(1), a, end, &[]));
        assert!(
            !lapsing_credit_pays(&grants, at(1), b, end, &[]),
            "B's credit is spent or not yet due"
        );
        let mut shared = grants.clone();
        shared.push(lapsing(None, 500, at(1) - Duration::days(31), at(1)));
        assert!(
            lapsing_credit_pays(&shared, at(1), b, end, &[]),
            "customer-wide credit pays B"
        );
        assert!(
            !lapsing_credit_pays(&grants, at(2), a, end, &[]),
            "nothing lapses then"
        );
        assert!(
            !lapsing_credit_pays(&grants, at(1), a, at(1) - Duration::days(31), &[]),
            "credit effective once a period ended never settles its usage"
        );
    }
}

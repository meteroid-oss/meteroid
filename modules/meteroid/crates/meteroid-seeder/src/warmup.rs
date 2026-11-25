//! Makes a seeded sandbox read like a live business once its backdated history is billed: settles
//! aged invoices (leaving a realistic receivables tail), fails one payment, issues one credit note,
//! applies one expansion and freshens recent usage. Every step is idempotent and Sandbox-only, so
//! an edition may also rerun the finalize pass periodically.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, InvoiceId, SubscriptionId, TenantId,
};
use meteroid_store::StoreResult;
use meteroid_store::domain::enums::{
    BillingMetricAggregateEnum, CreditNoteStatus, InvoicePaymentStatus, InvoiceStatusEnum,
    TenantStatusEnum,
};
use meteroid_store::domain::rate_cards::rates_at;
use meteroid_store::domain::sandbox_seeds::SandboxSeedStatus;
use meteroid_store::domain::{BillableMetricMeta, PaginationRequest, Tenant};
use meteroid_store::hooks::AnalyticsHook;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditNoteInterface, CreditType,
};
use meteroid_store::repositories::sandbox::SandboxInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::tenants::TenantInterface;
use meteroid_store::repositories::{
    CustomersInterface, InvoiceInterface, RateCardsInterface, SubscriptionInterface,
};
use meteroid_store::{Services, Store};
use rust_decimal::Decimal;

use crate::presets::STAGED_USAGE_CUSTOMERS;
use crate::presets::credit_traffic::TOKENS_METRIC;
use crate::usage;

const PAGE_SIZE: u32 = 100;
/// Invoices whose due date is at least this many days in the past are settled;
/// more recent ones stay open so the sandbox shows a realistic AR balance.
const COLLECTED_AFTER_DAYS: i64 = 5;
/// Roughly one aged invoice in this many is left unpaid, for AR-aging buckets.
const OVERDUE_EVERY: u64 = 12;
/// Only sandboxes a user has loaded within this many days get usage top-ups.
const FRESH_ACTIVE_DAYS: i64 = 7;
/// Usage top-up regenerates the trailing few days so recent charts stay live.
const FRESH_WINDOW_DAYS: i64 = 3;
/// Modest per-customer daily event volume for freshness (backfill is denser).
const FRESH_DAILY_VOLUME: u32 = 6;
/// Units per event of a metric a rate card rates, enough to bill at its per-1M-unit rates.
const RATED_MIN_UNITS: u64 = 1_000;
const RATED_MAX_UNITS: u64 = 20_000;
/// Hard ceiling on events emitted per tenant per freshen pass, independent of
/// customer/metric count. Bounds sandbox ingest load (~100/day over the window);
/// oversized tenants are evenly downsampled to this.
const FRESH_MAX_EVENTS_PER_TENANT: usize = 100 * (FRESH_WINDOW_DAYS as usize + 1);

/// One tenant's full finalize pass — settle invoices, credit note, expansion, freshen,
/// and (with a hook) re-derive analytics. The post-seed warmup passes a hook; a periodic
/// rerun passes `None` and sets `skip_freshen` once the tenant was freshened that day.
/// Skips non-Active tenants; returns invoices settled.
pub async fn finalize_tenant(
    store: &Arc<Store>,
    services: &Arc<Services>,
    tenant: &Tenant,
    today: NaiveDate,
    analytics: Option<&Arc<dyn AnalyticsHook>>,
    skip_freshen: bool,
) -> usize {
    if tenant.status != TenantStatusEnum::Active {
        return 0;
    }
    // A seed still running, or one that failed, is left alone.
    let seeded = match store.get_sandbox_seed(tenant.id).await {
        Ok(seed) => seed.is_none_or(|seed| seed.status == SandboxSeedStatus::Completed),
        Err(e) => {
            log::warn!(
                "sandbox finalize: tenant {} seed status lookup failed: {e:?}",
                tenant.id
            );
            false
        }
    };
    // A credit tenant's replayed books are kept as they are: of the demo steps, only paying its
    // aged sent invoices still applies.
    let settled = if !seeded {
        0
    } else {
        match is_credit_tenant(store, tenant.id).await {
            Ok(true) => {
                match settle_tenant(store, services, tenant.id, today, Collection::SentInvoices)
                    .await
                {
                    Ok(n) => n,
                    Err(e) => {
                        log::warn!(
                            "sandbox finalize: tenant {} settle failed: {e:?}",
                            tenant.id
                        );
                        0
                    }
                }
            }
            Ok(false) => run_demo_steps(store, services, tenant, today, skip_freshen).await,
            Err(e) => {
                log::warn!(
                    "sandbox finalize: tenant {} kind lookup failed: {e:?}",
                    tenant.id
                );
                0
            }
        }
    };
    if let Some(analytics) = analytics {
        analytics
            .tenant_history_written(store, tenant.id, today - ChronoDuration::days(366))
            .await;
    }
    settled
}

async fn run_demo_steps(
    store: &Arc<Store>,
    services: &Arc<Services>,
    tenant: &Tenant,
    today: NaiveDate,
    skip_freshen: bool,
) -> usize {
    let settled = match settle_tenant(store, services, tenant.id, today, Collection::All).await {
        Ok(n) => n,
        Err(e) => {
            log::warn!(
                "sandbox finalize: tenant {} settle failed: {e:?}",
                tenant.id
            );
            0
        }
    };
    if let Err(e) = stage_demo_payment_failure(store, tenant.id).await {
        log::warn!(
            "sandbox finalize: tenant {} payment failure staging failed: {e:?}",
            tenant.id
        );
    }
    if let Err(e) = issue_credit_note(store, tenant.id).await {
        log::warn!(
            "sandbox finalize: tenant {} credit note failed: {e:?}",
            tenant.id
        );
    }
    if let Err(e) = apply_demo_expansion(store, services, tenant.id).await {
        log::warn!(
            "sandbox finalize: tenant {} expansion failed: {e:?}",
            tenant.id
        );
    }
    if !skip_freshen && let Err(e) = freshen(store, services, tenant, today).await {
        log::warn!(
            "sandbox finalize: tenant {} freshen failed: {e:?}",
            tenant.id
        );
    }
    settled
}

/// After a seed, wait (bounded) for catch-up, then finalize and hand the tenant to the
/// edition's analytics, so AR/steering surfaces are live as soon as the history is billed.
/// Best-effort; failures are logged, never propagated.
pub fn spawn_sandbox_warmup(
    store: Arc<Store>,
    services: Arc<Services>,
    analytics: Arc<dyn AnalyticsHook>,
    tenant_id: TenantId,
) {
    tokio::spawn(async move {
        if let Err(e) = warm_up_sandbox(&store, &services, &analytics, tenant_id).await {
            log::warn!("sandbox warmup failed for tenant {tenant_id}: {e:?}");
        }
    });
}

#[tracing::instrument(skip_all, fields(tenant_id = %tenant_id))]
pub async fn warm_up_sandbox(
    store: &Arc<Store>,
    services: &Arc<Services>,
    analytics: &Arc<dyn AnalyticsHook>,
    tenant_id: TenantId,
) -> StoreResult<()> {
    const POLL: Duration = Duration::from_secs(5);
    const MAX_ATTEMPTS: usize = 180; // ~15 min ceiling
    const SUBS_GRACE_ATTEMPTS: usize = 12; // ~60s for seeding to create the subscriptions

    // The seed runs in the background: its data is complete once it finishes.
    for _ in 0..MAX_ATTEMPTS {
        match store.get_sandbox_seed(tenant_id).await {
            Ok(Some(seed)) if seed.status == SandboxSeedStatus::InProgress => {
                tokio::time::sleep(POLL).await
            }
            _ => break,
        }
    }

    // Wait until the backdated subs exist (pending > 0) then catch up (back to 0); the
    // grace bounds the "before subs exist" window so a sub-less scenario can't stall.
    let mut saw_pending = false;
    for attempt in 0..MAX_ATTEMPTS {
        let pending = store
            .count_subscriptions_pending_catchup(tenant_id)
            .await
            .unwrap_or(0);
        if pending > 0 {
            saw_pending = true;
        } else if saw_pending || attempt >= SUBS_GRACE_ATTEMPTS {
            break;
        }
        tokio::time::sleep(POLL).await;
    }

    let today = Utc::now().naive_utc().date();
    let tenant = store.find_tenant_by_id(tenant_id).await?.tenant;
    finalize_tenant(store, services, &tenant, today, Some(analytics), false).await;

    analytics
        .sandbox_warmed(store, services, tenant_id, today)
        .await;
    Ok(())
}

/// A Halyard AI (credits) sandbox, whose replayed books only get their sent invoices paid.
pub async fn is_credit_tenant(store: &Store, tenant_id: TenantId) -> StoreResult<bool> {
    let metrics = store
        .list_billable_metrics(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
            None,
            Some(false),
            None,
            None,
        )
        .await?;
    Ok(metrics.items.iter().any(|m| m.code == TOKENS_METRIC))
}

/// Keep recently-loaded sandboxes' usage charts live by regenerating the trailing
/// few days of events. Gated on `last_active_at` so abandoned sandboxes decay
/// instead of accruing perpetual ingest load.
async fn freshen(
    store: &Arc<Store>,
    services: &Arc<Services>,
    tenant: &Tenant,
    today: NaiveDate,
) -> StoreResult<usize> {
    let active = tenant
        .last_active_at
        .map(|t| (today - t.date()).num_days() <= FRESH_ACTIVE_DAYS)
        .unwrap_or(false);
    if !active {
        return Ok(0);
    }

    let metrics = store
        .list_billable_metrics(
            tenant.id,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
            None,
            Some(false),
            None,
            None,
        )
        .await?;
    let customers = store
        .list_customers(
            tenant.id,
            PaginationRequest {
                page: 0,
                per_page: Some(200),
            },
            None,
            None,
            Some(false),
        )
        .await?;
    if metrics.items.is_empty() || customers.items.is_empty() {
        return Ok(0);
    }

    let rated = rated_dimensions(store, tenant.id, today).await?;
    let streams: Vec<usage::UsageStream> = metrics
        .items
        .iter()
        .map(|metric| fresh_stream(metric, rated.get(&metric.id)))
        .collect();
    let customers: Vec<(CustomerId, &str)> = customers
        .items
        .iter()
        .map(|c| (c.id, c.name.as_str()))
        .collect();
    let start = today - ChronoDuration::days(FRESH_WINDOW_DAYS);
    let mut events: Vec<_> = freshen_events(&streams, &customers, start, today)
        .into_iter()
        .map(usage::to_ingest_event)
        .collect();

    if events.is_empty() {
        return Ok(0);
    }
    // Hard ceiling regardless of customer/metric count: evenly downsample oversized
    // tenants so a single pass can't flood ingestion.
    if events.len() > FRESH_MAX_EVENTS_PER_TENANT {
        let stride = events.len() / FRESH_MAX_EVENTS_PER_TENANT;
        events = events
            .into_iter()
            .step_by(stride)
            .take(FRESH_MAX_EVENTS_PER_TENANT)
            .collect();
    }
    Ok(usage::ingest_all(
        services.usage_clients(),
        tenant.id,
        tenant.keep_last_events,
        events,
    )
    .await)
}

/// The event properties of each metric a rate card rates, from the first card rating it.
async fn rated_dimensions(
    store: &Store,
    tenant_id: TenantId,
    today: NaiveDate,
) -> StoreResult<HashMap<BillableMetricId, Vec<usage::Dimension>>> {
    let mut rated = HashMap::new();
    for card in store.list_rate_cards(tenant_id, None).await? {
        if rated.contains_key(&card.metric_id) {
            continue;
        }
        let (card, entries) = store.get_rate_card(tenant_id, card.id).await?;
        let keys: Vec<Vec<String>> = rates_at(&entries, today)
            .iter()
            .map(|entry| entry.key_text())
            .collect();
        rated.insert(card.metric_id, card_dimensions(&card.dimensions, &keys));
    }
    Ok(rated)
}

/// Per dimension of a card, the values its `keys` name, equally often, and one no key names at
/// a tenth of the events, so the card's `*` row bills too.
fn card_dimensions(dimensions: &[String], keys: &[Vec<String>]) -> Vec<usage::Dimension> {
    dimensions
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let mut values: Vec<String> = Vec::new();
            for pattern in keys.iter().filter_map(|k| k.get(i)) {
                if !pattern.ends_with('*') && !values.contains(pattern) {
                    values.push(pattern.clone());
                }
            }
            let named = values.len() as u32;
            let mut weights = vec![9; values.len()];
            let unnamed = (0..)
                .map(|n| match n {
                    0 => "other".to_string(),
                    n => format!("other-{n}"),
                })
                .find(|value| !values.contains(value))
                .expect("a value no key names");
            values.push(unnamed);
            weights.push(named.max(1));
            usage::Dimension {
                key: key.clone(),
                values,
                weights,
            }
        })
        .collect()
}

fn fresh_stream(
    metric: &BillableMetricMeta,
    rated: Option<&Vec<usage::Dimension>>,
) -> usage::UsageStream {
    let value_key = metric
        .aggregation_key
        .clone()
        .unwrap_or_else(|| "value".to_string());
    let is_gauge = matches!(
        metric.aggregation_type,
        BillingMetricAggregateEnum::Max
            | BillingMetricAggregateEnum::Min
            | BillingMetricAggregateEnum::Mean
            | BillingMetricAggregateEnum::Latest
    );
    let stream = usage::stream_for(&metric.code, &value_key, is_gauge, FRESH_DAILY_VOLUME);
    match rated {
        Some(dimensions) => {
            stream.with_events(RATED_MIN_UNITS, RATED_MAX_UNITS, dimensions.clone())
        }
        None => stream,
    }
}

/// Trailing-window events for every customer except the staged ones, whose seeded
/// cycle usage is what their account signals are computed from.
fn freshen_events(
    streams: &[usage::UsageStream],
    customers: &[(CustomerId, &str)],
    start: NaiveDate,
    today: NaiveDate,
) -> Vec<usage::GeneratedEvent> {
    streams
        .iter()
        .flat_map(|stream| {
            let ucs: Vec<usage::UsageCustomer> = customers
                .iter()
                .filter(|(_, name)| !has_staged_usage(name))
                .map(|(id, _)| usage::UsageCustomer {
                    customer_id: *id,
                    scale: usage::scale_for(*id),
                    start,
                    end: today,
                    seed: usage::seed_for(*id, &stream.metric_code),
                })
                .collect();
            usage::generate_stream(stream, &ucs)
        })
        .collect()
}

fn has_staged_usage(customer_name: &str) -> bool {
    STAGED_USAGE_CUSTOMERS.contains(&customer_name)
}

/// Fails the payment of the latest open invoice once per tenant, so the payment_failing
/// signal has a live example. Runs after settlement, which only settles UNPAID invoices.
async fn stage_demo_payment_failure(store: &Arc<Store>, tenant_id: TenantId) -> StoreResult<()> {
    let errored = store
        .list_invoices(
            tenant_id,
            None,
            None,
            None,
            Some(InvoicePaymentStatus::Errored),
            None,
            false,
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(1),
            },
        )
        .await?;
    if !errored.items.is_empty() {
        return Ok(());
    }

    let mut open = Vec::new();
    let mut page = 0u32;
    loop {
        let res = store
            .list_invoices(
                tenant_id,
                None,
                None,
                Some(InvoiceStatusEnum::Finalized),
                Some(InvoicePaymentStatus::Unpaid),
                None,
                false,
                None,
                vec![],
                None,
                PaginationRequest {
                    page,
                    per_page: Some(PAGE_SIZE),
                },
            )
            .await?;
        let count = res.items.len();
        open.extend(res.items.into_iter().map(|iwc| {
            (
                iwc.invoice.id,
                iwc.invoice.invoice_date,
                iwc.invoice.amount_due,
            )
        }));
        if count < PAGE_SIZE as usize {
            break;
        }
        page += 1;
    }

    let Some(invoice_id) = payment_failure_target(&open) else {
        return Ok(());
    };
    store
        .stage_sandbox_payment_failure(tenant_id, invoice_id)
        .await?;
    log::info!(
        "sandbox finalize: staged a failed payment on invoice {invoice_id} for tenant {tenant_id}"
    );
    Ok(())
}

/// The latest invoice with an amount due; the lowest id breaks a same-day tie.
fn payment_failure_target(open: &[(InvoiceId, NaiveDate, i64)]) -> Option<InvoiceId> {
    open.iter()
        .filter(|(_, _, amount_due)| *amount_due > 0)
        .max_by_key(|(id, date, _)| (*date, std::cmp::Reverse(id.as_uuid())))
        .map(|(id, _, _)| *id)
}

/// Which aged invoices the settle step pays.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Collection {
    /// Every one, but a slice left overdue for AR aging.
    All,
    /// Only those of subscriptions that send their invoices: the others are charged to a card.
    SentInvoices,
}

async fn settle_tenant(
    store: &Arc<Store>,
    services: &Arc<Services>,
    tenant_id: TenantId,
    today: NaiveDate,
    collection: Collection,
) -> StoreResult<usize> {
    let mut page = 0u32;
    let mut settled = 0usize;
    let mut charged_automatically: HashMap<SubscriptionId, bool> = HashMap::new();
    loop {
        let res = store
            .list_invoices(
                tenant_id,
                None,
                None,
                Some(InvoiceStatusEnum::Finalized),
                None,
                None,
                false,
                None,
                vec![],
                None,
                PaginationRequest {
                    page,
                    per_page: Some(PAGE_SIZE),
                },
            )
            .await?;
        let count = res.items.len();

        for iwc in res.items {
            let inv = iwc.invoice;
            if inv.payment_status != InvoicePaymentStatus::Unpaid || inv.amount_due <= 0 {
                continue;
            }
            let due = inv.due_at.map(|d| d.date()).unwrap_or(inv.invoice_date);
            if due > today - ChronoDuration::days(COLLECTED_AFTER_DAYS) {
                continue; // recent: keep it as an open receivable
            }
            match collection {
                Collection::All if leave_overdue(&inv.id) => {
                    continue; // aged: leave a slice unpaid for AR aging
                }
                Collection::All => {}
                Collection::SentInvoices => {
                    let Some(subscription_id) = inv.subscription_id else {
                        continue;
                    };
                    let charged = match charged_automatically.get(&subscription_id) {
                        Some(charged) => *charged,
                        None => match store.get_subscription(tenant_id, subscription_id).await {
                            Ok(subscription) => {
                                charged_automatically
                                    .insert(subscription_id, subscription.charge_automatically);
                                subscription.charge_automatically
                            }
                            Err(e) => {
                                log::warn!(
                                    "sandbox finalize: subscription {subscription_id} of \
                                     invoice {} unreadable: {e:?}",
                                    inv.id
                                );
                                continue;
                            }
                        },
                    };
                    if charged {
                        continue;
                    }
                }
            }

            // Sandbox archetype currencies are all 2-decimal, so cents / 100 is exact;
            // a mismatch would only make mark_invoice_as_paid a no-op, never corrupt data.
            let amount = Decimal::from(inv.amount_due) / Decimal::from(100);
            let pay_date = (due + ChronoDuration::days(2))
                .min(today)
                .and_hms_opt(12, 0, 0)
                .expect("valid time")
                .and_utc()
                .fixed_offset();

            match services
                .mark_invoice_as_paid(
                    Actor::System,
                    tenant_id,
                    inv.id,
                    amount,
                    pay_date,
                    Some("Sandbox seed payment".to_string()),
                )
                .await
            {
                Ok(_) => settled += 1,
                Err(e) => log::debug!("sandbox finalize: mark paid failed for {}: {e:?}", inv.id),
            }
        }

        if count < PAGE_SIZE as usize {
            break;
        }
        page += 1;
    }
    Ok(settled)
}

/// Issue exactly one credit note per sandbox tenant, once a finalized+paid invoice
/// exists, so the Credit Notes surface isn't empty. Idempotent: skips the tenant
/// if any non-voided credit note already exists. A full-value `Refund` against a
/// paid invoice models a goodwill service credit and leaves the invoice paid.
async fn issue_credit_note(store: &Arc<Store>, tenant_id: TenantId) -> StoreResult<()> {
    let existing = store
        .list_credit_notes(
            tenant_id,
            None,
            None,
            None,
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
        )
        .await?;
    if existing
        .items
        .iter()
        .any(|cn| cn.status != CreditNoteStatus::Voided)
    {
        return Ok(());
    }

    let invoices = store
        .list_invoices(
            tenant_id,
            None,
            None,
            Some(InvoiceStatusEnum::Finalized),
            None,
            None,
            false,
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(PAGE_SIZE),
            },
        )
        .await?;

    let Some(target) = invoices.items.into_iter().find(|iwc| {
        iwc.invoice.payment_status == InvoicePaymentStatus::Paid && iwc.invoice.total > 0
    }) else {
        return Ok(());
    };

    store
        .create_and_finalize_credit_note(
            Actor::System,
            tenant_id,
            CreateCreditNoteParams {
                invoice_id: target.invoice.id,
                line_items: vec![],
                reason: Some("Billing adjustment".to_string()),
                memo: Some("Goodwill service credit (sandbox seed).".to_string()),
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await?;
    log::info!("sandbox finalize: issued credit note for tenant {tenant_id}");
    Ok(())
}

/// Apply one demo expansion per tenant so the Expansion MRR report isn't empty:
/// add an add-on to a single deterministic active subscription that lacks it. Runs
/// here (not in the seeder) because in-place amendments require the subscription to
/// be caught up to its current period, which only holds after the billing worker
/// has processed the backdated periods. Idempotent — it targets one fixed
/// (subscription, add-on) pair and no-ops once that add-on is present. Staged
/// accounts are left out so their billing cycle stays the one their usage was staged in.
async fn apply_demo_expansion(
    store: &Arc<Store>,
    services: &Arc<Services>,
    tenant_id: TenantId,
) -> StoreResult<()> {
    use meteroid_store::domain::enums::SubscriptionStatusEnum;
    use meteroid_store::domain::subscription_add_ons::{
        CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
    };
    use meteroid_store::domain::subscription_amendment::{AddOnChanges, SubscriptionAmendment};
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let add_ons = store
        .list_add_ons(
            tenant_id,
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(50),
            },
            None,
            None,
            false,
            None,
        )
        .await?;
    let Some(add_on_id) = add_ons
        .items
        .iter()
        .map(|a| a.id)
        .min_by_key(|id| id.as_uuid())
    else {
        return Ok(());
    };

    let subs = store
        .list_subscriptions(
            tenant_id,
            None,
            None,
            Some(vec![SubscriptionStatusEnum::Active]),
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
        )
        .await?;
    let Some(subscription_id) = subs
        .items
        .iter()
        .filter(|s| !has_staged_usage(&s.customer_name))
        .map(|s| s.id)
        .min_by_key(|id| id.as_uuid())
    else {
        return Ok(());
    };

    // Idempotent: only add if this fixed target doesn't already carry the add-on.
    let details = store
        .get_subscription_details(tenant_id, subscription_id)
        .await?;
    if details.add_ons.iter().any(|a| a.add_on_id == add_on_id) {
        return Ok(());
    }

    let amendment = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: Default::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
            ..Default::default()
        },
    };
    services
        .apply_amendment_immediate(Actor::System, subscription_id, tenant_id, amendment)
        .await?;
    log::info!("sandbox finalize: seeded an expansion amendment for tenant {tenant_id}");
    Ok(())
}

fn leave_overdue(id: &InvoiceId) -> bool {
    fnv1a(&format!("{id:?}")).is_multiple_of(OVERDUE_EVERY)
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer(n: u128) -> CustomerId {
        CustomerId::from_const(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn freshen_skips_staged_customers() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let start = today - ChronoDuration::days(FRESH_WINDOW_DAYS);
        let streams = vec![usage::stream_for(
            "ai_actions",
            "value",
            false,
            FRESH_DAILY_VOLUME,
        )];
        let (staged, other) = (customer(1), customer(2));
        let customers: Vec<(CustomerId, &str)> = STAGED_USAGE_CUSTOMERS
            .iter()
            .map(|name| (staged, *name))
            .chain([(other, "Solstice Retail")])
            .collect();

        let events = freshen_events(&streams, &customers, start, today);

        assert!(events.iter().all(|e| e.customer_id == other));
        let unstaged = freshen_events(&streams, &[(other, "Solstice Retail")], start, today);
        assert!(!unstaged.is_empty());
        assert_eq!(events, unstaged);
    }

    fn keys(keys: &[[&str; 2]]) -> Vec<Vec<String>> {
        keys.iter()
            .map(|key| key.iter().map(|p| p.to_string()).collect())
            .collect()
    }

    /// The LLM card's exact values, each equally often, and one no key names at a tenth.
    #[test]
    fn a_rated_metrics_events_take_the_values_its_card_names() {
        let dimensions = ["model".to_string(), "token_type".to_string()];
        let card = keys(&[
            ["claude-sonnet", "input"],
            ["claude-sonnet", "output"],
            ["gpt-4o", "input"],
            ["gpt-4o", "output"],
            ["gpt-4o-mini", "*"],
            ["*", "*"],
        ]);
        let derived = card_dimensions(&dimensions, &card);
        let [model, token_type] = derived.as_slice() else {
            panic!("a property per dimension");
        };
        assert_eq!(model.key, "model");
        assert_eq!(
            model.values,
            vec!["claude-sonnet", "gpt-4o", "gpt-4o-mini", "other"]
        );
        assert_eq!(model.weights, vec![9u32, 9, 9, 3]);
        assert_eq!(token_type.key, "token_type");
        assert_eq!(token_type.values, vec!["input", "output", "other"]);
        assert_eq!(token_type.weights, vec![9u32, 9, 2]);

        let named_other = card_dimensions(&dimensions[..1], &keys(&[["other", "*"]]));
        assert_eq!(named_other[0].values, vec!["other", "other-1"]);
        let only_any = card_dimensions(&dimensions[..1], &keys(&[["*", "*"]]));
        assert_eq!(only_any[0].values, vec!["other"]);
        assert_eq!(only_any[0].weights, vec![1u32]);
    }

    /// A rated metric's fresh events carry its card's dimensions in whole units; another keeps
    /// the default sample.
    #[test]
    fn fresh_events_of_a_rated_metric_carry_its_cards_dimensions() {
        let metric = |code: &str| BillableMetricMeta {
            id: BillableMetricId::new(),
            name: code.to_string(),
            code: code.to_string(),
            description: None,
            aggregation_type: BillingMetricAggregateEnum::Sum,
            aggregation_key: Some("tokens".to_string()),
            unit_conversion_factor: None,
            segmentation_matrix: None,
            filters: None,
            unit_singular: None,
            unit_plural: None,
            created_at: chrono::Utc::now().naive_utc(),
            archived_at: None,
        };
        let card = card_dimensions(
            &["model".to_string(), "token_type".to_string()],
            &keys(&[["gpt-4o", "input"], ["gpt-4o", "output"], ["*", "*"]]),
        );
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let start = today - ChronoDuration::days(FRESH_WINDOW_DAYS);
        let customers = [(customer(2), "Solstice Retail")];
        let rated = freshen_events(
            &[fresh_stream(&metric("llm_tokens"), Some(&card))],
            &customers,
            start,
            today,
        );
        assert!(!rated.is_empty());
        for event in &rated {
            let value = |key: &str| {
                event
                    .properties
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| panic!("{key} on every event"))
            };
            let tokens: u64 = value("tokens").parse().expect("whole tokens");
            assert!((RATED_MIN_UNITS..=RATED_MAX_UNITS).contains(&tokens));
            assert!(["gpt-4o", "other"].contains(&value("model").as_str()));
            assert!(["input", "output", "other"].contains(&value("token_type").as_str()));
        }
        let plain = freshen_events(
            &[fresh_stream(&metric("tracked_events"), None)],
            &customers,
            start,
            today,
        );
        assert!(!plain.is_empty());
        assert!(
            plain
                .iter()
                .all(|e| e.properties.iter().any(|(k, _)| k == "region"))
        );
    }

    #[test]
    fn payment_failure_targets_the_latest_invoice_due() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 9, d).unwrap();
        let id = |n| InvoiceId::from_const(uuid::Uuid::from_u128(n));
        let open = [
            (id(5), day(1), 1_000),
            (id(3), day(20), 1_000),
            (id(2), day(20), 1_000),
            (id(1), day(22), 0),
        ];
        assert_eq!(payment_failure_target(&open), Some(id(2)));
        assert_eq!(payment_failure_target(&open[3..]), None);
    }
}

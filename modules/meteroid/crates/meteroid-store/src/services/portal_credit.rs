//! The customer portal's read model of a customer's credit, limits and plan prices.
use crate::StoreResult;
use crate::domain::credit::{
    AutomaticTopUp, CreditGrant, CreditGrantSource, CreditGrantState, CreditWallet, GrantRollover,
    LedgerEntryKind, PortalCreditCapabilities,
};
use crate::domain::credit_outlook::CreditOutlook;
use crate::domain::entitlements::{EffectiveEntitlementValue, OverageBehavior};
use crate::domain::enums::SubscriptionStatusEnum;
use crate::domain::invoice_lines::LineItem;
use crate::domain::portal_credit::{
    PortalCreditGrant, PortalCreditOverview, PortalCreditSource, PortalCreditTotals,
    PortalExpiringCredit, PortalLimitGroupUsage, PortalLimitOrigin, PortalNextCredit,
    PortalPlanInputs, PortalPlanInputsBySubscription, PortalUpcomingInvoice, PortalUsageLimit,
    PortalUsageQuantity, PortalWalletOverview, component_in_effect,
};
use crate::domain::subscription_components::{SubscriptionComponent, SubscriptionFee};
use crate::domain::subscription_coupons::AppliedCouponDetailed;
use crate::domain::usage_alert_events::LIMIT_PCT;
use crate::domain::{
    BillableMetric, PaginationRequest, Subscription, SubscriptionDetails, UsagePeriod,
};
use crate::errors::StoreError;
use crate::hooks::{ForecastRead, LimitCrossing};
use crate::repositories::SubscriptionInterface;
use crate::repositories::credit::grants_in_draw_order;
use crate::services::ServicesEdge;
use crate::services::credit::forecast::{
    ForecastDocumentKind, ForecastUntil, GrantLabels, MAX_FORECAST_SUBSCRIPTIONS,
};
use crate::services::credit::value_application::quoted_subtotal;
use crate::services::credit::wallet_purchase::AUTOMATIC_TOP_UP_NAMESPACE;
use crate::services::entitlements::limits::entitlement_monitor_id;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, CreditGrantId, CustomerId, EntitlementEntityId, InvoiceId,
    SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::query::portal_credit::{
    list_active_slots, list_applied_coupons_for_subscriptions, list_customer_invoice_numbers,
    list_portal_grant_details, list_portal_ledger_kind_totals,
};
use diesel_models::subscription_adjustments::SubscriptionAdjustmentRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use error_stack::Report;
use futures::future::join_all;
use itertools::Itertools;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// Breakdowns and grouped usage list at most this many entries.
const TOP_GROUPS: usize = 5;
/// Below this much of the period, a linear projection says more about noise than about usage.
pub const MIN_PROJECTION_ELAPSED_SECONDS: i64 = 3600;

fn currency_key(currency: &str) -> String {
    currency.trim().to_uppercase()
}

fn overflow() -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(
        "a credit amount exceeds the supported range".to_owned(),
    ))
}

fn day_start(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(chrono::NaiveTime::MIN).and_utc()
}

/// Whether a subscription bills usage in the wallet's denomination: its currency, or a credit
/// unit one of its components is priced or funded in.
fn subscription_uses(
    subscription: &Subscription,
    components: &[SubscriptionComponent],
    wallet: &CreditWallet,
) -> bool {
    let key = currency_key(&wallet.currency);
    match &wallet.credit_unit {
        None => currency_key(&subscription.currency) == key,
        Some(_) => components.iter().any(|component| {
            let code = match &component.fee {
                SubscriptionFee::Usage { credit_unit, .. }
                | SubscriptionFee::Capacity { credit_unit, .. } => {
                    credit_unit.as_ref().map(|unit| unit.code.as_str())
                }
                SubscriptionFee::Credit { credit_unit, .. } => {
                    credit_unit.as_ref().map(|unit| unit.code.as_str())
                }
                _ => None,
            };
            code.is_some_and(|code| currency_key(code) == key)
        }),
    }
}

fn portal_source(grant: &CreditGrant, automatic_top_up: bool) -> PortalCreditSource {
    if grant.is_bonus() {
        return PortalCreditSource::Bonus;
    }
    match grant.source {
        CreditGrantSource::Purchase if automatic_top_up => PortalCreditSource::AutomaticTopUp,
        CreditGrantSource::Purchase => PortalCreditSource::Purchased,
        CreditGrantSource::Included => PortalCreditSource::Included,
        CreditGrantSource::Manual | CreditGrantSource::Coupon => PortalCreditSource::Added,
        CreditGrantSource::Rollover => PortalCreditSource::RolledOver,
        CreditGrantSource::CreditNote => PortalCreditSource::CreditNote,
        CreditGrantSource::Overpayment => PortalCreditSource::Overpayment,
        CreditGrantSource::NegativeInvoice
        | CreditGrantSource::InvoiceVoid
        | CreditGrantSource::UsageSettlementReturn => PortalCreditSource::Returned,
        CreditGrantSource::CustomerBalance => PortalCreditSource::PreviousBalance,
    }
}

/// `used` linearly extrapolated over the whole period, from what was used by `as_of`.
fn project_usage(
    used_minor: i64,
    period: (NaiveDate, NaiveDate),
    as_of: DateTime<Utc>,
) -> Option<i64> {
    let (start, end) = (day_start(period.0), day_start(period.1));
    let total = (end - start).num_seconds();
    if total <= 0 {
        return None;
    }
    if as_of >= end {
        return Some(used_minor);
    }
    let elapsed = (as_of - start).num_seconds();
    if elapsed < MIN_PROJECTION_ELAPSED_SECONDS {
        return None;
    }
    let projected = i128::from(used_minor) * i128::from(total) / i128::from(elapsed);
    i64::try_from(projected).ok()
}

/// The components of a subscription that bill metered usage: usage fees, and the overage of
/// capacity fees.
struct MeteredFees {
    components: HashSet<SubscriptionPriceComponentId>,
    add_ons: HashSet<SubscriptionAddOnId>,
}

fn is_metered(fee: &SubscriptionFee) -> bool {
    matches!(
        fee,
        SubscriptionFee::Usage { .. } | SubscriptionFee::Capacity { .. }
    )
}

impl MeteredFees {
    fn of(details: &SubscriptionDetails) -> Self {
        Self {
            components: details
                .price_components
                .iter()
                .filter(|component| is_metered(&component.fee))
                .map(|component| component.id)
                .collect(),
            add_ons: details
                .add_ons
                .iter()
                .filter(|add_on| is_metered(&add_on.fee))
                .map(|add_on| add_on.id)
                .collect(),
        }
    }

    /// A metered fee's usage line. Such lines alone carry the metric they price: the fixed part
    /// of a capacity fee, like any fixed fee, carries none even when billed in arrears.
    fn bills_usage(
        &self,
        metric_id: Option<BillableMetricId>,
        sub_component_id: Option<SubscriptionPriceComponentId>,
        sub_add_on_id: Option<SubscriptionAddOnId>,
    ) -> bool {
        metric_id.is_some()
            && (sub_component_id.is_some_and(|id| self.components.contains(&id))
                || sub_add_on_id.is_some_and(|id| self.add_ons.contains(&id)))
    }
}

/// The usage lines of a subscription's next invoice: its current period's usage, priced uncapped,
/// before any credit applies.
fn usage_lines(upcoming: &PortalUpcomingInvoice) -> Vec<&LineItem> {
    let metered = MeteredFees::of(&upcoming.details);
    upcoming
        .content
        .invoice_lines
        .iter()
        .filter(|line| {
            metered.bills_usage(line.metric_id, line.sub_component_id, line.sub_add_on_id)
        })
        .collect()
}

/// A usage line as the live projection prices what credit pays of it: in the figures it was
/// quoted in, plus the tax its rates levy on what promotional credit leaves, rounded up per tax.
fn usage_with_tax(line: &LineItem, tax_inclusive: bool) -> StoreResult<i64> {
    let quoted = quoted_subtotal(line, tax_inclusive);
    let taxable = quoted - line.promotional_credit.unwrap_or(0);
    if tax_inclusive || taxable <= 0 {
        return Ok(quoted);
    }
    let rates = if line.tax_details.is_empty() {
        vec![line.tax_rate]
    } else {
        line.tax_details.iter().map(|tax| tax.tax_rate).collect()
    };
    rates.into_iter().try_fold(quoted, |sum, rate| {
        (Decimal::from(taxable) * rate)
            .ceil()
            .to_i64()
            .and_then(|tax| sum.checked_add(tax))
            .ok_or_else(overflow)
    })
}

/// What `lines` price in the wallet's denomination: credits of its unit, or money in its currency
/// with the tax its credit pays, as the wallet's unbilled usage counts it.
fn usage_in_wallet(
    subscription: &Subscription,
    lines: &[&LineItem],
    wallet: &CreditWallet,
) -> StoreResult<i64> {
    let key = currency_key(&wallet.currency);
    match &wallet.credit_unit {
        Some(_) => lines
            .iter()
            .filter_map(|line| line.credit_unit.as_ref())
            .filter(|unit| currency_key(&unit.code) == key)
            .try_fold(0_i64, |sum, unit| sum.checked_add(unit.consumed))
            .ok_or_else(overflow),
        None if currency_key(&subscription.currency) == key => {
            lines.iter().try_fold(0_i64, |sum, line| {
                sum.checked_add(usage_with_tax(line, subscription.tax_inclusive)?)
                    .ok_or_else(overflow)
            })
        }
        None => Ok(0),
    }
}

/// The metric whose usage the wallet's lines price, when a single one does.
fn wallet_metric(lines: &[&LineItem], wallet: &CreditWallet) -> Option<BillableMetricId> {
    let key = currency_key(&wallet.currency);
    let metrics = lines
        .iter()
        .filter(|line| match (&wallet.credit_unit, &line.credit_unit) {
            (Some(_), Some(unit)) => currency_key(&unit.code) == key,
            (None, None) => true,
            _ => false,
        })
        .filter_map(|line| line.metric_id)
        .collect::<HashSet<_>>();
    match metrics.into_iter().collect::<Vec<_>>()[..] {
        [metric] => Some(metric),
        _ => None,
    }
}

struct ActiveSubscriptions {
    subscriptions: Vec<Subscription>,
    components: HashMap<SubscriptionId, Vec<SubscriptionComponent>>,
    cancellations: HashMap<SubscriptionId, NaiveDate>,
}

impl ActiveSubscriptions {
    fn components_of(&self, id: SubscriptionId) -> &[SubscriptionComponent] {
        self.components.get(&id).map_or(&[], Vec::as_slice)
    }

    fn drawing_on<'a>(
        &'a self,
        wallet: &'a CreditWallet,
    ) -> impl Iterator<Item = &'a Subscription> {
        self.subscriptions.iter().filter(move |subscription| {
            subscription_uses(subscription, self.components_of(subscription.id), wallet)
        })
    }

    /// No further period starts: paused, or cancelled at or before the end of this one.
    fn renews(&self, subscription: &Subscription) -> bool {
        subscription.status != SubscriptionStatusEnum::Paused
            && match (
                self.cancellations.get(&subscription.id),
                subscription.current_period_end,
            ) {
                (Some(cancels), Some(end)) => *cancels > end,
                (Some(_), None) => false,
                (None, _) => true,
            }
    }
}

/// The credit pack line of a subscription's next invoice funding the wallet: the credit it funds
/// once paid, at the price the invoice charges for it (seats, coupons and tax applied). Only the
/// next invoice is computed, so a pack on another cadence that it does not bill is not shown.
fn next_credit_of(
    invoice: &PortalUpcomingInvoice,
    wallet: &CreditWallet,
) -> Option<PortalNextCredit> {
    let subscription = &invoice.details.subscription;
    let key = currency_key(&wallet.currency);
    let lines = invoice
        .content
        .invoice_lines
        .iter()
        .filter(|line| {
            line.credit_grant_amount.is_some()
                && match (&line.credit_grant_unit, &wallet.credit_unit) {
                    (Some(unit), Some(_)) => currency_key(&unit.code) == key,
                    (None, None) => currency_key(&subscription.currency) == key,
                    _ => false,
                }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let cadence = lines
        .iter()
        .filter_map(|line| line.sub_component_id)
        .find_map(|id| {
            invoice
                .details
                .price_components
                .iter()
                .find(|component| component.id == id)
                .map(|component| component.period)
        })
        .unwrap_or_else(|| subscription.period.as_subscription_billing_period());
    let price_minor = lines
        .iter()
        .map(|line| {
            if subscription.tax_inclusive {
                line.amount_total
            } else {
                line.taxable_amount
            }
        })
        .sum();
    Some(PortalNextCredit {
        amount_minor: lines
            .iter()
            .filter_map(|line| line.credit_grant_amount)
            .sum(),
        date: subscription.current_period_end?,
        price_minor,
        price_currency: subscription.currency.clone(),
        cadence,
        price_excludes_tax: !subscription.tax_inclusive
            && lines.iter().any(|line| line.tax_amount > 0),
    })
}

/// When usage first reached the limit in its current window: the earliest crossing of the limit
/// one of its monitors recorded for that window.
fn reached_at(
    crossings: &[LimitCrossing],
    monitor_ids: &[Uuid],
    window_start: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    crossings
        .iter()
        .filter(|row| {
            monitor_ids.contains(&row.monitor_id) && row.window_start == Some(window_start)
        })
        .map(|row| row.fired_at)
        .min()
}

fn hard_ceiling(limit: Decimal, grace_pct: Option<u32>) -> Decimal {
    limit + limit * Decimal::from(grace_pct.unwrap_or(0)) / Decimal::ONE_HUNDRED
}

/// A group at or over `warning_pct` of the limit; without a warning threshold, at the limit.
fn at_warning(groups: &[PortalLimitGroupUsage], limit: Decimal, warning_pct: Option<u32>) -> bool {
    let pct = warning_pct.unwrap_or(LIMIT_PCT).min(LIMIT_PCT);
    let threshold = limit * Decimal::from(pct) / Decimal::ONE_HUNDRED;
    groups.iter().any(|group| group.used >= threshold)
}

fn top_groups(groups: impl Iterator<Item = PortalLimitGroupUsage>) -> Vec<PortalLimitGroupUsage> {
    let mut groups = groups.collect::<Vec<_>>();
    groups.sort_by(|a, b| b.used.cmp(&a.used).then_with(|| a.group.cmp(&b.group)));
    groups.truncate(TOP_GROUPS);
    groups
}

impl ServicesEdge {
    async fn portal_active_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<ActiveSubscriptions> {
        let today = Utc::now().date_naive();
        let mut subscriptions = self
            .store
            .list_subscriptions(
                tenant_id,
                Some(customer_id),
                None,
                Some(vec![
                    SubscriptionStatusEnum::Active,
                    SubscriptionStatusEnum::TrialActive,
                    SubscriptionStatusEnum::Paused,
                ]),
                None,
                vec![],
                None,
                PaginationRequest {
                    page: 0,
                    per_page: Some(u32::try_from(MAX_FORECAST_SUBSCRIPTIONS).unwrap_or(u32::MAX)),
                },
            )
            .await?
            .items;
        subscriptions
            .sort_by_key(|subscription| (subscription.current_period_end, subscription.created_at));
        let ids = subscriptions
            .iter()
            .map(|subscription| subscription.id)
            .collect::<Vec<_>>();
        let cancellations = self
            .store
            .list_pending_cancellations(tenant_id, &ids)
            .await?;
        let mut conn = self.store.get_conn().await?;
        let components = SubscriptionComponentRow::list_subscription_components_by_subscriptions(
            &mut conn, &tenant_id, &ids,
        )
        .await?
        .into_iter()
        .map(|(id, rows)| {
            rows.into_iter()
                .map(TryInto::try_into)
                .collect::<StoreResult<Vec<SubscriptionComponent>>>()
                .map(|components| {
                    let components = components
                        .into_iter()
                        .filter(|component| component_in_effect(component, today))
                        .collect();
                    (id, components)
                })
        })
        .collect::<StoreResult<HashMap<_, _>>>()?;
        Ok(ActiveSubscriptions {
            subscriptions,
            components,
            cancellations,
        })
    }

    /// Invoice numbers of the customer's own invoices among `invoice_ids`.
    pub async fn portal_invoice_numbers(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        invoice_ids: &[InvoiceId],
    ) -> StoreResult<HashMap<InvoiceId, String>> {
        let mut conn = self.store.get_conn().await?;
        Ok(
            list_customer_invoice_numbers(&mut conn, tenant_id, customer_id, invoice_ids)
                .await?
                .into_iter()
                .collect(),
        )
    }

    /// The customer's grants an automatic top-up bought.
    pub async fn portal_automatic_top_up_grants(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<HashSet<CreditGrantId>> {
        let mut conn = self.store.get_conn().await?;
        Ok(list_portal_grant_details(
            &mut conn,
            tenant_id,
            customer_id,
            AUTOMATIC_TOP_UP_NAMESPACE,
        )
        .await?
        .into_iter()
        .filter(|row| row.automatic_top_up)
        .map(|row| row.grant_id)
        .collect())
    }

    /// The next invoice of each subscription, forecast together so shared credit is counted once
    /// in billing order, the subscriptions whose next invoice cannot be computed right now
    /// (metering unavailable, or past the forecast's bound), and the credit outlooks read from
    /// the same forecast.
    async fn portal_upcoming_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscriptions: &[&Subscription],
    ) -> StoreResult<(
        Vec<PortalUpcomingInvoice>,
        Vec<SubscriptionId>,
        Vec<CreditOutlook>,
    )> {
        let now = Utc::now();
        let mut conn = self.store.get_conn().await?;
        let forecast = self
            .services
            .forecast_customer_credit(
                &mut conn,
                tenant_id,
                customer_id,
                ForecastUntil::All,
                None,
                now,
            )
            .await?;
        drop(conn);
        let outlooks = self
            .store
            .hooks
            .credit
            .credit_outlooks(
                self,
                tenant_id,
                customer_id,
                now,
                ForecastRead::new(
                    &forecast.documents,
                    forecast.truncated || !forecast.unavailable.is_empty(),
                ),
            )
            .await?;
        let mut conn = self.store.get_conn().await?;
        let labels =
            GrantLabels::load_tx(&mut conn, tenant_id, &forecast.books, &forecast.documents)
                .await?;
        let mut upcoming = Vec::with_capacity(subscriptions.len());
        let mut unavailable = Vec::new();
        for subscription in subscriptions {
            let index = forecast.documents.iter().position(|document| {
                document.kind == ForecastDocumentKind::Upcoming(subscription.id)
            });
            match index.and_then(|index| {
                let document = &forecast.documents[index];
                document
                    .details
                    .as_deref()
                    .map(|details| (index, details, &document.content))
            }) {
                Some((index, details, content)) => upcoming.push(PortalUpcomingInvoice {
                    details: details.clone(),
                    invoice_date: forecast.documents[index].invoice_date,
                    content: content.clone(),
                    credit_estimate_shared: forecast.estimate_shared(&labels, index),
                    billed_first: forecast.billed_first(&labels, index),
                }),
                None => {
                    if !forecast.truncated && !forecast.unavailable.contains(&subscription.id) {
                        log::warn!(
                            "upcoming invoice of subscription {} was not forecast",
                            subscription.id
                        );
                    }
                    unavailable.push(subscription.id);
                }
            }
        }
        Ok((upcoming, unavailable, outlooks))
    }

    pub async fn portal_credit_overview(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<PortalCreditOverview> {
        let now = Utc::now();
        let capabilities: PortalCreditCapabilities = self
            .portal_credit_capabilities(tenant_id, customer_id)
            .await?;
        let wallets: Vec<(CreditWallet, Option<AutomaticTopUp>)> = self
            .customer_credit_wallets_with_auto_top_up(tenant_id, customer_id)
            .await?;
        let active = self
            .portal_active_subscriptions(tenant_id, customer_id)
            .await?;

        let (live_wallets, details, totals) = {
            let mut conn = self.store.get_conn().await?;
            let live_wallets = self
                .store
                .hooks
                .credit
                .live_wallets(&mut conn, tenant_id, customer_id, &wallets)
                .await?;
            let details = list_portal_grant_details(
                &mut conn,
                tenant_id,
                customer_id,
                AUTOMATIC_TOP_UP_NAMESPACE,
            )
            .await?
            .into_iter()
            .map(|row| (row.grant_id, row))
            .collect::<HashMap<_, _>>();
            let totals =
                list_portal_ledger_kind_totals(&mut conn, tenant_id, customer_id, now).await?;
            (live_wallets, details, totals)
        };
        let health = &live_wallets.health;
        let per_grant = &live_wallets.remaining_by_grant;

        let invoiced = active
            .subscriptions
            .iter()
            .filter(|subscription| subscription.current_period_end.is_some())
            .collect::<Vec<_>>();
        let (upcoming, upcoming_unavailable, outlooks) = self
            .portal_upcoming_invoices(tenant_id, customer_id, &invoiced)
            .await?;

        // A plan is credit-funded when usage stops once the credit it draws on runs out.
        let credit_funded = active.subscriptions.iter().any(|subscription| {
            let drawn = wallets
                .iter()
                .filter(|(wallet, _)| {
                    subscription_uses(subscription, active.components_of(subscription.id), wallet)
                })
                .collect::<Vec<_>>();
            !drawn.is_empty()
                && drawn.iter().all(|(wallet, _)| {
                    let plan = wallet
                        .plan_rules
                        .iter()
                        .find(|plan| plan.subscription_id == subscription.id)
                        .map(|plan| &plan.rule);
                    wallet.usage_at_zero.allowance(plan, &wallet.currency) == Some(0)
                })
        });

        let mut quantity_reads = Vec::with_capacity(wallets.len());
        let mut overviews = Vec::with_capacity(wallets.len());
        for ((wallet, auto_top_up), health) in wallets.iter().zip(health) {
            let live = health.live.as_ref();
            let grants = grants_in_draw_order(&wallet.grants)
                .into_iter()
                .filter(|grant| {
                    (grant.state == CreditGrantState::Active && grant.remaining_minor > 0)
                        || grant.state == CreditGrantState::Pending
                })
                .map(|grant| {
                    let detail = details.get(&grant.id);
                    let projected = live.and_then(|_| {
                        per_grant.get(&grant.id).copied().or_else(|| {
                            // Rolled credit is projected under the expired grant it stands in for.
                            (grant.source == CreditGrantSource::Rollover)
                                .then(|| {
                                    per_grant
                                        .get(&CreditGrantId::from(grant.source_id))
                                        .copied()
                                })
                                .flatten()
                        })
                    });
                    let rollover = match detail.and_then(|detail| detail.rollover.as_ref()) {
                        Some(json) if grant.expires_at.is_some() => {
                            Some(GrantRollover::from_json(json)?)
                        }
                        _ => None,
                    };
                    Ok(PortalCreditGrant {
                        source: portal_source(
                            grant,
                            detail.is_some_and(|detail| detail.automatic_top_up),
                        ),
                        remaining_after_usage_minor: (grant.state == CreditGrantState::Active)
                            .then_some(projected)
                            .flatten(),
                        rollover_max_minor: rollover.map(|rollover| rollover.max_minor),
                        invoice_number: detail.and_then(|detail| detail.invoice_number.clone()),
                        grant: grant.clone(),
                    })
                })
                .collect::<StoreResult<Vec<_>>>()?;

            // Usage missing a subscription's invoice would read low: none is shown instead.
            let usage_known = active
                .drawing_on(wallet)
                .all(|subscription| !upcoming_unavailable.contains(&subscription.id));
            let contributing = active
                .drawing_on(wallet)
                .filter(|_| usage_known)
                .filter_map(|subscription| {
                    let invoice = upcoming
                        .iter()
                        .find(|invoice| invoice.details.subscription.id == subscription.id)?;
                    let end = subscription.current_period_end?;
                    Some((
                        subscription,
                        usage_lines(invoice),
                        invoice,
                        (subscription.current_period_start, end),
                    ))
                })
                .collect::<Vec<_>>();
            let usages = contributing
                .iter()
                .map(|(subscription, lines, _, period)| {
                    usage_in_wallet(subscription, lines, wallet).map(|used| (used, *period))
                })
                .collect::<StoreResult<Vec<_>>>()?;
            let period_usage_minor = (!usages.is_empty())
                .then(|| {
                    usages
                        .iter()
                        .try_fold(0_i64, |sum, (used, _)| sum.checked_add(*used))
                        .ok_or_else(overflow)
                })
                .transpose()?;
            let projected_usage_minor = (!usages.is_empty())
                .then(|| {
                    usages
                        .iter()
                        .map(|(used, period)| project_usage(*used, *period, now))
                        .try_fold(0_i64, |sum, projected| sum.checked_add(projected?))
                })
                .flatten();
            let period = match contributing.first() {
                Some((_, _, _, first))
                    if contributing.iter().all(|(_, _, _, period)| period == first) =>
                {
                    Some(*first)
                }
                _ => None,
            };
            // Each subscription counts the usage its usage filter attributes to it.
            let quantity_metric = match period {
                Some(period) if !contributing.is_empty() => {
                    let lines = contributing
                        .iter()
                        .flat_map(|(_, lines, _, _)| lines.iter().copied())
                        .collect::<Vec<_>>();
                    wallet_metric(&lines, wallet).and_then(|metric_id| {
                        let metrics = contributing
                            .iter()
                            .filter_map(|(_, _, invoice, _)| {
                                invoice
                                    .details
                                    .metrics
                                    .iter()
                                    .find(|metric| metric.id == metric_id)
                                    .cloned()
                            })
                            .unique_by(|metric| metric.usage_scope.clone())
                            .collect::<Vec<_>>();
                        (!metrics.is_empty()).then_some((metrics, period.0))
                    })
                }
                _ => None,
            };
            quantity_reads.push(quantity_metric);

            let kind_total = |kind: LedgerEntryKind| {
                totals
                    .iter()
                    .filter(|row| row.account_id == wallet.id && row.kind == kind.as_db_str())
                    .map(|row| row.total)
                    .sum::<i64>()
            };
            // Value past its expiry the sweep has not recorded yet counts as lapsed, except what
            // will roll over into a new grant.
            let pending_rollover = wallet
                .grants
                .iter()
                .filter(|grant| grant.state == CreditGrantState::Expired)
                .filter(|grant| {
                    !wallet.grants.iter().any(|rolled| {
                        rolled.source == CreditGrantSource::Rollover
                            && rolled.source_id == grant.id.as_uuid()
                    })
                })
                // Once swept, a grant that carried anything has its rolled grant; one that did not
                // had nothing to carry, so `carried` of what lapsed is zero for it.
                .filter_map(|grant| {
                    let json = details.get(&grant.id)?.rollover.as_ref()?;
                    let rollover = GrantRollover::from_json(json).ok()?;
                    Some(rollover.carried(grant.expired_minor?))
                })
                .sum::<i64>();
            let expired = wallet
                .grants
                .iter()
                .filter_map(|grant| grant.expired_minor)
                .sum::<i64>()
                - pending_rollover;
            let added = kind_total(LedgerEntryKind::Fund)
                + kind_total(LedgerEntryKind::Rollover)
                + kind_total(LedgerEntryKind::Reversal)
                + kind_total(LedgerEntryKind::Reinstatement)
                + pending_rollover;
            let balance = live.map_or(wallet.available_minor, |live| live.live_balance_minor);
            let totals = PortalCreditTotals {
                since: wallet
                    .grants
                    .iter()
                    .filter(|grant| grant.effective_at <= now)
                    .map(|grant| grant.effective_at)
                    .min(),
                added_minor: added,
                used_minor: added
                    .checked_sub(expired)
                    .and_then(|left| left.checked_sub(balance + pending_rollover))
                    .ok_or_else(overflow)?,
                expired_minor: expired,
            };

            let next_credit = upcoming
                .iter()
                .filter(|invoice| active.renews(&invoice.details.subscription))
                .find_map(|invoice| next_credit_of(invoice, wallet));

            let next_expiry = grants
                .iter()
                .filter(|grant| grant.grant.state == CreditGrantState::Active)
                .filter_map(|grant| grant.grant.lapses_at())
                .min()
                .map(|at| {
                    let expiring = grants
                        .iter()
                        .filter(|grant| {
                            grant.grant.state == CreditGrantState::Active
                                && grant.grant.lapses_at() == Some(at)
                        })
                        .map(|grant| {
                            let left = grant
                                .remaining_after_usage_minor
                                .unwrap_or(grant.grant.remaining_minor)
                                .max(0);
                            let carried = grant
                                .rollover_max_minor
                                .map(|max| max.map_or(left, |max| left.min(max.max(0))));
                            (left, carried)
                        })
                        .collect::<Vec<_>>();
                    PortalExpiringCredit {
                        expires_at: at,
                        remaining_minor: expiring.iter().map(|(left, _)| left).sum(),
                        rollover_minor: expiring
                            .iter()
                            .any(|(_, carried)| carried.is_some())
                            .then(|| expiring.iter().filter_map(|(_, carried)| *carried).sum()),
                    }
                });

            let usage_paused_at = live_wallets.usage_paused_at.get(&wallet.id).copied();

            overviews.push(PortalWalletOverview {
                wallet: wallet.clone(),
                auto_top_up: auto_top_up.clone(),
                health: health.clone(),
                grants,
                period,
                projected_usage_minor,
                usage_quantity: None,
                totals,
                next_credit,
                next_expiry,
                usage_paused_at,
                period_usage_minor,
            });
        }

        let quantities = join_all(quantity_reads.into_iter().map(|read| async move {
            let (metrics, start) = read?;
            let period = UsagePeriod {
                start: day_start(start).naive_utc(),
                end: now.naive_utc(),
            };
            let mut quantity = Decimal::ZERO;
            for metric in &metrics {
                let read = self
                    .services
                    .usage_client
                    .fetch_total_usage(&tenant_id, &customer_id, metric, period.clone())
                    .await
                    .inspect_err(|error| {
                        log::warn!(
                            "portal usage quantity unavailable for customer {customer_id}: {error:?}"
                        );
                    })
                    .ok()?;
                quantity = quantity.checked_add(read)?;
            }
            let metric = metrics.into_iter().next()?;
            Some(PortalUsageQuantity {
                quantity,
                metric_name: metric.name,
                unit_singular: metric.unit_singular,
                unit_plural: metric.unit_plural,
            })
        }))
        .await;
        for (overview, quantity) in overviews.iter_mut().zip(quantities) {
            overview.usage_quantity = quantity;
        }

        Ok(PortalCreditOverview {
            wallets: overviews,
            capabilities,
            credit_funded,
            upcoming,
            upcoming_unavailable,
            outlooks,
        })
    }

    async fn portal_applicable_coupons(
        &self,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, Vec<AppliedCouponDetailed>>> {
        let mut conn = self.store.get_conn().await?;
        let mut coupons = list_applied_coupons_for_subscriptions(&mut conn, subscription_ids)
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<StoreResult<Vec<AppliedCouponDetailed>>>()?;
        coupons.retain(AppliedCouponDetailed::is_invoice_applicable);
        coupons.sort_by_key(|coupon| coupon.applied_coupon.created_at);
        let mut by_subscription: HashMap<SubscriptionId, Vec<AppliedCouponDetailed>> =
            HashMap::new();
        for coupon in coupons {
            by_subscription
                .entry(coupon.applied_coupon.subscription_id)
                .or_default()
                .push(coupon);
        }
        Ok(by_subscription)
    }

    /// The entitlement limits that refuse usage, from the plan or set for the account, as they
    /// stand in their current window. A metering failure leaves them out, since their usage is
    /// read live.
    pub async fn portal_usage_limits(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<PortalUsageLimit>> {
        let now = Utc::now();
        let subscription_ids = self
            .portal_active_subscriptions(tenant_id, customer_id)
            .await?
            .subscriptions
            .iter()
            .map(|subscription| subscription.id)
            .collect::<Vec<_>>();
        let entitlements = self
            .get_effective_entitlements(customer_id, tenant_id)
            .await
            .inspect_err(|error| {
                log::warn!("portal plan limits unavailable for customer {customer_id}: {error:?}");
            })
            .unwrap_or_default();

        let plan_limits = entitlements.into_iter().filter_map(|entitlement| {
            let EffectiveEntitlementValue::Metered {
                metric_id,
                limit: Some(limit),
                overage_behavior: OverageBehavior::Block { grace_period_pct },
                enabled,
                group_by,
                usage,
                thresholds,
                limit_reached,
                ..
            } = entitlement.value
            else {
                return None;
            };
            // A reached limit is still the customer's: it is what refuses their usage.
            if !enabled && !limit_reached {
                return None;
            }
            // Overrides of the customer or one of its subscriptions are set for this account.
            let origin = match entitlement.origin.entity {
                EntitlementEntityId::Customer(_) | EntitlementEntityId::Subscription(_) => {
                    PortalLimitOrigin::Account
                }
                _ => PortalLimitOrigin::Plan,
            };
            Some((
                entitlement.feature,
                origin,
                metric_id,
                limit,
                grace_period_pct,
                group_by,
                usage,
                thresholds.first().copied(),
            ))
        });
        let metric_ids = plan_limits
            .clone()
            .map(|(_, _, metric_id, ..)| metric_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        // Plan limits are the anchor subscription's, measured as its usage filter attributes them.
        let metrics = {
            let mut conn = self.store.get_conn().await?;
            let anchor = self
                .services
                .load_billing_context(&mut conn, customer_id, tenant_id)
                .await?
                .subscription_id;
            self.services
                .load_metrics(&mut conn, &metric_ids, &tenant_id, anchor)
                .await?
        };

        let mut monitors: Vec<Vec<Uuid>> = Vec::new();
        let mut limits = join_all(plan_limits.map(
            |(feature, origin, metric_id, limit, grace_pct, group_by, usage, warning_pct)| {
                let metric = metrics.get(&metric_id);
                async move {
                    let period = usage.period_start.map(|start| UsagePeriod {
                        start: start.naive_utc(),
                        end: now.naive_utc(),
                    });
                    let consumed = usage.consumed.unwrap_or_default();
                    let (groups, breakdown) = match (metric, &group_by, period) {
                        (Some(metric), Some(property), Some(period)) => (
                            self.portal_grouped_usage(
                                &tenant_id,
                                &customer_id,
                                metric,
                                property,
                                period,
                            )
                            .await,
                            Vec::new(),
                        ),
                        (metric, None, period) => (
                            vec![PortalLimitGroupUsage {
                                group: BTreeMap::new(),
                                used: consumed,
                            }],
                            match (metric, period) {
                                (Some(metric), Some(period))
                                    if metric.segmentation_matrix.is_some() =>
                                {
                                    self.portal_breakdown(&tenant_id, &customer_id, metric, period)
                                        .await
                                }
                                _ => Vec::new(),
                            },
                        ),
                        _ => (Vec::new(), Vec::new()),
                    };
                    // The consumed usage is the largest group's when grouped, as the engine holds it.
                    let reached = consumed >= hard_ceiling(limit, grace_pct);
                    let warning = reached
                        || at_warning(
                            &[PortalLimitGroupUsage {
                                group: BTreeMap::new(),
                                used: consumed,
                            }],
                            limit,
                            warning_pct,
                        );
                    PortalUsageLimit {
                        feature_id: feature.id,
                        name: feature.name,
                        origin,
                        group_by,
                        limit,
                        hard: true,
                        window_start: usage.period_start,
                        resets_at: usage.reset_at,
                        groups,
                        breakdown,
                        reached,
                        reached_at: None,
                        warning,
                        unit_singular: metric.and_then(|metric| metric.unit_singular.clone()),
                        unit_plural: metric.and_then(|metric| metric.unit_plural.clone()),
                    }
                }
            },
        ))
        .await;
        for limit in &limits {
            monitors.push(
                subscription_ids
                    .iter()
                    .map(|id| entitlement_monitor_id(limit.feature_id, *id))
                    .collect(),
            );
        }

        let crossings = {
            let mut conn = self.store.get_conn().await?;
            let ids = monitors.iter().flatten().copied().collect::<Vec<_>>();
            self.store
                .hooks
                .credit
                .held_limit_crossings(&mut conn, tenant_id, &ids)
                .await?
        };
        for (limit, monitor_ids) in limits.iter_mut().zip(&monitors) {
            limit.reached_at = limit
                .window_start
                .and_then(|start| reached_at(&crossings, monitor_ids, start));
        }
        Ok(limits)
    }

    pub async fn portal_billable_metrics(
        &self,
        tenant_id: TenantId,
        metric_ids: &[BillableMetricId],
    ) -> StoreResult<HashMap<BillableMetricId, BillableMetric>> {
        if metric_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut conn = self.store.get_conn().await?;
        BillableMetricRow::get_by_ids(&mut conn, metric_ids, &tenant_id)
            .await?
            .into_iter()
            .map(|row| {
                let metric: BillableMetric = row.try_into()?;
                Ok((metric.id, metric))
            })
            .collect()
    }

    async fn portal_grouped_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        property: &str,
        period: UsagePeriod,
    ) -> Vec<PortalLimitGroupUsage> {
        let Ok(usage) = self
            .services
            .usage_client
            .fetch_usage_grouped_by(tenant_id, customer_id, metric, property, period)
            .await
            .inspect_err(|error| {
                log::warn!(
                    "portal grouped usage unavailable for customer {customer_id}: {error:?}"
                );
            })
        else {
            return Vec::new();
        };
        let mut by_group: BTreeMap<String, Decimal> = BTreeMap::new();
        for point in usage.data {
            if let Some(value) = point.dimensions.get(property) {
                *by_group.entry(value.clone()).or_default() += point.value;
            }
        }
        top_groups(
            by_group
                .into_iter()
                .map(|(value, used)| PortalLimitGroupUsage {
                    group: BTreeMap::from([(property.to_owned(), value)]),
                    used,
                }),
        )
    }

    async fn portal_breakdown(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> Vec<PortalLimitGroupUsage> {
        let Ok(usage) = self
            .services
            .usage_client
            .fetch_usage(tenant_id, customer_id, metric, period)
            .await
            .inspect_err(|error| {
                log::warn!(
                    "portal usage breakdown unavailable for customer {customer_id}: {error:?}"
                );
            })
        else {
            return Vec::new();
        };
        top_groups(
            usage
                .data
                .into_iter()
                .filter(|point| !point.dimensions.is_empty())
                .map(|point| PortalLimitGroupUsage {
                    group: point.dimensions.into_iter().collect(),
                    used: point.value,
                }),
        )
    }

    /// What each subscription's recurring price is computed from: its components in effect, seats
    /// in use, minimum, coupons and usage metrics.
    pub async fn portal_plan_inputs(
        &self,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<PortalPlanInputsBySubscription> {
        let now = Utc::now();
        let today = now.date_naive();
        let (components, minimums, seats) = {
            let mut conn = self.store.get_conn().await?;
            let components =
                SubscriptionComponentRow::list_subscription_components_by_subscriptions(
                    &mut conn,
                    &tenant_id,
                    subscription_ids,
                )
                .await?
                .into_iter()
                .map(|(id, rows)| {
                    rows.into_iter()
                        .map(TryInto::try_into)
                        .collect::<StoreResult<Vec<SubscriptionComponent>>>()
                        .map(|components| {
                            let components = components
                                .into_iter()
                                .filter(|component| component_in_effect(component, today))
                                .collect::<Vec<_>>();
                            (id, components)
                        })
                })
                .collect::<StoreResult<HashMap<_, _>>>()?;
            let minimums =
                SubscriptionAdjustmentRow::list_by_subscription_ids(&mut conn, subscription_ids)
                    .await?;
            let pairs = components
                .iter()
                .flat_map(|(id, components)| {
                    components
                        .iter()
                        .filter_map(move |component| match &component.fee {
                            SubscriptionFee::Slot { unit, .. } => Some((*id, unit.clone())),
                            _ => None,
                        })
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let seats = list_active_slots(&mut conn, tenant_id, &pairs, now.naive_utc()).await?;
            (components, minimums, seats)
        };
        let mut coupons = self.portal_applicable_coupons(subscription_ids).await?;
        let metric_ids = components
            .values()
            .flatten()
            .filter_map(|component| match &component.fee {
                SubscriptionFee::Usage { metric_id, .. } => Some(*metric_id),
                _ => None,
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let metrics = self.portal_billable_metrics(tenant_id, &metric_ids).await?;

        let mut inputs: PortalPlanInputsBySubscription = HashMap::new();
        for (id, components) in components {
            let slots = components
                .iter()
                .filter_map(|component| match &component.fee {
                    SubscriptionFee::Slot { unit, .. } => Some(unit.clone()),
                    _ => None,
                })
                .map(|unit| {
                    // A seat unit without transactions has none in use, as billing reads it.
                    let count = seats
                        .iter()
                        .find(|row| row.subscription_id == id && row.unit == unit)
                        .map_or(0, |row| u32::try_from(row.active_slots).unwrap_or(0));
                    (unit, count)
                })
                .collect();
            let subscription_metrics = components
                .iter()
                .filter_map(|component| match &component.fee {
                    SubscriptionFee::Usage { metric_id, .. } => metrics
                        .get(metric_id)
                        .map(|metric| (*metric_id, metric.clone())),
                    _ => None,
                })
                .collect();
            inputs.insert(
                id,
                PortalPlanInputs {
                    components,
                    slots,
                    minimum_minor: None,
                    coupons: coupons.remove(&id).unwrap_or_default(),
                    metrics: subscription_metrics,
                },
            );
        }
        for minimum in minimums {
            inputs
                .entry(minimum.subscription_id)
                .or_default()
                .minimum_minor = Some(minimum.amount_subunit);
        }
        Ok(inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::TaxDetail;
    use crate::domain::credit::GrantClassification;
    use crate::domain::invoice_lines::LineItemKind;
    use crate::repositories::credit::customer_credit::BONUS_CREDIT;
    use common_domain::ids::WalletId;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    #[test]
    fn usage_is_projected_linearly_over_the_period() {
        let as_of = day_start(date(11));
        assert_eq!(project_usage(150, (date(1), date(21)), as_of), Some(300));
    }

    #[test]
    fn no_projection_right_after_the_period_starts_and_none_past_its_end() {
        let start = day_start(date(1));
        assert_eq!(project_usage(150, (date(1), date(21)), start), None);
        assert_eq!(
            project_usage(150, (date(1), date(21)), day_start(date(25))),
            Some(150)
        );
    }

    #[test]
    fn a_limit_warns_from_its_threshold_or_at_the_limit() {
        let groups = vec![PortalLimitGroupUsage {
            group: BTreeMap::new(),
            used: Decimal::from(80),
        }];
        assert!(at_warning(&groups, Decimal::from(100), Some(80)));
        assert!(!at_warning(&groups, Decimal::from(100), Some(90)));
        assert!(!at_warning(&groups, Decimal::from(100), None));
    }

    #[test]
    fn only_metered_lines_count_as_usage_never_a_capacity_base_fee() {
        let capacity = SubscriptionPriceComponentId::new();
        let rate = SubscriptionPriceComponentId::new();
        let metric = BillableMetricId::new();
        let capacity_fee = SubscriptionFee::Capacity {
            rate: Decimal::from(100),
            included: 1000,
            overage_rate: Decimal::ONE,
            metric_id: metric,
            credit_unit: None,
        };
        assert!(is_metered(&capacity_fee));
        assert!(!is_metered(&SubscriptionFee::Rate {
            rate: Decimal::from(100)
        }));
        let metered = MeteredFees {
            components: HashSet::from([capacity]),
            add_ons: HashSet::new(),
        };
        // The capacity base fee carries no metric, in advance or in arrears.
        assert!(!metered.bills_usage(None, Some(capacity), None));
        assert!(metered.bills_usage(Some(metric), Some(capacity), None));
        assert!(!metered.bills_usage(None, Some(rate), None));
        assert!(!metered.bills_usage(Some(metric), Some(rate), None));
    }

    #[test]
    fn a_limit_is_reached_at_its_first_crossing_in_the_current_window() {
        let monitor = Uuid::new_v4();
        let other = Uuid::new_v4();
        let window = day_start(date(1));
        let crossing = |monitor_id: Uuid, start: DateTime<Utc>, fired: u32| LimitCrossing {
            monitor_id,
            window_start: Some(start),
            fired_at: day_start(date(fired)),
        };
        let crossings = vec![
            crossing(monitor, day_start(date(1)) - chrono::Duration::days(30), 2),
            crossing(monitor, window, 9),
            crossing(monitor, window, 6),
            crossing(other, window, 3),
        ];
        assert_eq!(
            reached_at(&crossings, &[monitor], window),
            Some(day_start(date(6)))
        );
        assert_eq!(reached_at(&crossings, &[Uuid::new_v4()], window), None);
    }

    #[test]
    fn a_hard_limit_refuses_past_its_grace() {
        assert_eq!(
            hard_ceiling(Decimal::from(100), Some(10)),
            Decimal::from(110)
        );
        assert_eq!(hard_ceiling(Decimal::from(100), None), Decimal::from(100));
    }

    fn usage_line(amount: i64, tax_rate: Decimal) -> LineItem {
        let day = date(1);
        LineItem {
            local_id: "usage".to_owned(),
            name: "Tokens".to_owned(),
            charge_type: None,
            amount_subtotal: amount,
            tax_rate,
            taxable_amount: amount,
            tax_amount: 0,
            amount_total: amount,
            tax_details: vec![TaxDetail {
                tax_rate,
                tax_name: "Sales tax".to_owned(),
                tax_amount: 0,
            }],
            quantity: None,
            unit_price: None,
            start_date: day,
            end_date: date(30),
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: Some(BillableMetricId::new()),
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

    /// $150.00 of usage at 8%: the period usage is $162.00, the tax included as paid credit pays
    /// it.
    #[test]
    fn period_usage_includes_the_tax_paid_credit_pays() {
        let line = usage_line(15_000, Decimal::new(8, 2));
        assert_eq!(usage_with_tax(&line, false).unwrap(), 16_200);
    }

    #[test]
    fn promotional_credit_comes_off_before_tax_and_a_tax_inclusive_line_counts_as_quoted() {
        let mut promoted = usage_line(15_000, Decimal::new(8, 2));
        promoted.promotional_credit = Some(5_000);
        assert_eq!(usage_with_tax(&promoted, false).unwrap(), 15_800);

        let inclusive = usage_line(15_000, Decimal::new(8, 2));
        assert_eq!(usage_with_tax(&inclusive, true).unwrap(), 16_200);

        let untaxed = usage_line(15_000, Decimal::ZERO);
        assert_eq!(usage_with_tax(&untaxed, false).unwrap(), 15_000);
    }

    fn purchased(classification: GrantClassification, reason: &str) -> CreditGrant {
        CreditGrant {
            id: CreditGrantId::new(),
            wallet_id: WalletId::new(),
            amount_minor: 5_000,
            remaining_minor: 5_000,
            expired_minor: None,
            classification,
            priority: 100,
            source: CreditGrantSource::Purchase,
            source_id: Uuid::now_v7(),
            invoice_id: None,
            credit_note_id: None,
            effective_at: day_start(date(1)),
            expires_at: None,
            state: CreditGrantState::Active,
            reason: reason.to_owned(),
            reference: None,
            created_by: None,
            created_at: day_start(date(1)),
            cost: None,
            scope: crate::domain::credit::GrantScope::customer_wide(),
            ends_at: None,
            end_reason: None,
            rollover: None,
        }
    }

    #[test]
    fn bonus_credit_reads_as_bonus_even_beside_an_automatic_top_up() {
        let bonus = purchased(GrantClassification::Promotional, BONUS_CREDIT);
        assert_eq!(portal_source(&bonus, true), PortalCreditSource::Bonus);
        let paid = purchased(
            GrantClassification::Paid,
            "Credit purchased on a paid invoice",
        );
        assert_eq!(portal_source(&paid, false), PortalCreditSource::Purchased);
    }
}

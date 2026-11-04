use super::{fetch_price_credit_units, fetch_prices_and_products, resolve_fee_from_maps};
use crate::StoreResult;
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::domain::{
    AttributedSubscription, MeteredSpan, Subscription, SubscriptionComponent,
    SubscriptionUsageScopes, UsageFilter, UsageFilterVersion, scopes_for,
};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_usage_filters::{
    SubscriptionUsageFilterRow, SubscriptionUsageFilterRowNew,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, bail};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(crate) fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN)
}

/// Statuses in which a subscription bills no more usage: termination (and pause) opened its last
/// period at the date it took effect, an immediate cancellation also recorded its instant.
fn ended(status: &SubscriptionStatusEnum) -> bool {
    matches!(
        status,
        SubscriptionStatusEnum::Cancelled
            | SubscriptionStatusEnum::Completed
            | SubscriptionStatusEnum::Paused
            | SubscriptionStatusEnum::Superseded
            | SubscriptionStatusEnum::Errored
            | SubscriptionStatusEnum::Aborted
    )
}

/// `[start, end)` of the usage a subscription bills, and what it has invoiced of it so far.
fn lifetime(row: &SubscriptionRow) -> (NaiveDateTime, Option<NaiveDateTime>, NaiveDateTime) {
    let start = midnight(row.start_date);
    if ended(&row.status) {
        let end = row
            .terminated_at
            .unwrap_or_else(|| midnight(row.current_period_start));
        (start, Some(end), end)
    } else {
        (
            start,
            row.end_date.map(midnight),
            midnight(row.current_period_start).max(start),
        )
    }
}

/// When the subscription stops billing usage, if it does.
pub(crate) fn life_end(row: &SubscriptionRow) -> Option<NaiveDateTime> {
    lifetime(row).1
}

fn span(
    metric_id: BillableMetricId,
    life: (NaiveDateTime, Option<NaiveDateTime>),
    effective_from: NaiveDate,
    effective_to: Option<NaiveDate>,
) -> Option<MeteredSpan> {
    let from = life.0.max(midnight(effective_from));
    let until = match (life.1, effective_to.map(midnight)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    until
        .is_none_or(|until| from < until)
        .then_some(MeteredSpan {
            metric_id,
            from,
            until,
        })
}

fn version_of(row: &SubscriptionUsageFilterRow) -> StoreResult<UsageFilterVersion> {
    Ok(UsageFilterVersion {
        filter: UsageFilter::from_columns(row.property.clone(), row.value.clone())?,
        from: row.effective_from,
        until: row.effective_until,
    })
}

/// The customer's subscriptions as usage attribution sees them (aborted ones never bill), only
/// those that ever had a usage filter when `filtered_only`.
pub async fn attributed_subscriptions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    filtered_only: bool,
) -> StoreResult<Vec<AttributedSubscription>> {
    let rows = SubscriptionRow::list_for_usage_attribution(conn, tenant_id, customer_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let mut versions: HashMap<SubscriptionId, Vec<UsageFilterVersion>> = HashMap::new();
    for row in SubscriptionUsageFilterRow::list_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        versions
            .entry(row.subscription_id)
            .or_default()
            .push(version_of(&row)?);
    }
    let rows = rows
        .into_iter()
        .filter(|row| {
            !filtered_only
                || versions
                    .get(&row.id)
                    .is_some_and(|list| list.iter().any(|version| version.filter.is_some()))
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let components = SubscriptionComponentRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = SubscriptionAddOnRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let (prices, products) = fetch_prices_and_products(
        conn,
        tenant_id,
        components
            .iter()
            .filter_map(|row| row.price_id)
            .chain(add_ons.iter().filter_map(|row| row.price_id)),
        components
            .iter()
            .filter_map(|row| row.product_id)
            .chain(add_ons.iter().filter_map(|row| row.product_id)),
    )
    .await?;
    let units = fetch_price_credit_units(conn, tenant_id, &prices).await?;

    let lives = rows
        .iter()
        .map(|row| (row.id, lifetime(row)))
        .collect::<HashMap<_, _>>();
    let mut spans: HashMap<SubscriptionId, Vec<MeteredSpan>> = HashMap::new();
    let mut push = |subscription_id: SubscriptionId,
                    metric_id: Option<BillableMetricId>,
                    from: NaiveDate,
                    to: Option<NaiveDate>| {
        if let (Some(metric_id), Some((start, end, _))) = (metric_id, lives.get(&subscription_id))
            && let Some(span) = span(metric_id, (*start, *end), from, to)
        {
            spans.entry(subscription_id).or_default().push(span);
        }
    };
    for row in components {
        let (subscription_id, from, to) =
            (row.subscription_id, row.effective_from, row.effective_to);
        let metric_id =
            match resolve_fee_from_maps(row.price_id, row.product_id, &prices, &products, &units) {
                Some((_, fee)) => fee.metric_id(),
                None => TryInto::<SubscriptionComponent>::try_into(row)?.metric_id(),
            };
        push(subscription_id, metric_id, from, to);
    }
    for row in add_ons {
        let (subscription_id, from, to) =
            (row.subscription_id, row.effective_from, row.effective_to);
        let metric_id =
            match resolve_fee_from_maps(row.price_id, row.product_id, &prices, &products, &units) {
                Some((_, fee)) => fee.metric_id(),
                None => TryInto::<SubscriptionAddOn>::try_into(row)?.fee.metric_id(),
            };
        push(subscription_id, metric_id, from, to);
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let (start, end, invoiced_until) = lives[&row.id];
            AttributedSubscription {
                id: row.id,
                created_at: row.created_at,
                start,
                end,
                invoiced_until,
                versions: versions.remove(&row.id).unwrap_or_default(),
                spans: spans.remove(&row.id).unwrap_or_default(),
            }
        })
        .collect())
}

/// The scopes of subscription `id` among the customer's `filtered` subscriptions.
pub(crate) fn scopes_among(
    id: SubscriptionId,
    created_at: NaiveDateTime,
    filtered: &[AttributedSubscription],
) -> SubscriptionUsageScopes {
    let versions = filtered
        .iter()
        .find(|subscription| subscription.id == id)
        .map_or_else(Vec::new, |subscription| subscription.versions.clone());
    scopes_for(id, created_at, &versions, filtered)
}

/// How the subscription's usage is attributed among its customer's subscriptions.
pub(crate) async fn subscription_usage_scopes(
    conn: &mut PgConn,
    subscription: &Subscription,
) -> StoreResult<SubscriptionUsageScopes> {
    let filtered =
        attributed_subscriptions(conn, subscription.tenant_id, subscription.customer_id, true)
            .await?;
    Ok(scopes_among(
        subscription.id,
        subscription.created_at,
        &filtered,
    ))
}

/// The scopes of a subscription given its row, among the customer's `filtered` subscriptions.
pub fn scopes_of_row(
    row: &SubscriptionRow,
    filtered: &[AttributedSubscription],
) -> StoreResult<SubscriptionUsageScopes> {
    Ok(scopes_among(row.id, row.created_at, filtered))
}

/// How the usage of subscription `subscription_id` is attributed.
pub(crate) async fn usage_scopes_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<SubscriptionUsageScopes> {
    let row = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .subscription;
    let filtered = attributed_subscriptions(conn, tenant_id, row.customer_id, true).await?;
    scopes_of_row(&row, &filtered)
}

/// The earliest instant a new usage filter of `subject` can apply from without changing usage
/// already invoiced: not before `own_from` (what the subject has not invoiced yet), nor before
/// what each sibling whose events the change moves has invoiced. `skip`: siblings created with it.
pub(crate) async fn usage_filter_effective_from(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subject: &SubscriptionRow,
    own_from: NaiveDateTime,
    new: Option<&UsageFilter>,
    skip: &HashSet<SubscriptionId>,
) -> StoreResult<NaiveDateTime> {
    let all = attributed_subscriptions(conn, tenant_id, subject.customer_id, false).await?;
    let Some(this) = all.iter().find(|s| s.id == subject.id) else {
        bail!(StoreError::ValueNotFound(format!(
            "subscription {} for its usage filter",
            subject.id
        )));
    };
    let old = UsageFilter::from_columns(
        subject.usage_filter_property.clone(),
        subject.usage_filter_value.clone(),
    )?;
    let properties = old
        .iter()
        .chain(new)
        .map(|filter| filter.property.as_str())
        .collect::<Vec<_>>();
    let metrics = this
        .spans
        .iter()
        .map(|span| span.metric_id)
        .collect::<HashSet<_>>();

    let mut from = own_from.max(this.start);
    if let Some(latest) = this.versions.last() {
        from = from.max(latest.from);
    }
    for sibling in all
        .iter()
        .filter(|s| s.id != this.id && !skip.contains(&s.id))
    {
        if !sibling
            .spans
            .iter()
            .any(|span| metrics.contains(&span.metric_id))
        {
            continue;
        }
        let stretches = sibling.stretches();
        let falls_back = stretches.iter().any(|(_, _, filter)| filter.is_none());
        let excludes = this.outranks(sibling.id, sibling.created_at)
            && stretches.iter().any(|(_, _, filter)| {
                filter.is_some_and(|filter| properties.iter().any(|p| *p != filter.property))
            });
        if falls_back || excludes {
            from = from.max(sibling.invoiced_until);
        }
    }
    Ok(from)
}

/// Refuses a usage filter another subscription of the customer uses over an overlapping stretch
/// of time: one of the two would bill nothing.
pub(crate) async fn ensure_usage_filter_available(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    filter: &UsageFilter,
    from: NaiveDateTime,
    until: Option<NaiveDateTime>,
) -> StoreResult<()> {
    for other in attributed_subscriptions(conn, tenant_id, customer_id, true)
        .await?
        .iter()
        .filter(|other| other.id != subscription_id)
    {
        let clash = other.stretches().into_iter().any(|(start, end, used)| {
            used == Some(filter)
                && end.is_none_or(|end| from < end)
                && until.is_none_or(|until| start < until)
        });
        if clash {
            let plan = diesel_models::query::entity_display::subscription_plan_names(
                conn,
                tenant_id,
                &[other.id.as_uuid()],
            )
            .await
            .ok()
            .and_then(|mut names| names.pop())
            // Naming the plan is a courtesy: a failed lookup keeps the refusal a refusal.
            .map_or_else(
                || "another plan".to_owned(),
                |(_, name)| format!("\"{name}\""),
            );
            bail!(StoreError::InvalidArgument(format!(
                "this customer's subscription to {plan}, started {}, already bills the events where {} is {}",
                other.start.format("%b %-d, %Y"),
                filter.property,
                filter.value
            )));
        }
    }
    Ok(())
}

/// Starts a new version of the subscription's usage filter at `from` (none: no filter from then).
pub(crate) async fn record_usage_filter_version(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    filter: Option<&UsageFilter>,
    from: NaiveDateTime,
) -> StoreResult<()> {
    let versions =
        SubscriptionUsageFilterRow::list_by_subscriptions(conn, tenant_id, &[subscription_id])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let (property, value) = filter
        .map(|filter| (filter.property.clone(), filter.value.clone()))
        .unzip();
    let latest = versions.last().filter(|row| row.effective_until.is_none());
    match latest {
        Some(row) if row.property == property && row.value == value => return Ok(()),
        Some(row) if row.effective_from >= from => {
            return SubscriptionUsageFilterRow::set_filter(
                conn, tenant_id, row.id, property, value,
            )
            .await
            .map_err(Into::into);
        }
        Some(row) => {
            SubscriptionUsageFilterRow::close(conn, tenant_id, row.id, from)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }
        None if filter.is_none() => return Ok(()),
        None => {}
    }
    SubscriptionUsageFilterRowNew {
        id: Uuid::now_v7(),
        tenant_id,
        subscription_id,
        property,
        value,
        effective_from: from,
        effective_until: None,
    }
    .insert(conn)
    .await
    .map_err(Into::into)
}

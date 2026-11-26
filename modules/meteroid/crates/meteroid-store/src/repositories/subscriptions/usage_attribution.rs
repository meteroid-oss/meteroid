use super::{ChargeShapes, fetch_price_credit_units, fetch_prices, resolve_fee_from_maps};
use crate::StoreResult;
use crate::domain::credit_units::CreditUnitsById;
use crate::domain::prices::Price;
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::domain::{
    AttributedResource, AttributedSubscription, MeteredSpan, MetricCodes, SaleWarnings,
    Subscription, SubscriptionComponent, SubscriptionUsageScopes, UsageFloor, scopes_for,
};
use crate::errors::StoreError;
use crate::repositories::resources::resource_at_fault;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, PlanVersionId, PriceId, ResourceId, ResourceTypeId,
    SubscriptionId, TenantId,
};
use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::PgConn;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::enums::{SubscriptionActivationConditionEnum, SubscriptionStatusEnum};
use diesel_models::errors::IntoDbResult;
use diesel_models::resources::{ResourceRow, ResourceTypeKeyRow};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, bail};
use std::collections::HashMap;

pub(crate) fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN)
}

/// The first whole day from `at`.
fn first_day_from(at: NaiveDateTime) -> NaiveDate {
    if at == midnight(at.date()) {
        at.date()
    } else {
        at.date() + chrono::Days::new(1)
    }
}

/// Statuses in which a subscription bills no more usage.
fn ended(status: &SubscriptionStatusEnum) -> bool {
    matches!(
        status,
        SubscriptionStatusEnum::Cancelled
            | SubscriptionStatusEnum::Completed
            | SubscriptionStatusEnum::Aborted
    )
}

/// When an ended subscription (not an aborted one, which claims nothing) stopped claiming: its
/// termination, else, with no end date to end it either, its current period's start, as on main.
fn ended_at(
    status: &SubscriptionStatusEnum,
    terminated_at: Option<NaiveDateTime>,
    end_date: Option<NaiveDate>,
    current_period_start: NaiveDate,
) -> Option<NaiveDateTime> {
    if !matches!(
        status,
        SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Completed
    ) {
        return None;
    }
    terminated_at.or_else(|| end_date.is_none().then(|| midnight(current_period_start)))
}

/// The facts a subscription's claim on usage is read from (ADR 0011 §3.6).
#[derive(Debug, Clone, Copy, Default)]
struct ClaimFacts {
    settled: Option<NaiveDateTime>,
    cancelled_at: Option<NaiveDateTime>,
}

/// A subscription's claim on usage: `[start, end)`, no start while its start is not settled (it
/// claims nothing).
struct Life {
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
}

/// A subscription's dates and status as its claim reads them.
struct Dates {
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    activated_at: Option<NaiveDateTime>,
    on_start: bool,
    aborted: bool,
    /// When it terminated, once its status says it ended (see [`ended_at`]).
    terminated_at: Option<NaiveDateTime>,
}

impl From<&SubscriptionRow> for Dates {
    fn from(row: &SubscriptionRow) -> Self {
        Self {
            start_date: row.start_date,
            end_date: row.end_date,
            activated_at: row.activated_at,
            on_start: row.activation_condition == SubscriptionActivationConditionEnum::OnStart,
            aborted: row.status == SubscriptionStatusEnum::Aborted,
            terminated_at: ended_at(
                &row.status,
                row.terminated_at,
                row.end_date,
                row.current_period_start,
            ),
        }
    }
}

/// From its settled start, never before its start date, to the earlier of its cancellation (ended
/// without one, its termination) and its end date. A row without a start fact settles as
/// the release's backfill settles it.
fn lifetime(dates: &Dates, facts: ClaimFacts) -> Life {
    let settled = facts.settled.or_else(|| {
        dates
            .activated_at
            .or_else(|| dates.on_start.then(|| midnight(dates.start_date)))
    });
    let start = settled
        .filter(|_| !dates.aborted)
        .map(|at| at.max(midnight(dates.start_date)));
    let ended_at = facts.cancelled_at.or(dates.terminated_at);
    let end = match (ended_at, dates.end_date.map(midnight)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
    .map(|end| start.map_or(end, |start| end.max(start)));
    Life { start, end }
}

/// The day a start settled after its start date, from which its own usage counts (§3.6).
fn activated_from(start_date: NaiveDate, life: &Life) -> Option<NaiveDateTime> {
    life.start.filter(|start| *start > midnight(start_date))
}

fn span(
    metric_id: common_domain::ids::BillableMetricId,
    life: (NaiveDateTime, Option<NaiveDateTime>),
    effective_from: NaiveDate,
    effective_to: Option<NaiveDate>,
    floor: Option<NaiveDateTime>,
) -> Option<MeteredSpan> {
    let from = life
        .0
        .max(midnight(effective_from))
        .max(floor.unwrap_or_default());
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

/// The settled starts and cancellations of `ids`.
async fn claim_facts(
    conn: &mut PgConn,
    ids: &[SubscriptionId],
) -> StoreResult<HashMap<SubscriptionId, ClaimFacts>> {
    use diesel_models::schema::subscription_ending::dsl as e;
    use diesel_models::schema::subscription_start_settled::dsl as st;
    use diesel_models::subscription_facts::ENDING_CANCEL;
    let mut facts: HashMap<SubscriptionId, ClaimFacts> = HashMap::new();
    let starts: Vec<(SubscriptionId, NaiveDateTime)> = st::subscription_start_settled
        .filter(st::subscription_id.eq_any(ids))
        .select((st::subscription_id, st::at))
        .load(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    for (id, at) in starts {
        facts.entry(id).or_default().settled = Some(at);
    }
    let cancellations: Vec<(SubscriptionId, NaiveDateTime)> = e::subscription_ending
        .filter(e::subscription_id.eq_any(ids))
        .filter(e::kind.eq(ENDING_CANCEL))
        .select((e::subscription_id, e::at))
        .load(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    for (id, at) in cancellations {
        facts.entry(id).or_default().cancelled_at = Some(at);
    }
    Ok(facts)
}

#[derive(diesel::QueryableByName)]
struct FinalizedLines {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    subscription_id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    metric_id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    until: NaiveDateTime,
}

#[derive(diesel::QueryableByName)]
struct FinalizedDocuments {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    subscription_id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    until: NaiveDateTime,
}

/// How far subscriptions' documents that are no longer drafts billed their usage: never billed
/// again (R-091).
#[derive(Debug, Default)]
pub(crate) struct Ground {
    /// Per metric, how far their usage lines reach.
    lines: HashMap<(SubscriptionId, BillableMetricId), NaiveDateTime>,
    /// How far their recurring and final documents billed usage in arrears, lines or none.
    documents: HashMap<SubscriptionId, NaiveDateTime>,
}

impl Ground {
    /// How far `subscription` billed `metric`: its lines, or a document billing the usage of one of
    /// its spans pricing it, though it had none.
    fn of(
        &self,
        subscription: &AttributedSubscription,
        metric: BillableMetricId,
    ) -> Option<NaiveDateTime> {
        let lines = self.lines.get(&(subscription.id, metric)).copied();
        let documents = self.documents.get(&subscription.id).and_then(|until| {
            subscription
                .spans
                .iter()
                .filter(|span| span.metric_id == metric && span.from < *until)
                .map(|span| span.until.map_or(*until, |end| end.min(*until)))
                .max()
        });
        lines.max(documents)
    }
}

/// Invoice `i`, its parent `parent` when consolidated, is no longer a draft: read through the one a
/// draft was consolidated into, or a draft edited by hand, which nothing recomputes. Void ones are
/// billed again.
pub(crate) const FINALIZED_SQL: &str = "
    (COALESCE(parent.status, i.status) NOT IN ('DRAFT', 'VOID')
     OR (COALESCE(parent.status, i.status) = 'DRAFT'
         AND CASE WHEN parent.id IS NULL THEN i.manually_edited_at
                  ELSE parent.manually_edited_at END IS NOT NULL))";

/// A line cut at an instant (an immediate cancellation) reaches that instant, else its end date's
/// midnight. A recurring document bills usage up to its period's start, a final one to the ending.
async fn finalized_ground(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[SubscriptionId],
) -> StoreResult<Ground> {
    let ids = ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
    let lines: Vec<FinalizedLines> = diesel::sql_query(format!(
        "SELECT i.subscription_id, (l->>'metric_id')::uuid AS metric_id,
                max(COALESCE((l->>'usage_ends_at')::timestamptz AT TIME ZONE 'UTC',
                             (l->>'end_date')::date::timestamp)) AS until
         FROM invoice i
         LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
         CROSS JOIN LATERAL jsonb_array_elements(i.line_items) l
         WHERE {FINALIZED_SQL}
           AND l->>'metric_id' IS NOT NULL
           AND i.tenant_id = $1 AND i.subscription_id = ANY($2)
         GROUP BY 1, 2"
    ))
    .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(&ids)
    .load(conn)
    .await
    .into_db_result()
    .map_err(Into::<Report<StoreError>>::into)?;
    let documents: Vec<FinalizedDocuments> = diesel::sql_query(format!(
        "SELECT i.subscription_id,
                max(CASE WHEN i.invoice_type = 'FINAL'
                         THEN COALESCE(e.at, s.terminated_at, i.invoice_date::timestamp)
                         ELSE i.billing_period_start::timestamp END) AS until
         FROM invoice i
         LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
         JOIN subscription s ON s.id = i.subscription_id
         LEFT JOIN subscription_ending e ON e.subscription_id = s.id AND e.kind = 'CANCEL'
         WHERE {FINALIZED_SQL}
           AND i.tenant_id = $1 AND i.subscription_id = ANY($2)
           AND i.invoice_type IN ('RECURRING', 'FINAL')
         GROUP BY 1"
    ))
    .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(&ids)
    .load(conn)
    .await
    .into_db_result()
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(Ground {
        lines: lines
            .into_iter()
            .map(|row| {
                (
                    (row.subscription_id.into(), row.metric_id.into()),
                    row.until,
                )
            })
            .collect(),
        documents: documents
            .into_iter()
            .map(|row| (row.subscription_id.into(), row.until))
            .collect(),
    })
}

/// The stored resources of `ids` as attribution reads them, with their type's keys.
pub(crate) async fn attributed_resources(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[ResourceId],
) -> StoreResult<HashMap<ResourceId, AttributedResource>> {
    use diesel_models::schema::resource::dsl as r;
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let resources: Vec<(ResourceId, ResourceTypeId, String)> = r::resource
        .filter(r::tenant_id.eq(tenant_id))
        .filter(r::id.eq_any(ids.to_vec()))
        .select((r::id, r::resource_type_id, r::key_value))
        .load(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut keys: HashMap<ResourceTypeId, Vec<String>> = HashMap::new();
    for row in ResourceTypeKeyRow::list_by_tenant(conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        keys.entry(row.resource_type_id).or_default().push(row.key);
    }
    Ok(resources
        .into_iter()
        .map(|(id, type_id, key_value)| {
            let keys = keys.get(&type_id).cloned().unwrap_or_default();
            (
                id,
                AttributedResource {
                    type_id,
                    keys,
                    key_value,
                },
            )
        })
        .collect())
}

/// A subscription or quote is for a stored, unarchived resource of its own customer. Its type's
/// keys can't change until the caller's transaction commits (R-144).
pub(crate) async fn ensure_resource_for(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    resource_id: ResourceId,
) -> StoreResult<ResourceRow> {
    let resource = ResourceRow::find_by_id(conn, tenant_id, resource_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .filter(|resource| resource.customer_id == customer_id)
        .ok_or_else(|| {
            resource_at_fault(Report::new(StoreError::InvalidArgument(format!(
                "resource {resource_id} is not a stored resource of this customer"
            ))))
        })?;
    diesel_models::resources::ResourceTypeRow::lock_keys_shared(conn, resource.resource_type_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if resource.archived_at.is_some() {
        return Err(resource_at_fault(Report::new(StoreError::InvalidArgument(
            format!("resource {resource_id} is archived"),
        ))));
    }
    Ok(resource)
}

/// What a sale of `plan_version_id` pricing `metric_ids`, for a resource of `resource_type_id`
/// (`None`: the customer), is accepted with: its plan sold for another target, or metrics its
/// resource never bills (§3.5, §3.6). Warnings, never refusals.
pub(crate) async fn sale_warnings(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    resource_type_id: Option<ResourceTypeId>,
    metric_ids: &[BillableMetricId],
) -> StoreResult<SaleWarnings> {
    use diesel_models::schema::{plan, plan_version};
    let sold_for: Option<ResourceTypeId> = plan_version::table
        .inner_join(plan::table.on(plan::id.eq(plan_version::plan_id)))
        .filter(plan_version::tenant_id.eq(tenant_id))
        .filter(plan_version::id.eq(plan_version_id))
        .select(plan::sold_for_resource_type_id)
        .first(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    let not_attributed = match resource_type_id {
        Some(type_id) if !metric_ids.is_empty() => {
            BillableMetricRow::get_by_ids(conn, metric_ids, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .filter(|metric| metric.attributed_to_type_id != Some(type_id))
                .map(|metric| MetricCodes {
                    catalog_code: metric.catalog_code,
                    code: metric.code,
                })
                .collect()
        }
        _ => Vec::new(),
    };
    Ok(SaleWarnings::new(
        sold_for != resource_type_id,
        not_attributed,
    ))
}

/// What subscription `subscription_id` is sold with as it stands, read on every read of it: its
/// plan sold for another target, metrics its rows billing today or later price that its resource
/// never bills (§3.5, §3.6).
pub(crate) async fn subscription_sale_warnings(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<SaleWarnings> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let resource_type_id = match row.resource_id {
        Some(resource_id) => {
            let Some(resource) = ResourceRow::find_by_id(conn, tenant_id, resource_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
            else {
                return Ok(SaleWarnings::default());
            };
            Some(resource.resource_type_id)
        }
        None => None,
    };
    let today = common_clock::now().date_naive();
    let mut metric_ids = metered_rows(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .filter(|row| row.to.is_none_or(|to| to > today))
        .map(|row| row.metric_id)
        .collect::<Vec<_>>();
    metric_ids.sort_by_key(|id| id.as_uuid());
    metric_ids.dedup();
    sale_warnings(
        conn,
        tenant_id,
        row.plan_version_id,
        resource_type_id,
        &metric_ids,
    )
    .await
}

/// Customers of the tenant with a live subscription for a resource: the only ones whose usage a
/// change of attribution moves.
pub(crate) async fn customers_with_resource_subscriptions(
    conn: &mut PgConn,
    tenant_id: TenantId,
) -> StoreResult<Vec<CustomerId>> {
    use diesel_models::schema::subscription::dsl as s;
    s::subscription
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::resource_id.is_not_null())
        .filter(s::status.eq_any(vec![
            SubscriptionStatusEnum::Active,
            SubscriptionStatusEnum::TrialActive,
        ]))
        .select(s::customer_id)
        .distinct()
        .load(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)
}

/// Changes the resource of a subscription not yet active; the caller checks both.
pub(crate) async fn set_resource(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    resource_id: Option<ResourceId>,
) -> StoreResult<()> {
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription)
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::id.eq(subscription_id))
        .set(s::resource_id.eq(resource_id))
        .execute(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    mark(conn, tenant_id, row.customer_id).await
}

/// The customer's subscriptions as usage attribution sees them (aborted ones never bill), only
/// those for a resource when `resource_only`.
pub async fn attributed_subscriptions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    resource_only: bool,
) -> StoreResult<Vec<AttributedSubscription>> {
    let rows = SubscriptionRow::list_for_usage_attribution(conn, tenant_id, customer_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|row| !resource_only || row.resource_id.is_some())
        .collect::<Vec<_>>();
    attribute(conn, tenant_id, rows).await
}

/// The customer's subscriptions that may bill usage now, as attribution sees them: not those that
/// ended, whose history a live check never reads.
pub async fn billing_subscriptions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<Vec<AttributedSubscription>> {
    let rows = SubscriptionRow::list_for_usage_attribution(conn, tenant_id, customer_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|row| !ended(&row.status))
        .collect::<Vec<_>>();
    attribute(conn, tenant_id, rows).await
}

async fn attribute(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: Vec<SubscriptionRow>,
) -> StoreResult<Vec<AttributedSubscription>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let resource_ids = rows
        .iter()
        .filter_map(|row| row.resource_id)
        .collect::<Vec<_>>();
    let resources = attributed_resources(conn, tenant_id, &resource_ids).await?;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let components = SubscriptionComponentRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = SubscriptionAddOnRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let fees = RowFees::load(conn, tenant_id, &components, &add_ons).await?;

    let facts = claim_facts(conn, &ids).await?;
    let row_floors = components
        .iter()
        .filter_map(|row| row.usage_from.map(|at| (row.id.as_uuid(), at.naive_utc())))
        .chain(
            add_ons
                .iter()
                .filter_map(|row| row.usage_from.map(|at| (row.id.as_uuid(), at.naive_utc()))),
        )
        .collect::<HashMap<_, _>>();
    let lives = rows
        .iter()
        .map(|row| {
            let life = lifetime(
                &Dates::from(row),
                facts.get(&row.id).copied().unwrap_or_default(),
            );
            (row.id, life)
        })
        .collect::<HashMap<_, _>>();
    // What a subscription would bill once started, for those looking ahead.
    let bills = rows
        .iter()
        .map(|row| {
            let life = &lives[&row.id];
            let start = life.start.unwrap_or_else(|| midnight(row.start_date));
            (row.id, (start, life.end))
        })
        .collect::<HashMap<_, _>>();
    let mut spans: HashMap<SubscriptionId, Vec<MeteredSpan>> = HashMap::new();
    let mut floors: HashMap<SubscriptionId, Vec<UsageFloor>> = HashMap::new();
    let mut push = |subscription_id: SubscriptionId,
                    row_id: uuid::Uuid,
                    metric_id: Option<common_domain::ids::BillableMetricId>,
                    from: NaiveDate,
                    to: Option<NaiveDate>| {
        let Some(metric_id) = metric_id else {
            return;
        };
        let floor = row_floors.get(&row_id).copied();
        if let Some(until) = floor {
            floors.entry(subscription_id).or_default().push(UsageFloor {
                metric_id,
                from: midnight(from),
                until,
            });
        }
        if let Some(life) = bills.get(&subscription_id)
            && let Some(span) = span(metric_id, *life, from, to, floor)
        {
            spans.entry(subscription_id).or_default().push(span);
        }
    };
    for row in components {
        let (subscription_id, row_id, from, to) = (
            row.subscription_id,
            row.id.as_uuid(),
            row.effective_from,
            row.effective_to,
        );
        let metric_id = fees.component_metric(row)?;
        push(subscription_id, row_id, metric_id, from, to);
    }
    for row in add_ons {
        let (subscription_id, row_id, from, to) = (
            row.subscription_id,
            row.id.as_uuid(),
            row.effective_from,
            row.effective_to,
        );
        let metric_id = fees.add_on_metric(row)?;
        push(subscription_id, row_id, metric_id, from, to);
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let life = &lives[&row.id];
            AttributedSubscription {
                id: row.id,
                created_at: row.created_at,
                start: bills[&row.id].0,
                end: life.end,
                resource: row.resource_id.and_then(|id| resources.get(&id).cloned()),
                spans: spans.remove(&row.id).unwrap_or_default(),
                awaiting_activation: life.start.is_none(),
                activated_from: activated_from(row.start_date, life),
                claimed_until: life.end,
                floors: floors.remove(&row.id).unwrap_or_default(),
            }
        })
        .collect())
}

/// The scopes of subscription `id` among the customer's subscriptions for a resource. One for a
/// resource reads its own usage within `own`: from an activation after its start date, to the end
/// its decision stored. A customer-level one reads its periods' usage, as before resources.
pub(crate) fn scopes_among(
    id: SubscriptionId,
    created_at: NaiveDateTime,
    own: OwnBounds,
    for_resources: &[AttributedSubscription],
) -> SubscriptionUsageScopes {
    let resource = for_resources
        .iter()
        .find(|subscription| subscription.id == id)
        .and_then(|subscription| subscription.resource.clone());
    let mut scopes = scopes_for(id, created_at, resource, for_resources);
    if scopes.resource.is_some() {
        scopes.from = own.from.or(scopes.from);
        scopes.until = own.until.or(scopes.until);
    }
    scopes
}

/// Where a subscription's own usage is read: from an activation after its start date, to the end
/// the decision to end it stored (§3.6). Unbounded otherwise.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OwnBounds {
    from: Option<NaiveDateTime>,
    until: Option<NaiveDateTime>,
}

async fn own_bounds(
    conn: &mut PgConn,
    id: SubscriptionId,
    dates: &Dates,
) -> StoreResult<OwnBounds> {
    let facts = claim_facts(conn, &[id]).await?;
    let life = lifetime(dates, facts.get(&id).copied().unwrap_or_default());
    Ok(OwnBounds {
        from: activated_from(dates.start_date, &life),
        until: life.end,
    })
}

/// How the subscription's usage is attributed among its customer's subscriptions; it may not be
/// written yet (a preview).
pub(crate) async fn subscription_usage_scopes(
    conn: &mut PgConn,
    subscription: &Subscription,
) -> StoreResult<SubscriptionUsageScopes> {
    let for_resources =
        attributed_subscriptions(conn, subscription.tenant_id, subscription.customer_id, true)
            .await?;
    let dates = Dates {
        start_date: subscription.start_date,
        end_date: subscription.end_date,
        activated_at: subscription.activated_at,
        on_start: subscription.activation_condition
            == crate::domain::enums::SubscriptionActivationCondition::OnStart,
        aborted: subscription.status == crate::domain::enums::SubscriptionStatusEnum::Aborted,
        terminated_at: ended_at(
            &subscription.status.clone().into(),
            subscription.terminated_at,
            subscription.end_date,
            subscription.current_period_start,
        ),
    };
    let own = own_bounds(conn, subscription.id, &dates).await?;
    Ok(scopes_among(
        subscription.id,
        subscription.created_at,
        own,
        &for_resources,
    ))
}

/// How the usage of subscription `subscription_id` is attributed.
pub(crate) async fn usage_scopes_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<SubscriptionUsageScopes> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let for_resources = attributed_subscriptions(conn, tenant_id, row.customer_id, true).await?;
    let own = own_bounds(conn, row.id, &Dates::from(&row)).await?;
    Ok(scopes_among(row.id, row.created_at, own, &for_resources))
}

/// The metrics among `subscriptions`' spans attributed to a type, with their type: those a
/// subscription for a resource of that type claims.
async fn attributed_metrics(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscriptions: &[AttributedSubscription],
) -> StoreResult<HashMap<common_domain::ids::BillableMetricId, ResourceTypeId>> {
    let metric_ids = subscriptions
        .iter()
        .flat_map(|s| s.spans.iter().map(|span| span.metric_id))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(BillableMetricRow::get_by_ids(conn, &metric_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter_map(|m| m.attributed_to_type_id.map(|type_id| (m.id, type_id)))
        .collect())
}

/// Refuses a subscription newly for a resource (`subjects`, created or moved) that would claim
/// events of a metric a customer-level subscription pricing it has already invoiced: they would be
/// billed twice. `created`: subscriptions of this write, which have invoiced nothing.
pub(crate) async fn ensure_claims_uninvoiced(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subjects: &[(CustomerId, SubscriptionId)],
    created: &[SubscriptionId],
) -> StoreResult<()> {
    let mut customers = subjects
        .iter()
        .map(|(customer, _)| *customer)
        .collect::<Vec<_>>();
    customers.sort_by_key(|id| id.as_uuid());
    customers.dedup();
    for customer_id in customers {
        let all = attributed_subscriptions(conn, tenant_id, customer_id, false).await?;
        let attributed_to = attributed_metrics(conn, tenant_id, &all).await?;
        let ground = finalized_ground(
            conn,
            tenant_id,
            &all.iter().map(|s| s.id).collect::<Vec<_>>(),
        )
        .await?;
        for new in all
            .iter()
            .filter(|s| subjects.iter().any(|(_, id)| *id == s.id))
        {
            // Its claim starts at its activation, past anything invoiced by then.
            if new.awaiting_activation {
                continue;
            }
            let Some(resource) = &new.resource else {
                continue;
            };
            let others = all
                .iter()
                .filter(|other| other.resource.is_none() && !created.contains(&other.id))
                .collect::<Vec<_>>();
            let invoiced = new
                .spans
                .iter()
                .map(|span| span.metric_id)
                .filter(|metric| attributed_to.get(metric) == Some(&resource.type_id))
                .filter_map(|metric| ground_of(&ground, &others, metric))
                .max();
            if let Some(until) = invoiced.filter(|until| *until > new.start) {
                return Err(resource_at_fault(Report::new(StoreError::InvalidArgument(
                    format!(
                        "a subscription for this resource cannot start before {}: a \
                         subscription of the customer has invoiced its usage up to then",
                        first_day_from(until).format("%b %-d, %Y")
                    ),
                ))));
            }
        }
        if all
            .iter()
            .any(|s| s.resource.is_some() && subjects.iter().any(|(_, id)| *id == s.id))
        {
            mark(conn, tenant_id, customer_id).await?;
        }
    }
    Ok(())
}

/// How far `subscriptions` pricing `metric` have finalized its usage.
fn ground_of(
    ground: &Ground,
    subscriptions: &[&AttributedSubscription],
    metric: BillableMetricId,
) -> Option<NaiveDateTime> {
    subscriptions
        .iter()
        .filter_map(|s| ground.of(s, metric))
        .max()
}

/// Per metric of its resource's type it may bill, how far the subscriptions a change of `row`'s
/// claim moves its usage with have finalized it: the customer-level ones and those for its resource
/// pricing that metric (ADR 0011 §3.6). A customer-level subject moves no one's.
async fn finalized_by_others(
    conn: &mut PgConn,
    row: &SubscriptionRow,
) -> StoreResult<HashMap<BillableMetricId, NaiveDateTime>> {
    if row.resource_id.is_none() {
        return Ok(HashMap::new());
    }
    let all = attributed_subscriptions(conn, row.tenant_id, row.customer_id, false).await?;
    let Some(resource) = all
        .iter()
        .find(|s| s.id == row.id)
        .and_then(|s| s.resource.clone())
    else {
        return Ok(HashMap::new());
    };
    let attributed_to = attributed_metrics(conn, row.tenant_id, &all).await?;
    let others = all
        .iter()
        .filter(|s| s.id != row.id && s.resource.as_ref().is_none_or(|r| *r == resource))
        .collect::<Vec<_>>();
    let ground = finalized_ground(
        conn,
        row.tenant_id,
        &others.iter().map(|s| s.id).collect::<Vec<_>>(),
    )
    .await?;
    Ok(attributed_to
        .iter()
        .filter(|(_, type_id)| **type_id == resource.type_id)
        .filter_map(|(metric, _)| ground_of(&ground, &others, *metric).map(|g| (*metric, g)))
        .collect())
}

/// Refuses a change moving `subscription_id`'s claimed usage from `at` on, between it and the
/// customer's other subscriptions, once one of them finalized usage past `at` (R-143): billed
/// there already, it would be billed twice or never.
pub(crate) async fn ensure_unfinalized_move(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: NaiveDateTime,
    change: &str,
) -> StoreResult<()> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let ground = finalized_by_others(conn, &row).await?;
    let priced = metered_rows(conn, tenant_id, subscription_id).await?;
    let held = priced
        .iter()
        .filter_map(|span| ground.get(&span.metric_id).copied())
        .max();
    match held {
        Some(until) if until > at => bail!(StoreError::InvalidArgument(format!(
            "{change} would move usage of this resource that another subscription of the \
             customer has invoiced up to {}: date it from then",
            first_day_from(until).format("%b %-d, %Y")
        ))),
        _ => Ok(()),
    }
}

/// A plan change or amendment dated `applied`, its rows written, before what another subscription
/// of the customer finalized (R-143): a row it adds bills its attributed metric from that ground
/// on (its floor); one it ends, no other row billing that metric on, is refused.
pub(crate) async fn settle_late_change(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    applied: NaiveDate,
    change: &str,
) -> StoreResult<()> {
    use diesel_models::schema::resource::dsl as r;
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let Some(resource_id) = row.resource_id else {
        return Ok(());
    };
    mark(conn, tenant_id, row.customer_id).await?;
    let at = midnight(applied);
    let ground = finalized_by_others(conn, &row).await?;
    let held = |metric: &BillableMetricId| ground.get(metric).copied().filter(|held| *held > at);
    if ground.values().all(|held| *held <= at) {
        return Ok(());
    }
    let type_id: ResourceTypeId = r::resource
        .filter(r::tenant_id.eq(tenant_id))
        .filter(r::id.eq(resource_id))
        .select(r::resource_type_id)
        .first(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    let spans = metered_rows(conn, tenant_id, subscription_id).await?;
    let metric_ids = spans.iter().map(|span| span.metric_id).collect::<Vec<_>>();
    let attributed = BillableMetricRow::get_by_ids(conn, &metric_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|metric| metric.attributed_to_type_id == Some(type_id))
        .map(|metric| metric.id)
        .collect::<std::collections::HashSet<_>>();
    // Whether one of its rows bills `metric` on the day before `applied`, or on `applied`.
    let bills = |metric: BillableMetricId, before: bool| {
        spans.iter().any(|span| {
            span.metric_id == metric
                && if before {
                    span.from < applied && span.to.is_none_or(|to| to >= applied)
                } else {
                    span.from <= applied && span.to.is_none_or(|to| to > applied)
                }
        })
    };
    if let Some(until) = spans
        .iter()
        .filter(|span| {
            span.to == Some(applied)
                && attributed.contains(&span.metric_id)
                && !bills(span.metric_id, false)
        })
        .filter_map(|span| held(&span.metric_id))
        .max()
    {
        bail!(StoreError::InvalidArgument(format!(
            "{change} would end a fee over usage of this resource another subscription of the \
             customer has invoiced up to {}: date it from then",
            first_day_from(until).format("%b %-d, %Y")
        )));
    }
    let mut floors: HashMap<NaiveDateTime, Vec<FeeRowId>> = HashMap::new();
    for span in spans.iter().filter(|span| {
        span.from == applied && attributed.contains(&span.metric_id) && !bills(span.metric_id, true)
    }) {
        if let Some(until) = held(&span.metric_id) {
            floors.entry(until).or_default().push(span.row);
        }
    }
    if floors.is_empty() {
        return Ok(());
    }
    for (until, rows) in floors {
        crate::services::facts::record_usage_floor_tx(conn, &rows, until).await?;
        log::info!(
            "{change} of subscription {subscription_id} dated {applied}: {} new row(s) bill \
             their metric from {until}, what another subscription invoiced before it",
            rows.len()
        );
    }
    Ok(())
}

/// Resolves the metric a component or add-on row bills: its fee from its charge's structure and
/// its price, else from its stored fee.
struct RowFees {
    prices: HashMap<PriceId, Price>,
    shapes: ChargeShapes,
    units: CreditUnitsById,
}

impl RowFees {
    async fn load(
        conn: &mut PgConn,
        tenant_id: TenantId,
        components: &[SubscriptionComponentRow],
        add_ons: &[SubscriptionAddOnRow],
    ) -> StoreResult<Self> {
        let prices = fetch_prices(
            conn,
            tenant_id,
            components
                .iter()
                .filter_map(|row| row.price_id)
                .chain(add_ons.iter().filter_map(|row| row.price_id)),
        )
        .await?;
        let shapes = ChargeShapes::load(conn, tenant_id, components, add_ons).await?;
        let units = fetch_price_credit_units(conn, tenant_id, &prices).await?;
        Ok(Self {
            prices,
            shapes,
            units,
        })
    }

    fn component_metric(
        &self,
        row: SubscriptionComponentRow,
    ) -> StoreResult<Option<BillableMetricId>> {
        let shape = self.shapes.of_component(&row);
        Ok(
            match resolve_fee_from_maps(row.price_id, shape.as_ref(), &self.prices, &self.units) {
                Some((_, fee)) => fee.metric_id(),
                None => TryInto::<SubscriptionComponent>::try_into(row)?.metric_id(),
            },
        )
    }

    fn add_on_metric(&self, row: SubscriptionAddOnRow) -> StoreResult<Option<BillableMetricId>> {
        let shape = self.shapes.of_add_on(&row);
        Ok(
            match resolve_fee_from_maps(row.price_id, shape.as_ref(), &self.prices, &self.units) {
                Some((_, fee)) => fee.metric_id(),
                None => TryInto::<SubscriptionAddOn>::try_into(row)?.fee.metric_id(),
            },
        )
    }
}

/// A component or add-on row billing a metric, with its dates.
struct MeteredRow {
    row: FeeRowId,
    metric_id: common_domain::ids::BillableMetricId,
    from: NaiveDate,
    to: Option<NaiveDate>,
}

/// A fee row of a subscription, of either kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeeRowId {
    Component(common_domain::ids::SubscriptionPriceComponentId),
    AddOn(common_domain::ids::SubscriptionAddOnId),
}

/// The subscription's rows billing a metric, ended ones included.
async fn metered_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<MeteredRow>> {
    let components =
        SubscriptionComponentRow::list_all_by_subscriptions(conn, tenant_id, &[subscription_id])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons =
        SubscriptionAddOnRow::list_all_by_subscriptions(conn, tenant_id, &[subscription_id])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let fees = RowFees::load(conn, tenant_id, &components, &add_ons).await?;
    let mut rows = Vec::new();
    for row in components {
        let (id, from, to) = (row.id, row.effective_from, row.effective_to);
        let metric_id = fees.component_metric(row)?;
        if let Some(metric_id) = metric_id {
            rows.push(MeteredRow {
                row: FeeRowId::Component(id),
                metric_id,
                from,
                to,
            });
        }
    }
    for row in add_ons {
        let (id, from, to) = (row.id, row.effective_from, row.effective_to);
        let metric_id = fees.add_on_metric(row)?;
        if let Some(metric_id) = metric_id {
            rows.push(MeteredRow {
                row: FeeRowId::AddOn(id),
                metric_id,
                from,
                to,
            });
        }
    }
    Ok(rows)
}

async fn subscription_row(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<SubscriptionRow> {
    Ok(
        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .subscription,
    )
}

/// The customer's claims on usage moved: its credit is recomputed, and its drafts not edited by
/// hand are refreshed by the sweep, refused a hand edit until then (R-143).
async fn mark(conn: &mut PgConn, tenant_id: TenantId, customer_id: CustomerId) -> StoreResult<()> {
    diesel_models::customer_credit::CreditRecomputeMarkRow::mark_one(conn, tenant_id, customer_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    diesel_models::query::invoice_claims_moved::InvoiceClaimsMoved::mark_customer_drafts(
        conn,
        tenant_id,
        customer_id,
        common_clock::now().naive_utc(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(())
}

/// [`mark`] for a change of `subscription_id`'s claim, when it is for a resource.
pub(crate) async fn claims_moved(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    match row.resource_id {
        Some(_) => mark(conn, tenant_id, row.customer_id).await,
        None => Ok(()),
    }
}

/// Refuses a change releasing what `subscription_id` claims from its settled start (aborting it,
/// changing its resource) once another subscription of the customer finalized usage past it.
pub(crate) async fn ensure_claim_releasable(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &str,
) -> StoreResult<()> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let facts = claim_facts(conn, &[subscription_id]).await?;
    let life = lifetime(
        &Dates::from(&row),
        facts.get(&subscription_id).copied().unwrap_or_default(),
    );
    match life.start {
        Some(start) => ensure_unfinalized_move(conn, tenant_id, subscription_id, start, change)
            .await
            .map_err(resource_at_fault),
        None => Ok(()),
    }
}

/// Statuses a subscription never bills again from: anything else may still sell its resource.
fn terminated() -> Vec<SubscriptionStatusEnum> {
    vec![
        SubscriptionStatusEnum::Cancelled,
        SubscriptionStatusEnum::Aborted,
        SubscriptionStatusEnum::Completed,
    ]
}

/// A subscription for a resource, never aborted, pricing a metric.
pub(crate) struct PricingSubscription {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub live: bool,
}

/// The subscriptions for a resource, aborted ones aside, that price `metric_id` through a
/// component or an add-on, past or future.
pub(crate) async fn priced_for_a_resource(
    conn: &mut PgConn,
    tenant_id: TenantId,
    metric_id: common_domain::ids::BillableMetricId,
) -> StoreResult<Vec<PricingSubscription>> {
    use diesel_models::schema::subscription::dsl as s;
    let subscriptions: Vec<(SubscriptionId, CustomerId, SubscriptionStatusEnum)> = s::subscription
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::resource_id.is_not_null())
        .filter(s::status.ne(SubscriptionStatusEnum::Aborted))
        .select((s::id, s::customer_id, s::status))
        .load(conn)
        .await
        .into_db_result()
        .map_err(Into::<Report<StoreError>>::into)?;
    if subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let ids = subscriptions.iter().map(|s| s.0).collect::<Vec<_>>();
    let components = SubscriptionComponentRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = SubscriptionAddOnRow::list_all_by_subscriptions(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let fees = RowFees::load(conn, tenant_id, &components, &add_ons).await?;
    let mut pricing = std::collections::HashSet::new();
    for row in components {
        let subscription_id = row.subscription_id;
        let metric = fees.component_metric(row)?;
        if metric == Some(metric_id) {
            pricing.insert(subscription_id);
        }
    }
    for row in add_ons {
        let subscription_id = row.subscription_id;
        let metric = fees.add_on_metric(row)?;
        if metric == Some(metric_id) {
            pricing.insert(subscription_id);
        }
    }
    Ok(subscriptions
        .into_iter()
        .filter(|(id, _, _)| pricing.contains(id))
        .map(|(id, customer_id, status)| PricingSubscription {
            id,
            customer_id,
            live: !terminated().contains(&status),
        })
        .collect())
}

/// Whether a subscription, not terminated, is for a resource of type `type_id`.
pub(crate) async fn sold_for_a_resource_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    type_id: ResourceTypeId,
) -> StoreResult<bool> {
    use diesel::NullableExpressionMethods;
    use diesel_models::schema::resource::dsl as r;
    use diesel_models::schema::subscription::dsl as s;
    diesel::select(diesel::dsl::exists(
        s::subscription
            .inner_join(r::resource.on(r::id.nullable().eq(s::resource_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(r::resource_type_id.eq(type_id))
            .filter(s::status.ne_all(terminated())),
    ))
    .get_result(conn)
    .await
    .into_db_result()
    .map_err(Into::<Report<StoreError>>::into)
}

/// Refuses changing the keys of type `type_id` while a subscription for one of its resources is
/// live, or has an invoice, draft or finalized, while a metric is routed by those keys (R-144).
pub(crate) async fn refuse_key_change(
    conn: &mut PgConn,
    tenant_id: TenantId,
    type_id: ResourceTypeId,
) -> StoreResult<()> {
    #[derive(diesel::QueryableByName)]
    struct Held {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        draft: bool,
        #[diesel(sql_type = diesel::sql_types::Bool)]
        billed: bool,
    }
    if sold_for_a_resource_of(conn, tenant_id, type_id).await? {
        bail!(StoreError::InvalidArgument(
            "a subscription for one of its resources is live: its keys can't change, as events \
             already billed would move"
                .to_owned()
        ));
    }
    let held: Held = diesel::sql_query(format!(
        "SELECT EXISTS (SELECT 1 FROM subscription s
                        JOIN resource r ON r.id = s.resource_id
                        JOIN invoice i ON i.subscription_id = s.id
                        LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
                        WHERE s.tenant_id = $1 AND r.resource_type_id = $2
                          AND s.status <> 'ABORTED'
                          AND COALESCE(parent.status, i.status) = 'DRAFT') AS draft,
                EXISTS (SELECT 1 FROM invoice i
                        LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
                        WHERE {FINALIZED_SQL}
                          AND i.tenant_id = $1
                          AND EXISTS (SELECT 1 FROM subscription s
                                      JOIN resource r ON r.id = s.resource_id
                                      WHERE s.id = i.subscription_id AND s.status <> 'ABORTED'
                                        AND r.resource_type_id = $2)
                          AND EXISTS (SELECT 1 FROM billable_metric m
                                      WHERE m.tenant_id = $1
                                        AND m.attributed_to_type_id = $2
                                        AND m.attribution_property IS NULL)) AS billed"
    ))
    .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(type_id.as_uuid())
    .get_result(conn)
    .await
    .into_db_result()
    .map_err(Into::<Report<StoreError>>::into)?;
    if held.draft || held.billed {
        bail!(StoreError::InvalidArgument(
            "a subscription for one of its resources has invoiced, or has an invoice in draft, \
             while its keys route a metric: they can't change, as that usage would move"
                .to_owned()
        ));
    }
    Ok(())
}

/// Refuses `subscription_id`'s life running to `end` (none: its end date) when another subscription
/// sells its resource over it: withdrawing or moving its cancellation later must not oversell it.
pub(crate) async fn ensure_still_unsold(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    end: Option<NaiveDateTime>,
) -> StoreResult<()> {
    let row = subscription_row(conn, tenant_id, subscription_id).await?;
    let Some(resource_id) = row.resource_id else {
        return Ok(());
    };
    let end = match (end.map(first_day_from), row.end_date) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    ensure_resource_unsold(
        conn,
        tenant_id,
        resource_id,
        row.start_date,
        end,
        Some(subscription_id),
    )
    .await
}

/// Refuses a subscription for `resource_id` over `[start, end)` while another one, aborted ones
/// aside, sells it over any of it: a live one is upgraded by a plan change instead; one that ended
/// (cancelled, completed) keeps what it claimed up to its end, so the new one starts from then.
pub(crate) async fn ensure_resource_unsold(
    conn: &mut PgConn,
    tenant_id: TenantId,
    resource_id: ResourceId,
    start: NaiveDate,
    end: Option<NaiveDate>,
    except: Option<SubscriptionId>,
) -> StoreResult<()> {
    use diesel::OptionalExtension;
    #[derive(diesel::QueryableByName)]
    struct Selling {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: uuid::Uuid,
        #[diesel(sql_type = diesel::sql_types::Bool)]
        ended: bool,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamp>)]
        until: Option<NaiveDateTime>,
    }
    // An ended one sold it up to its cancellation, termination or end date, else (ended with none
    // of them) its last period's start, as its claim reads it (`ended_at`).
    let selling: Option<Selling> = diesel::sql_query(
        "SELECT o.id, o.ended, o.until FROM (
             SELECT o.id, o.start_date, o.status IN ('CANCELLED', 'COMPLETED') AS ended,
                    CASE WHEN o.status IN ('CANCELLED', 'COMPLETED')
                         THEN COALESCE(LEAST(o.end_date::timestamp, c.at, o.terminated_at),
                                       o.current_period_start::timestamp)
                         ELSE LEAST(o.end_date::timestamp, c.at) END AS until
             FROM subscription o
             LEFT JOIN subscription_ending c ON c.subscription_id = o.id AND c.kind = 'CANCEL'
             WHERE o.tenant_id = $1 AND o.resource_id = $2 AND o.id IS DISTINCT FROM $3
               AND o.status <> 'ABORTED'
         ) o
         WHERE ($5::date IS NULL OR o.start_date < $5)
           AND $4::timestamp < COALESCE(o.until, 'infinity')
           AND o.start_date::timestamp < COALESCE(o.until, 'infinity')
         ORDER BY o.start_date
         LIMIT 1",
    )
    .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(resource_id.as_uuid())
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Uuid>, _>(except.map(|id| id.as_uuid()))
    .bind::<diesel::sql_types::Timestamp, _>(midnight(start))
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Date>, _>(end)
    .get_result(conn)
    .await
    .optional()
    .into_db_result()
    .map_err(Into::<Report<StoreError>>::into)?;
    let Some(selling) = selling else {
        return Ok(());
    };
    let by = SubscriptionId::from(selling.id);
    let message = match selling.until.filter(|_| selling.ended) {
        Some(until) => format!(
            "resource {resource_id} was sold by subscription {by} until {}: a subscription for \
             it can start from then",
            first_day_from(until).format("%b %-d, %Y")
        ),
        None => format!(
            "resource {resource_id} is already sold by subscription {by} over this period: \
             change its plan instead"
        ),
    };
    Err(resource_at_fault(Report::new(StoreError::InvalidArgument(
        message,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_names_the_first_day_allowed() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        assert_eq!(first_day_from(midnight(day)), day);
        assert_eq!(
            first_day_from(day.and_hms_opt(9, 30, 0).unwrap()),
            day + chrono::Days::new(1)
        );
    }
}

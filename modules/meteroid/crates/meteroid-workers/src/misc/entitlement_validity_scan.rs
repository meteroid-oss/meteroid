//! The validity scan (ADR 0011 §3.10): a row reaching `valid_from` or `valid_until` changes a value
//! without a write, and monitors hold their compiled limit. The customers such a row reaches are
//! marked for the credit recompute, which the EE module recompiles and OSS drains.
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Timestamptz, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use distributed_lock::LeaderElection;
use error_stack::Report;
use meteroid_app::config::ParsedCron;
use meteroid_store::errors::StoreError;
use meteroid_store::{Store, StoreResult};
use scoped_futures::ScopedFutureExt;
use std::sync::Arc;

const BATCH: i64 = 5_000;

/// Rows crossing a bound in `($1, $2]`, and every customer they reach: their own (customer,
/// subscription, resource), or each live one of a plan (a trial's trial plan included), plan
/// version, add-on or feature default.
const MARK_CROSSED_TEMPLATE: &str = "
WITH crossed AS (
    SELECT tenant_id, entity_type, entity_id, resource_id
    FROM entitlement
    WHERE revoked_at IS NULL
      AND ((valid_from > $1 AND valid_from <= $2) OR (valid_until > $1 AND valid_until <= $2))
), live AS (
    SELECT id, tenant_id, customer_id, plan_version_id FROM subscription
    WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE')
    UNION ALL
    SELECT subscription_id, tenant_id, customer_id, plan_version_id FROM (TRIALING_PLAN_TIERS) t
), reached AS (
    SELECT c.tenant_id, c.entity_id AS customer_id FROM crossed c WHERE c.entity_type = 'CUSTOMER'
    UNION
    SELECT r.tenant_id, r.customer_id FROM crossed c JOIN resource r ON r.id = c.resource_id
    UNION
    SELECT s.tenant_id, s.customer_id FROM crossed c JOIN subscription s ON s.id = c.entity_id
    WHERE c.entity_type = 'SUBSCRIPTION'
    UNION
    SELECT s.tenant_id, s.customer_id FROM crossed c JOIN live s ON s.plan_version_id = c.entity_id
    WHERE c.entity_type = 'PLAN_VERSION'
    UNION
    SELECT s.tenant_id, s.customer_id FROM crossed c
    JOIN plan_version pv ON pv.plan_id = c.entity_id JOIN live s ON s.plan_version_id = pv.id
    WHERE c.entity_type = 'PLAN'
    UNION
    SELECT s.tenant_id, s.customer_id FROM crossed c
    JOIN subscription_add_on sa ON sa.add_on_id = c.entity_id
        AND (sa.effective_to IS NULL OR sa.effective_to > $1::date)
    JOIN live s ON s.id = sa.subscription_id
    WHERE c.entity_type = 'ADD_ON'
    UNION
    SELECT cu.tenant_id, cu.id FROM crossed c
    JOIN customer cu ON cu.tenant_id = c.tenant_id AND cu.archived_at IS NULL
    WHERE c.entity_type = 'FEATURE'
)
INSERT INTO credit_recompute_mark (tenant_id, customer_id)
SELECT tenant_id, customer_id FROM reached
WHERE $3::uuid IS NULL OR (tenant_id, customer_id) > ($3, $4)
ORDER BY tenant_id, customer_id
LIMIT $5
ON CONFLICT (tenant_id, customer_id) DO UPDATE SET marked_at = clock_timestamp()
RETURNING tenant_id, customer_id";

static MARK_CROSSED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    MARK_CROSSED_TEMPLATE.replace(
        "TRIALING_PLAN_TIERS",
        diesel_models::query::trial_plans::TRIALING_PLAN_TIERS,
    )
});

#[derive(QueryableByName)]
struct Marked {
    #[diesel(sql_type = SqlUuid)]
    tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    customer_id: CustomerId,
}

#[derive(QueryableByName)]
struct Cursor {
    #[diesel(sql_type = Timestamptz)]
    checked_until: DateTime<Utc>,
}

pub async fn run_entitlement_validity_scan_worker(
    store: Arc<Store>,
    elector: Arc<dyn LeaderElection>,
    schedule: ParsedCron,
) {
    super::run_scheduled("entitlement validity scan", schedule, elector, move || {
        let store = store.clone();
        async move {
            let now = common_clock::now();
            let marked = scan(&store, now).await?;
            if marked > 0 {
                log::info!("entitlement validity scan: {marked} customer(s) marked for recompute");
            }
            // The app-count op log rides the same tick: its retention is in days.
            let purged = store.purge_app_count_ops(now).await?;
            if purged > 0 {
                log::info!(
                    "entitlement validity scan: {purged} app-count op(s) past retention purged"
                );
            }
            Ok(())
        }
    })
    .await;
}

/// Marks the customers whose rows cross a bound in `(cursor, now]`, then moves the cursor to
/// `now`, in one transaction: an interrupted scan leaves the cursor where it was.
pub async fn scan(store: &Store, now: DateTime<Utc>) -> StoreResult<usize> {
    store
        .transaction(|conn| {
            async move {
                let from = diesel::sql_query(
                    "SELECT checked_until FROM entitlement_schedule_cursor FOR UPDATE",
                )
                .get_result::<Cursor>(conn)
                .await
                .map_err(db)?
                .checked_until;
                if from >= now {
                    return Ok(0);
                }
                let mut marked = 0;
                let mut after: Option<(TenantId, CustomerId)> = None;
                loop {
                    let batch: Vec<Marked> = diesel::sql_query(MARK_CROSSED.as_str())
                        .bind::<Timestamptz, _>(from)
                        .bind::<Timestamptz, _>(now)
                        .bind::<Nullable<SqlUuid>, _>(after.map(|(t, _)| t))
                        .bind::<Nullable<SqlUuid>, _>(after.map(|(_, c)| c))
                        .bind::<BigInt, _>(BATCH)
                        .load(conn)
                        .await
                        .map_err(db)?;
                    marked += batch.len();
                    let last = batch.iter().map(|m| (m.tenant_id, m.customer_id)).max();
                    match last {
                        Some(last) if batch.len() as i64 == BATCH => after = Some(last),
                        _ => break,
                    }
                }
                diesel::sql_query("UPDATE entitlement_schedule_cursor SET checked_until = $1")
                    .bind::<Timestamptz, _>(now)
                    .execute(conn)
                    .await
                    .map_err(db)?;
                Ok(marked)
            }
            .scope_boxed()
        })
        .await
}

fn db(e: diesel::result::Error) -> Report<StoreError> {
    Report::new(StoreError::from(e))
}

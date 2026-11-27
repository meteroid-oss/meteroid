//! The one reader of a trial's plan (ADR 0011 §3.9): a subscription in an active trial resolves
//! the active version of the plan its trial fact records, at the plan tiers.
use common_domain::ids::{PlanVersionId, SubscriptionId, TenantId};
use diesel::sql_types::{Array, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

/// `subscription_id, tenant_id, customer_id, plan_version_id` of every subscription in an active
/// trial giving another plan: that plan's active version. Spliced into SQL as a subquery.
pub const TRIALING_PLAN_TIERS: &str = "SELECT s.id AS subscription_id, s.tenant_id, s.customer_id, \
     tp.active_version_id AS plan_version_id \
     FROM subscription s \
     JOIN subscription_trial t ON t.subscription_id = s.id \
     JOIN plan tp ON tp.id = t.trial_plan_id \
     WHERE s.status = 'TRIAL_ACTIVE' AND tp.active_version_id IS NOT NULL";

#[derive(diesel::QueryableByName)]
struct TrialPlanTier {
    #[diesel(sql_type = SqlUuid)]
    subscription_id: SubscriptionId,
    #[diesel(sql_type = SqlUuid)]
    plan_version_id: PlanVersionId,
}

/// The trial plan's active version of each of `subscriptions` in an active trial giving one.
pub async fn trial_plan_versions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscriptions: &[SubscriptionId],
) -> DbResult<std::collections::HashMap<SubscriptionId, PlanVersionId>> {
    if subscriptions.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let rows: Vec<TrialPlanTier> = diesel::sql_query(format!(
        "SELECT subscription_id, plan_version_id FROM ({TRIALING_PLAN_TIERS}) tiers \
         WHERE tiers.tenant_id = $1 AND tiers.subscription_id = ANY($2)"
    ))
    .bind::<SqlUuid, _>(tenant_id)
    .bind::<Array<SqlUuid>, _>(subscriptions)
    .load(conn)
    .await
    .attach("Error while reading trials' plans")
    .into_db_result()?;
    Ok(rows
        .into_iter()
        .map(|row| (row.subscription_id, row.plan_version_id))
        .collect())
}

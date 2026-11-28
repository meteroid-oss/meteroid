use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel::prelude::*;
use diesel::sql_types::{Array, Nullable, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::renewals::{RenewalCandidateRow, RenewalNoticeRow, RenewalSettingsRow};
use crate::{DbResult, PgConn};

const LIST_CANDIDATES: &str = "
SELECT s.tenant_id, tn.organization_id, s.id AS subscription_id, s.customer_id,
       c.name AS customer_name, p.name AS plan_name, s.currency, s.mrr_cents, s.billing_day_anchor,
       s.period::text AS period, s.current_period_end, s.end_date, cancel.at AS cancel_date,
       t.term_end, t.auto_renew AS term_auto_renew, t.renewal_months AS term_renewal_months,
       t.uplift_percent AS term_uplift_percent, t.notice_days AS term_notice_days,
       t.renewal_phase_id AS term_renewal_phase_id,
       COALESCE(NOT t.auto_renew AND tr.until_date = t.term_end
                AND (tr.end_action = 'CANCEL' OR s.status = 'TRIAL_ACTIVE'), false)
           AS term_written_by_trial
FROM subscription s
JOIN tenant tn ON tn.id = s.tenant_id
JOIN customer c ON c.id = s.customer_id
JOIN plan_version pv ON pv.id = s.plan_version_id
JOIN plan p ON p.id = pv.plan_id
LEFT JOIN subscription_term t ON t.subscription_id = s.id
LEFT JOIN subscription_trial tr ON tr.subscription_id = s.id
LEFT JOIN LATERAL (
    SELECT MIN(e.at)::date AS at FROM subscription_ending e
    WHERE e.subscription_id = s.id AND e.kind = 'CANCEL'
) cancel ON true
WHERE s.status IN ('ACTIVE', 'TRIAL_ACTIVE')
  AND ($1::uuid[] IS NULL OR s.tenant_id = ANY($1))
  AND ($2::uuid IS NULL OR s.id = $2)
  AND (t.subscription_id IS NOT NULL OR s.period IN ('ANNUAL', 'SEMIANNUAL')
       OR s.end_date IS NOT NULL OR cancel.at IS NOT NULL)
ORDER BY s.tenant_id, s.id";

/// The live subscriptions of `tenants` (every tenant when `None`), or the one `subscription`, that a
/// term, a long period, an end date or a cancellation can renew or end.
pub async fn list_renewal_candidates(
    conn: &mut PgConn,
    tenants: Option<&[TenantId]>,
    subscription: Option<SubscriptionId>,
) -> DbResult<Vec<RenewalCandidateRow>> {
    let tenants: Option<Vec<uuid::Uuid>> =
        tenants.map(|ids| ids.iter().map(|id| id.as_uuid()).collect());
    diesel::sql_query(LIST_CANDIDATES)
        .bind::<Nullable<Array<SqlUuid>>, _>(tenants)
        .bind::<Nullable<SqlUuid>, _>(subscription.map(|id| id.as_uuid()))
        .load(conn)
        .await
        .attach("Error while listing renewal candidates")
        .into_db_result()
}

/// Records a notice; false when it was recorded already, by this pass or another.
pub async fn claim_notice(conn: &mut PgConn, notice: &RenewalNoticeRow) -> DbResult<bool> {
    use crate::schema::subscription_renewal_notice::dsl as n;
    diesel::insert_into(n::subscription_renewal_notice)
        .values(notice)
        .on_conflict_do_nothing()
        .execute(conn)
        .await
        .map(|inserted| inserted > 0)
        .attach("Error while claiming a renewal notice")
        .into_db_result()
}

/// The notices sent for a subscription, latest first.
pub async fn list_notices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> DbResult<Vec<RenewalNoticeRow>> {
    use crate::schema::subscription_renewal_notice::dsl as n;
    n::subscription_renewal_notice
        .filter(n::tenant_id.eq(tenant_id))
        .filter(n::subscription_id.eq(subscription_id))
        .order((n::notified_at.desc(), n::days_before.asc()))
        .select(RenewalNoticeRow::as_select())
        .load(conn)
        .await
        .attach("Error while listing a subscription's renewal notices")
        .into_db_result()
}

pub async fn get_settings(
    conn: &mut PgConn,
    tenant_id: TenantId,
) -> DbResult<Option<RenewalSettingsRow>> {
    use crate::schema::renewal_settings::dsl as s;
    s::renewal_settings
        .filter(s::tenant_id.eq(tenant_id))
        .select(RenewalSettingsRow::as_select())
        .first(conn)
        .await
        .optional()
        .attach("Error while reading the renewal settings")
        .into_db_result()
}

pub async fn upsert_settings(
    conn: &mut PgConn,
    row: &RenewalSettingsRow,
) -> DbResult<RenewalSettingsRow> {
    use crate::schema::renewal_settings::dsl as s;
    diesel::insert_into(s::renewal_settings)
        .values(row)
        .on_conflict(s::tenant_id)
        .do_update()
        .set((
            s::checkpoints.eq(&row.checkpoints),
            s::customer_reminder_days.eq(row.customer_reminder_days),
            s::updated_at.eq(row.updated_at),
        ))
        .returning(RenewalSettingsRow::as_returning())
        .get_result(conn)
        .await
        .attach("Error while saving the renewal settings")
        .into_db_result()
}

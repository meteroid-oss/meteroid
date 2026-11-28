//! Claims of the notices published once each: an override's end per lineage and kind (R-148), a
//! trial's end (R-158). A claim inserts its key and returns the rows it won, so a pass replayed or
//! run twice publishes nothing more.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{CustomerId, FeatureId, ResourceId, SubscriptionId, TenantId};
use diesel::QueryableByName;
use diesel::sql_types::{Bool, Date, Integer, Jsonb, Nullable, Text, Timestamptz, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

/// A customer or resource override whose end is claimed.
#[derive(Debug, Clone, QueryableByName)]
pub struct EndingOverrideRow {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = SqlUuid)]
    pub feature_id: FeatureId,
    #[diesel(sql_type = Text)]
    pub feature_code: String,
    #[diesel(sql_type = SqlUuid)]
    pub lineage_id: Uuid,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub resource_id: Option<ResourceId>,
    #[diesel(sql_type = Jsonb)]
    pub value: serde_json::Value,
    #[diesel(sql_type = Text)]
    pub effect: String,
    #[diesel(sql_type = Timestamptz)]
    pub valid_until: DateTime<Utc>,
    #[diesel(sql_type = Nullable<Text>)]
    pub reason: Option<String>,
}

/// The open row of each customer-level lineage with an end in `($2, $3]`, not yet noticed for
/// that end and kind: an edit opens a row with a new end; a revoked lineage has no open row.
const CLAIM_ENDING_OVERRIDES: &str = "
WITH due AS (
    SELECT e.tenant_id, e.entity_id AS customer_id, e.feature_id, f.code AS feature_code,
           e.lineage_id, e.resource_id, e.value, e.effect::text AS effect, e.valid_until, e.reason
    FROM entitlement e
    JOIN feature f ON f.id = e.feature_id
    WHERE e.entity_type = 'CUSTOMER'
      AND e.revoked_at IS NULL
      AND e.valid_until > $2 AND e.valid_until <= $3
), claimed AS (
    INSERT INTO entitlement_override_notice (lineage_id, valid_until, ended, tenant_id, noticed_at)
    SELECT lineage_id, valid_until, $4, tenant_id, $1 FROM due
    ON CONFLICT DO NOTHING
    RETURNING lineage_id, valid_until
)
SELECT due.* FROM due
JOIN claimed ON claimed.lineage_id = due.lineage_id AND claimed.valid_until = due.valid_until";

/// Overrides ending within `lead` of `now` (`ended` false), or that ended within `lead` before it
/// (`ended` true, so a long-past end is never noticed late), claimed for their notice.
pub async fn claim_ending_overrides(
    conn: &mut PgConn,
    now: DateTime<Utc>,
    lead: chrono::Duration,
    ended: bool,
) -> DbResult<Vec<EndingOverrideRow>> {
    let (from, to) = if ended {
        (now - lead, now)
    } else {
        (now, now + lead)
    };
    diesel::sql_query(CLAIM_ENDING_OVERRIDES)
        .bind::<Timestamptz, _>(now)
        .bind::<Timestamptz, _>(from)
        .bind::<Timestamptz, _>(to)
        .bind::<Bool, _>(ended)
        .load(conn)
        .await
        .attach("Error while claiming override end notices")
        .into_db_result()
}

/// A trialing subscription whose trial end is claimed.
#[derive(Debug, Clone, QueryableByName)]
pub struct EndingTrialRow {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = SqlUuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = Date)]
    pub until_date: NaiveDate,
    #[diesel(sql_type = Bool)]
    pub free: bool,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub trial_offer_id: Option<Uuid>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub end_action: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub without_card: String,
    #[diesel(sql_type = Bool)]
    pub charge_automatically: bool,
}

const CLAIM_ENDING_TRIALS: &str = "
WITH due AS (
    SELECT t.tenant_id, s.customer_id, t.subscription_id, t.until_date, t.free, t.trial_offer_id,
           t.end_action, t.without_card, s.charge_automatically
    FROM subscription_trial t
    JOIN subscription s ON s.id = t.subscription_id
    WHERE s.status = 'TRIAL_ACTIVE' AND t.outcome IS NULL
      AND t.until_date > $2 AND t.until_date <= $2 + $3
), claimed AS (
    INSERT INTO subscription_trial_notice (subscription_id, trial_end, tenant_id, noticed_at)
    SELECT subscription_id, until_date, tenant_id, $1 FROM due
    ON CONFLICT DO NOTHING
    RETURNING subscription_id, trial_end
)
SELECT due.* FROM due
JOIN claimed ON claimed.subscription_id = due.subscription_id AND claimed.trial_end = due.until_date";

/// Active trials ending within `lead_days` after `today`, claimed for their notice: a trial whose
/// end moves is claimed again for its new end.
pub async fn claim_ending_trials(
    conn: &mut PgConn,
    now: DateTime<Utc>,
    today: NaiveDate,
    lead_days: i32,
) -> DbResult<Vec<EndingTrialRow>> {
    diesel::sql_query(CLAIM_ENDING_TRIALS)
        .bind::<Timestamptz, _>(now)
        .bind::<Date, _>(today)
        .bind::<Integer, _>(lead_days)
        .load(conn)
        .await
        .attach("Error while claiming trial end notices")
        .into_db_result()
}

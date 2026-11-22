//! Changes scheduled for a later date, written as facts when requested.

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::enums::SubscriptionChangeKindEnum;
use crate::errors::IntoDbResult;
use crate::subscription_changes::{SubscriptionChangeRow, SubscriptionChangeRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{ScheduledEventId, SubscriptionId, TenantId};

/// A change the derived state has not applied yet: no Switch row names it.
const NOT_APPLIED_SQL: &str = "NOT EXISTS (SELECT 1 FROM subscription_event ev \
     WHERE ev.subscription_id = subscription_change.subscription_id AND ev.event_type = 'SWITCH' \
       AND ev.details ->> 'event' = subscription_change.id::text)";

fn not_applied() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    diesel::dsl::sql::<diesel::sql_types::Bool>(NOT_APPLIED_SQL)
}

impl SubscriptionChangeRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionChangeRow> {
        use crate::schema::subscription_change::dsl::subscription_change;

        let query = diesel::insert_into(subscription_change).values(self);

        crate::debug_sql(&query);

        query
            .returning(SubscriptionChangeRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while recording a scheduled change")
            .into_db_result()
    }
}

impl SubscriptionChangeRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        id_param: ScheduledEventId,
        tenant_id_param: &TenantId,
    ) -> DbResult<SubscriptionChangeRow> {
        use crate::schema::subscription_change::dsl::{id, subscription_change, tenant_id};

        subscription_change
            .filter(id.eq(id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .select(SubscriptionChangeRow::as_select())
            .get_result(conn)
            .await
            .attach("Error while fetching a scheduled change")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        id_param: ScheduledEventId,
        tenant_id_param: &TenantId,
    ) -> DbResult<Option<SubscriptionChangeRow>> {
        use crate::schema::subscription_change::dsl::{id, subscription_change, tenant_id};

        subscription_change
            .filter(id.eq(id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .select(SubscriptionChangeRow::as_select())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while looking for a scheduled change")
            .into_db_result()
    }

    /// The subscription's changes of `kinds` still to take effect, earliest first. Read under the
    /// customer's lock, as every write of them.
    pub async fn list_pending(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        kinds: &[SubscriptionChangeKindEnum],
    ) -> DbResult<Vec<SubscriptionChangeRow>> {
        use crate::schema::subscription_change::dsl::{
            effective_at, kind, subscription_change, subscription_id, tenant_id, withdrawn_at,
        };

        let query = subscription_change
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(withdrawn_at.is_null())
            .filter(not_applied())
            .filter(kind.eq_any(kinds.to_vec()))
            .order_by(effective_at.asc())
            .select(SubscriptionChangeRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the scheduled changes still to apply")
            .into_db_result()
    }

    /// Every change of the subscription still to take effect, as its details show them.
    pub async fn list_pending_user_visible(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<SubscriptionChangeRow>> {
        use crate::schema::subscription_change::dsl::{
            effective_at, subscription_change, subscription_id, tenant_id, withdrawn_at,
        };

        let query = subscription_change
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(withdrawn_at.is_null())
            .filter(not_applied())
            .order_by(effective_at.asc())
            .select(SubscriptionChangeRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the scheduled changes of a subscription")
            .into_db_result()
    }

    /// The changes of `kind` that took effect by `at`, earliest first.
    pub async fn list_in_force(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        kind_param: SubscriptionChangeKindEnum,
        at: NaiveDateTime,
    ) -> DbResult<Vec<SubscriptionChangeRow>> {
        use crate::schema::subscription_change::dsl::{
            effective_at, kind, subscription_change, subscription_id, tenant_id, withdrawn_at,
        };

        let query = subscription_change
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(withdrawn_at.is_null())
            .filter(effective_at.le(at))
            .filter(kind.eq(kind_param))
            .order_by(effective_at.asc())
            .select(SubscriptionChangeRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the scheduled changes in force")
            .into_db_result()
    }

    /// Moves a change still to take effect to `effective_at_param`, with the rows it now writes;
    /// returns whether it had not taken effect, nor was withdrawn.
    pub async fn defer(
        conn: &mut PgConn,
        id_param: &ScheduledEventId,
        effective_at_param: NaiveDateTime,
        written_facts_param: serde_json::Value,
    ) -> DbResult<bool> {
        use crate::schema::subscription_change::dsl::{
            effective_at, id, subscription_change, updated_at, withdrawn_at, written_facts,
        };

        let query = diesel::update(subscription_change)
            .filter(id.eq(id_param))
            .filter(withdrawn_at.is_null())
            .filter(not_applied())
            .set((
                effective_at.eq(effective_at_param),
                written_facts.eq(Some(written_facts_param)),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deferring a scheduled change")
            .into_db_result()
            .map(|updated| updated > 0)
    }

    /// Withdraws a change before it took effect; returns whether it had not, nor was withdrawn.
    pub async fn withdraw(
        conn: &mut PgConn,
        id_param: &ScheduledEventId,
        reason: &str,
    ) -> DbResult<bool> {
        use crate::schema::subscription_change::dsl::{
            id, subscription_change, updated_at, withdrawn_at, withdrawn_reason,
        };

        let now = common_clock::now().naive_utc();
        let query = diesel::update(subscription_change)
            .filter(id.eq(id_param))
            .filter(withdrawn_at.is_null())
            .filter(not_applied())
            .set((
                withdrawn_at.eq(Some(now)),
                withdrawn_reason.eq(Some(reason.to_string())),
                updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while withdrawing a scheduled change")
            .map(|updated| updated > 0)
            .into_db_result()
    }
}

impl SubscriptionChangeRow {
    /// Marks the change of `kind` dated `on`, its facts written, as left for the billing
    /// controller to settle, when `on` falls inside the subscription's stored period.
    pub async fn mark_settlement_due_inside_period(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: chrono::NaiveDate,
        kind: SubscriptionChangeKindEnum,
    ) -> DbResult<()> {
        diesel::sql_query(
            "INSERT INTO subscription_settlement_due (subscription_id, effective_on, tenant_id, kind)
             SELECT s.id, $2, s.tenant_id, $3 FROM subscription s
             WHERE s.id = $1 AND s.current_period_start < $2
               AND (s.current_period_end IS NULL OR s.current_period_end > $2)
             ON CONFLICT (subscription_id, effective_on) DO NOTHING",
        )
        .bind::<diesel::sql_types::Uuid, _>(subscription_id)
        .bind::<diesel::sql_types::Date, _>(on)
        .bind::<crate::schema::sql_types::SubscriptionChangeKindEnum, _>(kind)
        .execute(conn)
        .await
        .attach("Error while marking a change's settlement due")
        .into_db_result()
        .map(|_| ())
    }

    /// Drops the settlement left to the controller for a scheduled amendment dated `on`, which no
    /// longer takes effect that day; an immediate amendment's of that day stays.
    pub async fn drop_amendment_settlement_due(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: chrono::NaiveDate,
    ) -> DbResult<()> {
        diesel::sql_query(
            "DELETE FROM subscription_settlement_due d
             WHERE d.subscription_id = $1 AND d.effective_on = $2 AND d.kind = 'AMENDMENT'
               AND NOT EXISTS (SELECT 1 FROM subscription_event ev
                               WHERE ev.subscription_id = $1 AND ev.event_type = 'SWITCH'
                                 AND ev.applies_to = $2 AND ev.details ->> 'mode' = 'immediate')",
        )
        .bind::<diesel::sql_types::Uuid, _>(subscription_id)
        .bind::<diesel::sql_types::Date, _>(on)
        .execute(conn)
        .await
        .attach("Error while dropping an amendment's settlement due")
        .into_db_result()
        .map(|_| ())
    }

    /// The days of the subscription's changes left for the controller to settle.
    pub async fn settlements_due(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<chrono::NaiveDate>> {
        use crate::schema::subscription_settlement_due::dsl as d;
        d::subscription_settlement_due
            .filter(d::subscription_id.eq(subscription_id))
            .select(d::effective_on)
            .load(conn)
            .await
            .attach("Error while listing the settlements due")
            .into_db_result()
    }

    /// The kind of the change dated `on` left for the controller to settle, taken: None when no
    /// change was left so.
    pub async fn take_settlement_due(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: chrono::NaiveDate,
    ) -> DbResult<Option<SubscriptionChangeKindEnum>> {
        use crate::schema::subscription_settlement_due::dsl as d;
        diesel::delete(
            d::subscription_settlement_due
                .filter(d::subscription_id.eq(subscription_id))
                .filter(d::effective_on.eq(on)),
        )
        .returning(d::kind)
        .get_result(conn)
        .await
        .optional()
        .attach("Error while taking a change's settlement due")
        .into_db_result()
    }
}

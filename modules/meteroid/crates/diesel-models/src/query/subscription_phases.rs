//! A subscription's phases, live (not withdrawn) in `seq` order.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::subscription_phases::{SubscriptionPhaseRow, SubscriptionTimelineItemRow};
use crate::{DbResult, PgConn};
use common_domain::ids::{SubscriptionId, SubscriptionPhaseId, TenantId};

impl SubscriptionPhaseRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionPhaseRow> {
        use crate::schema::subscription_phase::dsl::subscription_phase;

        let query = diesel::insert_into(subscription_phase).values(self);

        crate::debug_sql(&query);

        query
            .returning(SubscriptionPhaseRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while recording a subscription phase")
            .into_db_result()
    }

    pub async fn list_live(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<Vec<SubscriptionPhaseRow>> {
        use crate::schema::subscription_phase::dsl::{
            seq, subscription_id, subscription_phase, tenant_id, withdrawn_at,
        };

        let query = subscription_phase
            .filter(tenant_id.eq(tenant_id_param))
            .filter(subscription_id.eq(subscription_id_param))
            .filter(withdrawn_at.is_null())
            .order(seq.asc())
            .select(SubscriptionPhaseRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription phases")
            .into_db_result()
    }

    /// Every live phase of the tenant, for the invariant comparing phases with their changes.
    pub async fn list_live_by_tenant(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<SubscriptionPhaseRow>> {
        use crate::schema::subscription_phase::dsl::{
            seq, subscription_id, subscription_phase, tenant_id, withdrawn_at,
        };

        let query = subscription_phase
            .filter(tenant_id.eq(tenant_id_param))
            .filter(withdrawn_at.is_null())
            .order((subscription_id.asc(), seq.asc()))
            .select(SubscriptionPhaseRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription phases")
            .into_db_result()
    }

    pub async fn withdraw(conn: &mut PgConn, id_param: SubscriptionPhaseId) -> DbResult<bool> {
        use crate::schema::subscription_phase::dsl::{id, subscription_phase, withdrawn_at};

        let query = diesel::update(subscription_phase)
            .filter(id.eq(id_param))
            .filter(withdrawn_at.is_null())
            .set(withdrawn_at.eq(Some(common_clock::now().naive_utc())));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while withdrawing a subscription phase")
            .map(|updated| updated > 0)
            .into_db_result()
    }

    /// The phase's boundary written again (a rewrite of the timeline, R-162), and its terms.
    pub async fn rewritten(
        conn: &mut PgConn,
        id_param: SubscriptionPhaseId,
        change_id_param: common_domain::ids::ScheduledEventId,
        terms_param: serde_json::Value,
    ) -> DbResult<()> {
        use crate::schema::subscription_phase::dsl::{change_id, id, subscription_phase, terms};

        let query = diesel::update(subscription_phase)
            .filter(id.eq(id_param))
            .set((change_id.eq(change_id_param), terms.eq(terms_param)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while rewriting a subscription phase")
            .map(|_| ())
            .into_db_result()
    }
}

impl SubscriptionTimelineItemRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionTimelineItemRow> {
        use crate::schema::subscription_timeline_item::dsl::subscription_timeline_item;

        let query = diesel::insert_into(subscription_timeline_item).values(self);

        crate::debug_sql(&query);

        query
            .returning(SubscriptionTimelineItemRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while recording a timeline item")
            .into_db_result()
    }

    pub async fn list_live(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<Vec<SubscriptionTimelineItemRow>> {
        use crate::schema::subscription_timeline_item::dsl::{
            start_date, subscription_id, subscription_timeline_item, tenant_id, withdrawn_at,
        };

        let query = subscription_timeline_item
            .filter(tenant_id.eq(tenant_id_param))
            .filter(subscription_id.eq(subscription_id_param))
            .filter(withdrawn_at.is_null())
            .order(start_date.asc())
            .select(SubscriptionTimelineItemRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing timeline items")
            .into_db_result()
    }

    pub async fn withdraw(conn: &mut PgConn, id_param: uuid::Uuid) -> DbResult<bool> {
        use crate::schema::subscription_timeline_item::dsl::{
            id, subscription_timeline_item, withdrawn_at,
        };

        let query = diesel::update(subscription_timeline_item)
            .filter(id.eq(id_param))
            .filter(withdrawn_at.is_null())
            .set(withdrawn_at.eq(Some(common_clock::now().naive_utc())));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while withdrawing a timeline item")
            .map(|updated| updated > 0)
            .into_db_result()
    }

    /// The item's end moved to `until`, written by the change `end_change_id_param`.
    pub async fn move_end(
        conn: &mut PgConn,
        id_param: uuid::Uuid,
        until: chrono::NaiveDate,
        end_change_id_param: common_domain::ids::ScheduledEventId,
    ) -> DbResult<()> {
        use crate::schema::subscription_timeline_item::dsl::{
            end_change_id, id, subscription_timeline_item, until_date,
        };

        let query = diesel::update(subscription_timeline_item)
            .filter(id.eq(id_param))
            .set((until_date.eq(until), end_change_id.eq(end_change_id_param)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while moving a timeline item's end")
            .map(|_| ())
            .into_db_result()
    }

    /// The item's boundaries written again (a rewrite of the timeline, R-162).
    pub async fn rewritten(
        conn: &mut PgConn,
        id_param: uuid::Uuid,
        start_change_id_param: Option<common_domain::ids::ScheduledEventId>,
        end_change_id_param: common_domain::ids::ScheduledEventId,
    ) -> DbResult<()> {
        use crate::schema::subscription_timeline_item::dsl::{
            end_change_id, id, start_change_id, subscription_timeline_item,
        };

        let query = diesel::update(subscription_timeline_item)
            .filter(id.eq(id_param))
            .set((
                start_change_id.eq(start_change_id_param),
                end_change_id.eq(end_change_id_param),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while rewriting a timeline item")
            .map(|_| ())
            .into_db_result()
    }
}

impl crate::subscription_phases::SubscriptionTermRow {
    /// The term, replacing the one recorded before.
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<Self> {
        use crate::schema::subscription_term::dsl::{subscription_id, subscription_term};

        let query = diesel::insert_into(subscription_term)
            .values(self)
            .on_conflict(subscription_id)
            .do_update()
            .set(self);

        crate::debug_sql(&query);

        query
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while recording a subscription's term")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::subscription_term::dsl::{
            subscription_id, subscription_term, tenant_id,
        };

        let query = subscription_term
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .select(Self::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a subscription's term")
            .into_db_result()
    }

    /// The term whose renewal is `phase`, when one is.
    pub async fn renewed_by(
        conn: &mut PgConn,
        phase: SubscriptionPhaseId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::subscription_term::dsl::{renewal_phase_id, subscription_term};

        let query = subscription_term
            .filter(renewal_phase_id.eq(phase))
            .select(Self::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading the term a phase renews")
            .into_db_result()
    }
}

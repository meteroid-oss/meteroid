use crate::errors::IntoDbResult;
use crate::rate_cards::{RateCardEntryRow, RateCardRow, RateCardRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{BillableMetricId, RateCardEntryId, RateCardId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl RateCardRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<RateCardRow> {
        use crate::schema::rate_card::dsl as c;

        let query = diesel::insert_into(c::rate_card)
            .values(self)
            .returning(RateCardRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while inserting rate card")
            .into_db_result()
    }
}

impl RateCardRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
    ) -> DbResult<Vec<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let mut query = c::rate_card
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::archived_at.is_null())
            .order(c::code.asc())
            .select(RateCardRow::as_select())
            .into_boxed();
        if let Some(metric_id) = metric_id {
            query = query.filter(c::metric_id.eq(metric_id));
        }
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing rate cards")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: RateCardId,
    ) -> DbResult<Option<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let query = c::rate_card
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::id.eq(id))
            .select(RateCardRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding rate card")
            .into_db_result()
    }

    pub async fn find_by_code(
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
    ) -> DbResult<Option<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let query = c::rate_card
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::code.eq(code))
            .select(RateCardRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding rate card by code")
            .into_db_result()
    }

    /// The card, locked so entry writes against it are serial.
    pub async fn find_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: RateCardId,
    ) -> DbResult<Option<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let query = c::rate_card
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::id.eq(id))
            .for_update()
            .select(RateCardRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking rate card")
            .into_db_result()
    }
}

impl RateCardEntryRow {
    pub async fn list_by_card(
        conn: &mut PgConn,
        tenant_id: TenantId,
        rate_card_id: RateCardId,
    ) -> DbResult<Vec<RateCardEntryRow>> {
        use crate::schema::rate_card_entry::dsl as e;

        let query = e::rate_card_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::rate_card_id.eq(rate_card_id))
            .order((e::key.asc(), e::effective_from.asc()))
            .select(RateCardEntryRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing rate card entries")
            .into_db_result()
    }

    pub async fn insert_batch(conn: &mut PgConn, rows: &[RateCardEntryRow]) -> DbResult<()> {
        use crate::schema::rate_card_entry::dsl as e;

        if rows.is_empty() {
            return Ok(());
        }
        let query = diesel::insert_into(e::rate_card_entry).values(rows);
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while inserting rate card entries")
            .into_db_result()
    }

    /// Removes entries not yet in force: a scheduled change replaced before its day.
    pub async fn delete_scheduled(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[RateCardEntryId],
        today: chrono::NaiveDate,
    ) -> DbResult<usize> {
        use crate::schema::rate_card_entry::dsl as e;

        if ids.is_empty() {
            return Ok(0);
        }
        let query = diesel::delete(e::rate_card_entry)
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::id.eq_any(ids))
            .filter(e::effective_from.gt(today));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while deleting scheduled rate card entries")
            .into_db_result()
    }
}

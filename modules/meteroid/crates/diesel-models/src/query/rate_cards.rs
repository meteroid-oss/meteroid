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
    /// Renames the card and sets what unmatched usage does; a field left `None` stays. None when
    /// the tenant has no such card.
    pub async fn update_terms(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: RateCardId,
        name: Option<String>,
        on_unmatched: Option<crate::enums::RateCardUnmatchedEnum>,
    ) -> DbResult<Option<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let current = c::rate_card
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::id.eq(id))
            .select(RateCardRow::as_select())
            .for_update()
            .first(conn)
            .await
            .optional()
            .attach("Error while locking rate card")
            .into_db_result()?;
        let Some(current) = current else {
            return Ok(None);
        };
        let query = diesel::update(c::rate_card)
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::id.eq(id))
            .set((
                c::name.eq(name.unwrap_or(current.name)),
                c::on_unmatched.eq(on_unmatched.unwrap_or(current.on_unmatched)),
                c::updated_at.eq(common_clock::now()),
            ))
            .returning(RateCardRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating rate card")
            .into_db_result()
    }

    /// Archives the card (`Some`) or restores it (`None`); None when the tenant has no such card.
    pub async fn set_archived(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: RateCardId,
        archived_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> DbResult<Option<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let query = diesel::update(c::rate_card)
            .filter(c::tenant_id.eq(tenant_id))
            .filter(c::id.eq(id))
            .set((
                c::archived_at.eq(archived_at),
                c::updated_at.eq(common_clock::now()),
            ))
            .returning(RateCardRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while archiving rate card")
            .into_db_result()
    }

    /// The tenant's cards after `after` in id order, at most `limit` (API v2 keyset pages).
    pub async fn list_page(
        conn: &mut PgConn,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
        purpose: Option<crate::enums::RateCardPurposeEnum>,
        include_archived: bool,
        after: Option<RateCardId>,
        limit: i64,
    ) -> DbResult<Vec<RateCardRow>> {
        use crate::schema::rate_card::dsl as c;

        let mut query = c::rate_card.filter(c::tenant_id.eq(tenant_id)).into_boxed();
        if let Some(purpose) = purpose {
            query = query.filter(c::purpose.eq(purpose));
        }
        if !include_archived {
            query = query.filter(c::archived_at.is_null());
        }
        if let Some(metric_id) = metric_id {
            query = query.filter(c::metric_id.eq(metric_id));
        }
        if let Some(after) = after {
            query = query.filter(c::id.gt(after));
        }
        let query = query
            .order(c::id.asc())
            .limit(limit)
            .select(RateCardRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while fetching a page of rate cards")
            .into_db_result()
    }

    /// Every purpose when `purpose` is None.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
        purpose: Option<crate::enums::RateCardPurposeEnum>,
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
        if let Some(purpose) = purpose {
            query = query.filter(c::purpose.eq(purpose));
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

//! Queries for the quote-level *adjustment* tables (the stateless floor/cap family).
//! See `query/plan_version_adjustments.rs` for the kind-scoping rationale; today these are
//! minimum-specific accessors over the generic `quote_adjustment` table.

use crate::enums::AdjustmentKindEnum;
use crate::errors::IntoDbResult;
use crate::quote_adjustments::{
    QuoteAdjustmentComponentRow, QuoteAdjustmentComponentRowNew, QuoteAdjustmentRow,
    QuoteAdjustmentRowNew,
};
use crate::{DbResult, PgConn};

use common_domain::ids::QuoteId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteAdjustmentRow {
    pub async fn find_by_quote_id(
        conn: &mut PgConn,
        quote_id_param: QuoteId,
    ) -> DbResult<Option<QuoteAdjustmentRow>> {
        use crate::schema::quote_adjustment::dsl as m_dsl;

        let query = m_dsl::quote_adjustment
            .filter(m_dsl::quote_id.eq(quote_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(QuoteAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching quote_adjustment")
            .into_db_result()
    }

    pub async fn delete_by_quote_id(conn: &mut PgConn, quote_id_param: QuoteId) -> DbResult<()> {
        use crate::schema::quote_adjustment::dsl as m_dsl;

        let query = diesel::delete(m_dsl::quote_adjustment)
            .filter(m_dsl::quote_id.eq(quote_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting quote_adjustment")
            .map(|_| ())
            .into_db_result()
    }
}

impl QuoteAdjustmentRowNew {
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<QuoteAdjustmentRow> {
        use crate::schema::quote_adjustment::dsl as m_dsl;

        let query = diesel::insert_into(m_dsl::quote_adjustment)
            .values(self)
            .on_conflict((m_dsl::quote_id, m_dsl::kind))
            .do_update()
            .set((
                m_dsl::amount_subunit.eq(self.amount_subunit),
                m_dsl::includes_all_components.eq(self.includes_all_components),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting quote_adjustment")
            .into_db_result()
    }
}

impl QuoteAdjustmentComponentRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        quote_id_param: QuoteId,
    ) -> DbResult<Vec<QuoteAdjustmentComponentRow>> {
        use crate::schema::quote_adjustment_component::dsl as c_dsl;

        let query = c_dsl::quote_adjustment_component
            .filter(c_dsl::quote_id.eq(quote_id_param))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(QuoteAdjustmentComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing quote_adjustment_components")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[QuoteAdjustmentComponentRowNew],
    ) -> DbResult<()> {
        use crate::schema::quote_adjustment_component::dsl as c_dsl;

        if batch.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(c_dsl::quote_adjustment_component).values(batch);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting quote_adjustment_components batch")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn delete_by_quote_id(conn: &mut PgConn, quote_id_param: QuoteId) -> DbResult<()> {
        use crate::schema::quote_adjustment_component::dsl as c_dsl;

        let query = diesel::delete(c_dsl::quote_adjustment_component)
            .filter(c_dsl::quote_id.eq(quote_id_param))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting quote_adjustment_components")
            .map(|_| ())
            .into_db_result()
    }
}

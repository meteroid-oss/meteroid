//! Queries for the plan-version *adjustment* tables (the stateless floor/cap family).
//!
//! Today only `AdjustmentKindEnum::Minimum` exists, so the read/delete helpers below scope to
//! that kind: each is a minimum-specific accessor over the generic `plan_version_adjustment`
//! table, and is therefore safe once a second kind (e.g. Maximum) is added — it will not touch
//! rows of another kind. When more kinds are wired, add kind-parameterised variants alongside.

use crate::enums::AdjustmentKindEnum;
use crate::errors::IntoDbResult;
use crate::plan_version_adjustments::{
    PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentComponentRowNew,
    PlanVersionAdjustmentRow, PlanVersionAdjustmentRowNew,
};
use crate::{DbResult, PgConn};

use common_domain::ids::PlanVersionId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl PlanVersionAdjustmentRow {
    pub async fn find_by_plan_version_id(
        conn: &mut PgConn,
        plan_version_id_param: &PlanVersionId,
    ) -> DbResult<Option<PlanVersionAdjustmentRow>> {
        use crate::schema::plan_version_adjustment::dsl as m_dsl;

        let query = m_dsl::plan_version_adjustment
            .filter(m_dsl::plan_version_id.eq(plan_version_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(PlanVersionAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching plan_version_adjustment")
            .into_db_result()
    }

    pub async fn list_by_plan_version_ids(
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
    ) -> DbResult<Vec<PlanVersionAdjustmentRow>> {
        use crate::schema::plan_version_adjustment::dsl as m_dsl;

        if plan_version_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = m_dsl::plan_version_adjustment
            .filter(m_dsl::plan_version_id.eq_any(plan_version_ids))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(PlanVersionAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing plan_version_adjustments by ids")
            .into_db_result()
    }

    pub async fn delete_by_plan_version_id(
        conn: &mut PgConn,
        plan_version_id_param: &PlanVersionId,
    ) -> DbResult<()> {
        use crate::schema::plan_version_adjustment::dsl as m_dsl;

        let query = diesel::delete(m_dsl::plan_version_adjustment)
            .filter(m_dsl::plan_version_id.eq(plan_version_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan_version_adjustment")
            .map(|_| ())
            .into_db_result()
    }
}

impl PlanVersionAdjustmentRowNew {
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<PlanVersionAdjustmentRow> {
        use crate::schema::plan_version_adjustment::dsl as m_dsl;

        let query = diesel::insert_into(m_dsl::plan_version_adjustment)
            .values(self)
            .on_conflict((m_dsl::plan_version_id, m_dsl::kind))
            .do_update()
            .set((
                m_dsl::amount_subunit.eq(self.amount_subunit),
                m_dsl::includes_all_components.eq(self.includes_all_components),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting plan_version_adjustment")
            .into_db_result()
    }
}

impl PlanVersionAdjustmentComponentRow {
    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        plan_version_id_param: &PlanVersionId,
    ) -> DbResult<Vec<PlanVersionAdjustmentComponentRow>> {
        use crate::schema::plan_version_adjustment_component::dsl as c_dsl;

        let query = c_dsl::plan_version_adjustment_component
            .filter(c_dsl::plan_version_id.eq(plan_version_id_param))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(PlanVersionAdjustmentComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing plan_version_adjustment_components")
            .into_db_result()
    }

    pub async fn list_by_plan_version_ids(
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
    ) -> DbResult<Vec<PlanVersionAdjustmentComponentRow>> {
        use crate::schema::plan_version_adjustment_component::dsl as c_dsl;

        if plan_version_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = c_dsl::plan_version_adjustment_component
            .filter(c_dsl::plan_version_id.eq_any(plan_version_ids))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(PlanVersionAdjustmentComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing plan_version_adjustment_components by ids")
            .into_db_result()
    }

    pub async fn delete_by_plan_version_id(
        conn: &mut PgConn,
        plan_version_id_param: &PlanVersionId,
    ) -> DbResult<()> {
        use crate::schema::plan_version_adjustment_component::dsl as c_dsl;

        let query = diesel::delete(c_dsl::plan_version_adjustment_component)
            .filter(c_dsl::plan_version_id.eq(plan_version_id_param))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan_version_adjustment_components")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[PlanVersionAdjustmentComponentRowNew],
    ) -> DbResult<()> {
        use crate::schema::plan_version_adjustment_component::dsl as c_dsl;

        if batch.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(c_dsl::plan_version_adjustment_component).values(batch);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting plan_version_adjustment_components batch")
            .map(|_| ())
            .into_db_result()
    }

    /// Atomic replace: delete all existing rows for `plan_version_id` (of this kind), insert
    /// `new_rows`. Both halves run inside the caller's transaction.
    pub async fn replace_for_plan_version(
        conn: &mut PgConn,
        plan_version_id_param: PlanVersionId,
        new_rows: &[PlanVersionAdjustmentComponentRowNew],
    ) -> DbResult<()> {
        Self::delete_by_plan_version_id(conn, &plan_version_id_param).await?;
        Self::insert_batch(conn, new_rows).await
    }
}

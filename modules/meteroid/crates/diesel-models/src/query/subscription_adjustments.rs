//! Queries for the subscription-level *adjustment* tables (the stateless floor/cap family).
//! See `query/plan_version_adjustments.rs` for the kind-scoping rationale; today these are
//! minimum-specific accessors over the generic `subscription_adjustment` table.

use crate::enums::AdjustmentKindEnum;
use crate::errors::IntoDbResult;
use crate::subscription_adjustments::{
    SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentComponentRowNew,
    SubscriptionAdjustmentRow, SubscriptionAdjustmentRowNew,
};
use crate::{DbResult, PgConn};

use common_domain::ids::SubscriptionId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionAdjustmentRow {
    pub async fn find_by_subscription_id(
        conn: &mut PgConn,
        subscription_id_param: &SubscriptionId,
    ) -> DbResult<Option<SubscriptionAdjustmentRow>> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;

        let query = m_dsl::subscription_adjustment
            .filter(m_dsl::subscription_id.eq(subscription_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(SubscriptionAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching subscription_adjustment")
            .into_db_result()
    }

    pub async fn delete_by_subscription_id(
        conn: &mut PgConn,
        subscription_id_param: &SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;

        let query = diesel::delete(m_dsl::subscription_adjustment)
            .filter(m_dsl::subscription_id.eq(subscription_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting subscription_adjustment")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn list_by_subscription_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionAdjustmentRow>> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = m_dsl::subscription_adjustment
            .filter(m_dsl::subscription_id.eq_any(subscription_ids))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(SubscriptionAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription_adjustments by ids")
            .into_db_result()
    }
}

impl SubscriptionAdjustmentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionAdjustmentRow> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;

        let query = diesel::insert_into(m_dsl::subscription_adjustment).values((
            self,
            crate::schema::subscription_adjustment::created_at
                .eq(common_domain::clock::now().naive_utc()),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting subscription_adjustment")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[SubscriptionAdjustmentRowNew],
    ) -> DbResult<()> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;

        if batch.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(m_dsl::subscription_adjustment).values(
            batch
                .iter()
                .map(|row| {
                    (
                        row,
                        crate::schema::subscription_adjustment::created_at
                            .eq(common_domain::clock::now().naive_utc()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting subscription_adjustment batch")
            .map(|_| ())
            .into_db_result()
    }
}

impl SubscriptionAdjustmentComponentRow {
    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        subscription_id_param: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAdjustmentComponentRow>> {
        use crate::schema::subscription_adjustment_component::dsl as c_dsl;

        let query = c_dsl::subscription_adjustment_component
            .filter(c_dsl::subscription_id.eq(subscription_id_param))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(SubscriptionAdjustmentComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription_adjustment_components")
            .into_db_result()
    }

    pub async fn list_by_subscription_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionAdjustmentComponentRow>> {
        use crate::schema::subscription_adjustment_component::dsl as c_dsl;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = c_dsl::subscription_adjustment_component
            .filter(c_dsl::subscription_id.eq_any(subscription_ids))
            .filter(c_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .select(SubscriptionAdjustmentComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription_adjustment_components by ids")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[SubscriptionAdjustmentComponentRowNew],
    ) -> DbResult<()> {
        use crate::schema::subscription_adjustment_component::dsl as c_dsl;

        if batch.is_empty() {
            return Ok(());
        }

        let query = diesel::insert_into(c_dsl::subscription_adjustment_component).values(batch);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting subscription_adjustment_components batch")
            .map(|_| ())
            .into_db_result()
    }
}

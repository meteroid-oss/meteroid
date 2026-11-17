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

use chrono::NaiveDate;
use common_domain::ids::SubscriptionId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};

/// A scope row of the minimum in force on its subscription's read day: a minimum a scheduled
/// change dated later shows from its date.
fn of_open_minimum() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    let in_force = crate::query::subscription_facts::in_force_sql("a", "s");
    diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
        "EXISTS (SELECT 1 FROM subscription_adjustment a \
         JOIN subscription s ON s.id = a.subscription_id \
         WHERE a.subscription_id = subscription_adjustment_component.subscription_id \
           AND a.kind = subscription_adjustment_component.kind \
           AND a.effective_from = subscription_adjustment_component.effective_from \
           AND {in_force})"
    ))
}
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

fn minimum_in_force() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    let day = crate::query::subscription_facts::read_day_sql("s");
    diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
        "EXISTS (SELECT 1 FROM subscription s \
         WHERE s.id = subscription_adjustment.subscription_id \
           AND subscription_adjustment.effective_from <= {day} \
           AND (subscription_adjustment.effective_to IS NULL \
                OR subscription_adjustment.effective_to > {day}))"
    ))
}

impl SubscriptionAdjustmentRow {
    pub async fn find_by_subscription_id(
        conn: &mut PgConn,
        subscription_id_param: &SubscriptionId,
    ) -> DbResult<Option<SubscriptionAdjustmentRow>> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;
        // In force on the subscription's read day: a scheduled change's minimum shows from its date.
        let query = m_dsl::subscription_adjustment
            .filter(m_dsl::subscription_id.eq(subscription_id_param))
            .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
            .filter(minimum_in_force())
            .select(SubscriptionAdjustmentRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching subscription_adjustment")
            .into_db_result()
    }

    /// Ends the minimums from `on`: those starting on or after it are dropped (a backdated change
    /// replaces them), the one covering it is closed there, so no two rows overlap.
    pub async fn end_current(
        conn: &mut PgConn,
        subscription_id_param: &SubscriptionId,
        on: NaiveDate,
    ) -> DbResult<()> {
        use crate::schema::subscription_adjustment::dsl as m_dsl;
        use diesel::BoolExpressionMethods;

        let minimums = || {
            m_dsl::subscription_adjustment
                .filter(m_dsl::subscription_id.eq(subscription_id_param))
                .filter(m_dsl::kind.eq(AdjustmentKindEnum::Minimum))
        };

        diesel::delete(minimums().filter(m_dsl::effective_from.ge(on)))
            .execute(conn)
            .await
            .attach("Error while deleting subscription_adjustment")
            .into_db_result()?;

        diesel::update(
            minimums()
                .filter(m_dsl::effective_from.lt(on))
                .filter(m_dsl::effective_to.is_null().or(m_dsl::effective_to.gt(on))),
        )
        .set(m_dsl::effective_to.eq(Some(on)))
        .execute(conn)
        .await
        .attach("Error while closing subscription_adjustment")
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
            .filter(minimum_in_force())
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
            crate::schema::subscription_adjustment::created_at.eq(common_clock::now().naive_utc()),
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
                            .eq(common_clock::now().naive_utc()),
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
            .filter(of_open_minimum())
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
            .filter(of_open_minimum())
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

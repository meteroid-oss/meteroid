use crate::errors::IntoDbResult;
use crate::plans::PlanRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{PlanId, TenantId};
use diesel::{ExpressionMethods, SelectableHelper};
use error_stack::ResultExt;

impl PlanRow {
    /// Who may see or get the plan; `None` lets anyone in. Live: no version is made.
    pub async fn set_eligibility(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
        eligibility: Option<serde_json::Value>,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::eligibility.eq(eligibility),
                p_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ))
            .returning(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while setting a plan's eligibility")
            .into_db_result()
    }
}

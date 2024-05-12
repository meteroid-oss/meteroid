use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersion, PlanVersionNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl PlanVersionNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_version).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan version")
            .into_db_result()
    }
}

impl PlanVersion {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding plan version by id")
            .into_db_result()
    }

    pub async fn find_latest_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
        is_draft: Option<bool>,
    ) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(is_draft) = is_draft {
            query = query.filter(pv_dsl::is_draft_version.eq(is_draft));
        }

        query = query.order_by(pv_dsl::version.desc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding latest plan version")
            .into_db_result()
    }
}

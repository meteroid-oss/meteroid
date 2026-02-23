use crate::errors::IntoDbResult;
use crate::plan_version_add_ons::{PlanVersionAddOnRow, PlanVersionAddOnRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{AddOnId, PlanVersionId, TenantId};
use diesel::{ExpressionMethods, Insertable, IntoSql, QueryDsl, SelectableHelper, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use tap::TapFallible;

impl PlanVersionAddOnRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersionAddOnRow> {
        use crate::schema::plan_version_add_on::dsl as pva_dsl;

        let query = diesel::insert_into(pva_dsl::plan_version_add_on).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting plan_version_add_on")
            .into_db_result()
    }
}

impl PlanVersionAddOnRow {
    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanVersionAddOnRow>> {
        use crate::schema::plan_version_add_on::dsl as pva_dsl;

        let query = pva_dsl::plan_version_add_on
            .filter(pva_dsl::plan_version_id.eq(plan_version_id))
            .filter(pva_dsl::tenant_id.eq(tenant_id))
            .select(PlanVersionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while listing plan_version_add_ons: {e:?}"))
            .attach("Error while listing plan_version_add_ons")
            .into_db_result()
    }

    pub async fn list_by_add_on_id(
        conn: &mut PgConn,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanVersionAddOnRow>> {
        use crate::schema::plan_version_add_on::dsl as pva_dsl;

        let query = pva_dsl::plan_version_add_on
            .filter(pva_dsl::add_on_id.eq(add_on_id))
            .filter(pva_dsl::tenant_id.eq(tenant_id))
            .select(PlanVersionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .tap_err(|e| {
                log::error!("Error while listing plan_version_add_ons by add_on_id: {e:?}")
            })
            .attach("Error while listing plan_version_add_ons by add_on_id")
            .into_db_result()
    }

    pub async fn clone_all(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
    ) -> DbResult<usize> {
        use crate::schema::plan_version_add_on::dsl as pva_dsl;

        diesel::define_sql_function! {
            fn gen_random_uuid() -> diesel::sql_types::Uuid;
        }

        let query = pva_dsl::plan_version_add_on
            .filter(pva_dsl::plan_version_id.eq(src_plan_version_id))
            .select((
                gen_random_uuid(),
                dst_plan_version_id.into_sql::<diesel::sql_types::Uuid>(),
                pva_dsl::add_on_id,
                pva_dsl::price_id,
                pva_dsl::self_serviceable,
                pva_dsl::max_instances_per_subscription,
                pva_dsl::tenant_id,
            ))
            .insert_into(pva_dsl::plan_version_add_on)
            .into_columns((
                pva_dsl::id,
                pva_dsl::plan_version_id,
                pva_dsl::add_on_id,
                pva_dsl::price_id,
                pva_dsl::self_serviceable,
                pva_dsl::max_instances_per_subscription,
                pva_dsl::tenant_id,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while cloning plan_version_add_ons")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        plan_version_id: PlanVersionId,
        add_on_id: AddOnId,
        tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::plan_version_add_on::dsl as pva_dsl;

        let query = diesel::delete(pva_dsl::plan_version_add_on)
            .filter(pva_dsl::plan_version_id.eq(plan_version_id))
            .filter(pva_dsl::add_on_id.eq(add_on_id))
            .filter(pva_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while deleting plan_version_add_on: {e:?}"))
            .attach("Error while deleting plan_version_add_on")
            .into_db_result()?;

        Ok(())
    }
}

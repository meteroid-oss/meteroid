use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponentRow, PriceComponentRowNew};
use std::collections::HashMap;

use crate::{DbResult, PgConn};

use common_domain::ids::{PlanVersionId, PriceComponentId, TenantId};
use diesel::{
    ExpressionMethods, Insertable, IntoSql, OptionalExtension, QueryDsl, SelectableHelper,
    debug_query,
};
use error_stack::ResultExt;
use itertools::Itertools;
use tap::prelude::*;

impl PriceComponentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PriceComponentRow> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }
}

impl PriceComponentRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: PriceComponentId,
    ) -> DbResult<PriceComponentRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version)
            .filter(pv_dsl::tenant_id.eq(param_tenant_id))
            .filter(pc_dsl::id.eq(param_id))
            .select(PriceComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while fetching price component")
            .into_db_result()
    }

    pub async fn insert(
        conn: &mut PgConn,
        price_component_param: PriceComponentRowNew,
    ) -> DbResult<PriceComponentRow> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_component_param);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        price_components: Vec<PriceComponentRowNew>,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_components);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price components: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }

    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        plan_version_id_param: PlanVersionId,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let query = price_component
            .inner_join(plan_version_dsl::plan_version)
            .filter(plan_version_id.eq(plan_version_id_param))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id_param))
            .select(PriceComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {e:?}"))
            .attach("Error while fetching price components")
            .into_db_result()
    }

    pub async fn get_by_plan_version_ids(
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
        param_tenant_id: TenantId,
    ) -> DbResult<HashMap<PlanVersionId, Vec<PriceComponentRow>>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl::{plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let query = price_component
            .inner_join(pv_dsl::plan_version)
            .filter(plan_version_id.eq_any(plan_version_ids))
            .filter(pv_dsl::tenant_id.eq(param_tenant_id))
            .select(PriceComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let res: Vec<PriceComponentRow> = query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {e:?}"))
            .attach("Error while fetching price components")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.plan_version_id);

        Ok(grouped)
    }

    pub async fn update(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Option<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{id, plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(self.plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::update(price_component)
            .filter(id.eq(self.id))
            .filter(plan_version_id.eq_any(plan_version_with_id_in_tenant))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating price component: {e:?}"))
            .attach("Error while updating price component")
            .into_db_result()
    }

    pub async fn delete_by_id_and_tenant(
        conn: &mut PgConn,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{id, plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        // check the tenant (https://github.com/diesel-rs/diesel/issues/1478)
        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::delete(price_component)
            .filter(id.eq(component_id))
            .filter(plan_version_id.eq_any(plan_version_with_id_in_tenant));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while deleting price component: {e:?}"))
            .attach("Error while deleting price component")
            .into_db_result()?;

        Ok(())
    }

    pub async fn list_all_by_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version)
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .select(PriceComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing all price components by tenant")
            .into_db_result()
    }

    pub async fn clone_all(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
    ) -> DbResult<usize> {
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        diesel::define_sql_function! {
            fn gen_random_uuid() -> Uuid;
        }

        let query = pc_dsl::price_component
            .filter(pc_dsl::plan_version_id.eq(src_plan_version_id))
            .select((
                gen_random_uuid(),
                pc_dsl::name,
                pc_dsl::legacy_fee,
                dst_plan_version_id.into_sql::<diesel::sql_types::Uuid>(),
                pc_dsl::product_id,
                pc_dsl::billable_metric_id,
            ))
            .insert_into(pc_dsl::price_component)
            .into_columns((
                pc_dsl::id,
                pc_dsl::name,
                pc_dsl::legacy_fee,
                pc_dsl::plan_version_id,
                pc_dsl::product_id,
                pc_dsl::billable_metric_id,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while cloning price components")
            .into_db_result()
    }
}

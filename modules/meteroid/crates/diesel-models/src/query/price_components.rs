use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponent, PriceComponentNew};
use std::collections::HashMap;

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use itertools::Itertools;
use tap::prelude::*;

impl PriceComponentNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PriceComponent> {
        use crate::schema::price_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {:?}", e))
            .attach_printable("Error while inserting price component")
            .into_db_result()
    }
}


// TODO check tenants in all methods
impl PriceComponent {
    pub async fn insert(
        conn: &mut PgConn,
        price_component_param: PriceComponentNew,
    ) -> DbResult<PriceComponent> {
        use crate::schema::price_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_component_param);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {:?}", e))
            .attach_printable("Error while inserting price component")
            .into_db_result()
    }


    pub async fn insert_batch(
        conn: &mut PgConn,
        price_components: Vec<PriceComponentNew>,
    ) -> DbResult<Vec<PriceComponent>> {
        use crate::schema::price_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_components);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price components: {:?}", e))
            .attach_printable("Error while inserting price component")
            .into_db_result()
    }


    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        tenant_id_param: uuid::Uuid,
        plan_version_id_param: uuid::Uuid,
    ) -> DbResult<Vec<PriceComponent>> {
        use crate::schema::price_component::dsl::*;
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use diesel_async::RunQueryDsl;

        let query = price_component
            .inner_join(plan_version_dsl::plan_version)
            .filter(plan_version_id.eq(plan_version_id_param))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id_param))
            .select(PriceComponent::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {:?}", e))
            .attach_printable("Error while fetching price components")
            .into_db_result()
    }

    pub async fn get_by_plan_ids(
        conn: &mut PgConn,
        plan_version_ids: &[uuid::Uuid],
    ) -> DbResult<HashMap<uuid::Uuid, Vec<PriceComponent>>> {
        use crate::schema::price_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = price_component.filter(plan_version_id.eq_any(plan_version_ids));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        let res: Vec<PriceComponent> = query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {:?}", e))
            .attach_printable("Error while fetching price components")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.plan_version_id);

        Ok(grouped)
    }

    pub async fn update(
        &self,
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Option<PriceComponent>> {
        use crate::schema::price_component::dsl::*;
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use diesel_async::RunQueryDsl;

        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(self.plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::update(price_component)
            .filter(id.eq(self.id))
            .filter(
                plan_version_id.eq_any(plan_version_with_id_in_tenant),
            )
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| {
                log::error!("Error while updating price component: {:?}", e)
            })
            .attach_printable("Error while updating price component")
            .into_db_result()
    }

    pub async fn delete_by_id_and_tenant(
        conn: &mut PgConn,
        component_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<()> {
        use crate::schema::price_component::dsl::*;
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use diesel_async::RunQueryDsl;

        // check the tenant (https://github.com/diesel-rs/diesel/issues/1478)
        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::delete(price_component)
            .filter(id.eq(component_id))
            .filter(
                plan_version_id.eq_any(plan_version_with_id_in_tenant),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .tap_err(|e| {
                log::error!("Error while deleting price component: {:?}", e)
            })
            .attach_printable("Error while deleting price component")
            .into_db_result()?;

        Ok(())
    }
}

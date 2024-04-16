use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponent, PriceComponentNew};
use std::collections::HashMap;

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, QueryDsl};
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

impl PriceComponent {
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
}

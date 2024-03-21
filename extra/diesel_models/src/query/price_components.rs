use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponent, PriceComponentNew};
use crate::schema::price_component;
use crate::{errors, DbResult, PgConn};
use common_utils::fp::TapExt;
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl PriceComponentNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PriceComponent> {
        use crate::schema::price_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .tap_error(|e| log::error!("Error while inserting price component: {:?}", e))
            .attach_printable("Error while inserting plan")
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
            .tap_error(|e| log::error!("Error while inserting price components: {:?}", e))
            .attach_printable("Error while inserting plan")
            .into_db_result()
    }
}

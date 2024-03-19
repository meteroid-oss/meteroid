use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponent, PriceComponentNew};
use crate::schema::price_component;
use crate::{errors, DbResult, PgConn};
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
            .into_db_result()
            .attach_printable("Error while inserting plan")
    }
}

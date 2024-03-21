use crate::errors::IntoDbResult;
use crate::product_families::{ProductFamily, ProductFamilyNew};
use crate::schema::product_family;
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl ProductFamilyNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProductFamily> {
        use crate::schema::product_family::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product_family).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting product family")
            .into_db_result()
    }
}

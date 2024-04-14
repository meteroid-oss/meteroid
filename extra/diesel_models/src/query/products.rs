use crate::errors::IntoDbResult;
use crate::products::{Product, ProductNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl ProductNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Product> {
        use crate::schema::product::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting product")
            .into_db_result()
    }
}

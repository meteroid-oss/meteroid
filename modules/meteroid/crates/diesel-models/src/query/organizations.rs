use crate::errors::IntoDbResult;
use crate::organizations::{Organization, OrganizationNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl OrganizationNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Organization> {
        use crate::schema::organization::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization")
            .into_db_result()
    }
}

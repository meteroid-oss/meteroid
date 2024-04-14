use crate::errors::IntoDbResult;
use crate::organization_members::OrganizationMember;

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl OrganizationMember {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationMember> {
        use crate::schema::organization_member::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization_member).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization member")
            .into_db_result()
    }
}

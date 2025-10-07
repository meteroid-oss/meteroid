use crate::errors::IntoDbResult;
use crate::organization_members::OrganizationMemberRow;

use crate::{DbResult, PgConn};

use common_domain::ids::OrganizationId;
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use diesel::debug_query;
use error_stack::ResultExt;
use uuid::Uuid;

impl OrganizationMemberRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationMemberRow> {
        use crate::schema::organization_member::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization_member).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization member")
            .into_db_result()
    }

    pub async fn find_by_user_and_org(
        conn: &mut PgConn,
        user_id_param: Uuid,
        org_id_param: OrganizationId,
    ) -> DbResult<OrganizationMemberRow> {
        use crate::schema::organization_member::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = organization_member
            .filter(user_id.eq(user_id_param))
            .filter(organization_id.eq(org_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization member")
            .into_db_result()
    }
}

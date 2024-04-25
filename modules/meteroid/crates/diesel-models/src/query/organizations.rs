use crate::errors::IntoDbResult;
use crate::organizations::{Organization, OrganizationNew};

use crate::{DbResult, PgConn};

use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{debug_query, JoinOnDsl, SelectableHelper};
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

impl Organization {
    pub async fn by_user_id(conn: &mut PgConn, user_id: uuid::Uuid) -> DbResult<Organization> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization
            .inner_join(om_dsl::organization_member.on(o_dsl::id.eq(om_dsl::organization_id)))
            .inner_join(u_dsl::user.on(om_dsl::user_id.eq(u_dsl::id)))
            .filter(u_dsl::id.eq(user_id))
            .select(Organization::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by user_id")
            .into_db_result()
    }

    pub async fn find_by_invite_link(
        conn: &mut PgConn,
        invite_link_hash: String,
    ) -> DbResult<Organization> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.filter(o_dsl::invite_link_hash.eq(invite_link_hash));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by invite_link_hash")
            .into_db_result()
    }
}

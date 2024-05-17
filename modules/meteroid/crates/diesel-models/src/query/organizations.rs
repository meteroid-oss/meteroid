use crate::errors::IntoDbResult;
use crate::organizations::{Organization, OrganizationNew};

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use tap::TapFallible;

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
    pub async fn find_all(conn: &mut PgConn) -> DbResult<Vec<Organization>> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.select(Organization::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while obtaining all organizations")
            .into_db_result()
    }

    pub async fn find_by_user_id(conn: &mut PgConn, user_id: uuid::Uuid) -> DbResult<Organization> {
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

    pub async fn update_invite_link(
        conn: &mut PgConn,
        param_id: uuid::Uuid,
        new_invite_hash_link: &String,
    ) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(o_dsl::organization)
            .filter(o_dsl::id.eq(param_id))
            .set(o_dsl::invite_link_hash.eq(new_invite_hash_link));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while updating organization: {:?}", e))
            .attach_printable("Error while updating organization")
            .into_db_result()
    }
}

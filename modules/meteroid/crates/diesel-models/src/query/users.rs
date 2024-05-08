use crate::errors::IntoDbResult;

use crate::users::{User, UserNew};
use crate::{DbResult, PgConn};

use diesel::{
    debug_query, ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use uuid::Uuid;

impl UserNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::user::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(dsl::user).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach_printable("Error while inserting user")
            .into_db_result()
    }
}

impl User {
    pub async fn find_by_id(conn: &mut PgConn, id: Uuid) -> DbResult<User> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::id.eq(id))
            .select(User::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding user by id")
            .into_db_result()
    }

    pub async fn find_by_id_and_org_id(
        conn: &mut PgConn,
        id: Uuid,
        organization_id: Uuid,
    ) -> DbResult<User> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::id.eq(id))
            .filter(om_dsl::organization_id.eq(organization_id))
            .select(User::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding user by id")
            .into_db_result()
    }

    pub async fn find_by_email_and_org_id(
        conn: &mut PgConn,
        email: String,
        organization_id: Uuid,
    ) -> DbResult<User> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::email.eq(email))
            .filter(om_dsl::organization_id.eq(organization_id))
            .select(User::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding user by email")
            .into_db_result()
    }

    pub async fn find_by_email(conn: &mut PgConn, email: String) -> DbResult<Option<User>> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::email.eq(email))
            .select(User::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .optional()
            .attach_printable("Error while finding user by email")
            .into_db_result()
    }

    pub async fn list_by_org_id(conn: &mut PgConn, organization_id: Uuid) -> DbResult<Vec<User>> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(om_dsl::organization_id.eq(organization_id))
            .select(User::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing users")
            .into_db_result()
    }

    pub async fn any_exists(conn: &mut PgConn) -> DbResult<bool> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(u_dsl::user.limit(1)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while checking if any user exists")
            .into_db_result()
    }
}

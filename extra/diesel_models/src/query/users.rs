use crate::errors::IntoDbResult;

use crate::users::{User, UserNew};
use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl UserNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<User> {
        use crate::schema::user::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(user).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting user")
            .into_db_result()
    }
}

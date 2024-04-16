use crate::api_tokens::{ApiToken, ApiTokenNew};
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl ApiTokenNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ApiToken> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(api_token).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting api token")
            .into_db_result()
    }
}

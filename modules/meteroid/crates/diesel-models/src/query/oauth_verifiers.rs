use crate::errors::IntoDbResult;
use crate::oauth_verifiers::OauthVerifierRow;
use crate::{DbResult, PgConn};
use diesel::{ExpressionMethods, SelectableHelper, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl OauthVerifierRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OauthVerifierRow> {
        use crate::schema::oauth_verifier::dsl as ov_dsl;

        let query = diesel::insert_into(ov_dsl::oauth_verifier).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting oauth_verifier")
            .into_db_result()
    }

    pub async fn delete_by_csrf_token(
        conn: &mut PgConn,
        csrf_token: &str,
    ) -> DbResult<OauthVerifierRow> {
        use crate::schema::oauth_verifier::dsl as ov_dsl;

        let query = diesel::delete(ov_dsl::oauth_verifier)
            .filter(ov_dsl::csrf_token.eq(csrf_token))
            .returning(OauthVerifierRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while deleting oauth_verifier")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        created_before: chrono::NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::oauth_verifier::dsl as ov_dsl;

        let query =
            diesel::delete(ov_dsl::oauth_verifier).filter(ov_dsl::created_at.lt(created_before));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting expired oauth_verifiers")
            .into_db_result()
    }
}

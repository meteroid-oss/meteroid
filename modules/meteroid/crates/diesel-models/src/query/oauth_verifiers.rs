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
            .attach("Error while inserting oauth_verifier")
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
            .attach("Error while deleting oauth_verifier")
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
            .attach("Error while deleting expired oauth_verifiers")
            .into_db_result()
    }
}

/// Secret-envelope maintenance: see `ConnectorRow::lock_legacy_sensitive`.
impl OauthVerifierRow {
    pub async fn lock_legacy_pkce_verifiers(
        conn: &mut PgConn,
        envelope_prefix: &str,
    ) -> DbResult<Vec<(uuid::Uuid, String)>> {
        use crate::schema::oauth_verifier::dsl as ov_dsl;
        use diesel::{QueryDsl, TextExpressionMethods};
        use diesel_async::RunQueryDsl;

        let query = ov_dsl::oauth_verifier
            .filter(ov_dsl::pkce_verifier.not_like(format!("{envelope_prefix}%")))
            .select((ov_dsl::id, ov_dsl::pkce_verifier))
            .order(ov_dsl::id.asc())
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while locking legacy oauth verifiers")
            .into_db_result()
    }

    pub async fn set_pkce_verifier(
        conn: &mut PgConn,
        id: uuid::Uuid,
        pkce_verifier: &str,
    ) -> DbResult<usize> {
        use crate::schema::oauth_verifier::dsl as ov_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(ov_dsl::oauth_verifier)
            .filter(ov_dsl::id.eq(id))
            .set(ov_dsl::pkce_verifier.eq(pkce_verifier));

        query
            .execute(conn)
            .await
            .attach("Error while rewriting oauth verifier")
            .into_db_result()
    }
}

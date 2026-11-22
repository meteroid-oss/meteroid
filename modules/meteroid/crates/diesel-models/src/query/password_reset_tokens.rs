use crate::errors::IntoDbResult;
use crate::password_reset_tokens::{PasswordResetTokenRow, PasswordResetTokenRowNew};
use crate::{DbResult, PgConn};

use chrono::{DateTime, Utc};
use common_domain::ids::UserId;
use diesel::prelude::ExpressionMethods;
use diesel::{OptionalExtension, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl PasswordResetTokenRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::password_reset_token::dsl as d;

        let query = diesel::insert_into(d::password_reset_token).values(self);
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while inserting password reset token")
            .into_db_result()
    }
}

impl PasswordResetTokenRow {
    /// Marks the token used if it is still open, returning it. Of concurrent callers only one
    /// gets it back.
    pub async fn consume(
        conn: &mut PgConn,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> DbResult<Option<Self>> {
        use crate::schema::password_reset_token::dsl as d;

        let query = diesel::update(d::password_reset_token)
            .filter(d::token_hash.eq(token_hash))
            .filter(d::consumed_at.is_null())
            .filter(d::expires_at.gt(now))
            .set(d::consumed_at.eq(now))
            .returning(PasswordResetTokenRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while consuming password reset token")
            .into_db_result()
    }

    pub async fn consume_all_for_user(
        conn: &mut PgConn,
        user_id: UserId,
        now: DateTime<Utc>,
    ) -> DbResult<usize> {
        use crate::schema::password_reset_token::dsl as d;

        let query = diesel::update(d::password_reset_token)
            .filter(d::user_id.eq(user_id))
            .filter(d::consumed_at.is_null())
            .set(d::consumed_at.eq(now));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while closing password reset tokens")
            .into_db_result()
    }

    pub async fn delete_expired_before(
        conn: &mut PgConn,
        before: DateTime<Utc>,
    ) -> DbResult<usize> {
        use crate::schema::password_reset_token::dsl as d;

        let query = diesel::delete(d::password_reset_token).filter(d::expires_at.lt(before));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting expired password reset tokens")
            .into_db_result()
    }
}

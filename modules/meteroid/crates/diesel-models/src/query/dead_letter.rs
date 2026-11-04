use crate::dead_letter::DeadLetterMessageRowNew;
use crate::errors::IntoDbResult;
use crate::schema::dead_letter_message;
use crate::{DbResult, PgConn};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl DeadLetterMessageRowNew {
    pub async fn insert_batch(conn: &mut PgConn, entries: &[Self]) -> DbResult<()> {
        diesel::insert_into(dead_letter_message::table)
            .values(entries)
            .execute(conn)
            .await
            .map(drop)
            .attach("Failed to insert dead letter messages")
            .into_db_result()
    }
}

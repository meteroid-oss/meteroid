use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use crate::outbox_event::OutboxEventRowNew;
use diesel::debug_query;
use error_stack::ResultExt;

impl OutboxEventRowNew {
    pub async fn insert_batch(conn: &mut PgConn, events: &[OutboxEventRowNew]) -> DbResult<()> {
        use crate::schema::outbox_event::dsl as oe_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(oe_dsl::outbox_event).values(events);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while inserting outbox events")
            .into_db_result()
            .map(|_| ())
    }
}

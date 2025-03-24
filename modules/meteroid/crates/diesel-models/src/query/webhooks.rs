use crate::errors::IntoDbResult;
use crate::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use crate::{DbResult, PgConn};
use diesel::debug_query;
use error_stack::ResultExt;

impl WebhookInEventRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookInEventRow> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(wi_dsl::webhook_in_event).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting webhook_in_event")
            .into_db_result()
    }
}

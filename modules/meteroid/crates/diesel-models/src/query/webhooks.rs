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
            .attach("Error while inserting webhook_in_event")
            .into_db_result()
    }

    /// Insert with idempotency on `(provider_config_id, provider_event_id)`.
    /// Returns `Ok(None)` when a row with the same provider event id already
    /// exists for this connector — the caller treats that as a duplicate
    /// delivery and skips processing.
    ///
    /// `ON CONFLICT DO NOTHING` means PostgreSQL silently swallows the
    /// constraint violation; we read the row count to tell success from
    /// dedup. The partial index excludes legacy rows with NULL
    /// provider_event_id so this insert path is safe to mix with old data.
    pub async fn insert_or_skip_if_duplicate(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<Option<WebhookInEventRow>> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(wi_dsl::webhook_in_event)
            .values(self)
            .on_conflict((wi_dsl::provider_config_id, wi_dsl::provider_event_id))
            .do_nothing();
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .map(|rows: Vec<WebhookInEventRow>| rows.into_iter().next())
            .attach("Error while inserting webhook_in_event with idempotency")
            .into_db_result()
    }
}

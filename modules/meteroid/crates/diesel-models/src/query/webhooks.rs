use crate::errors::IntoDbResult;
use crate::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use crate::{DbResult, PgConn};
use diesel::OptionalExtension;
use diesel::debug_query;
use diesel::prelude::ExpressionMethods;
use diesel::query_dsl::methods::FilterDsl;
use error_stack::ResultExt;
use uuid::Uuid;

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

    /// Insert with idempotency on `(provider_config_id, provider_event_id)`,
    /// returning `Ok(None)` on conflict. The index is non-partial (Diesel's
    /// `ON CONFLICT (cols)` can't match a partial one), so under `NULLS
    /// DISTINCT` a NULL `provider_event_id` never dedups — only rely on this for
    /// events that carry an id.
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

impl WebhookInEventRow {
    /// A returned row means the event was already processed (or permanently
    /// rejected); the router skips it.
    pub async fn find_by_provider_event(
        conn: &mut PgConn,
        provider_config_id_param: Uuid,
        provider_event_id_param: &str,
    ) -> DbResult<Option<WebhookInEventRow>> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        let query = FilterDsl::filter(
            wi_dsl::webhook_in_event,
            wi_dsl::provider_config_id.eq(provider_config_id_param),
        )
        .filter(wi_dsl::provider_event_id.eq(provider_event_id_param));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while looking up webhook_in_event by provider event id")
            .into_db_result()
    }
}

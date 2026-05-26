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

    /// Insert with idempotency on `(provider_config_id, provider_event_id)`.
    /// Returns `Ok(None)` when a row with the same provider event id already
    /// exists for this connector — the caller treats that as a duplicate
    /// delivery and skips processing.
    ///
    /// `ON CONFLICT DO NOTHING` means PostgreSQL silently swallows the
    /// constraint violation; we read the row count to tell success from
    /// dedup. NOTE: the unique index is **not** partial (see the migration for
    /// why — Diesel's `ON CONFLICT (cols)` without a predicate cannot match a
    /// partial index). Under PostgreSQL's default `NULLS DISTINCT`, rows with a
    /// NULL `provider_event_id` never conflict, so callers must only rely on
    /// this dedup for events that carry a provider event id.
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
    /// Look up a previously-recorded inbound event by its provider config +
    /// provider event id. The webhook router uses this to decide whether a
    /// (re)delivery has already been handled before doing any work — a row
    /// existing here means "we already processed (or permanently rejected)
    /// this event", so it must be skipped.
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

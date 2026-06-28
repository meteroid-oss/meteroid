use crate::errors::IntoDbResult;
use crate::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
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

    /// Insert the event, deduping on `(provider_config_id, event_id)`.
    /// Returns `None` when a row with the same provider event id already exists
    /// (a repeated webhook delivery); `Some(row)` when this is a fresh event.
    pub async fn insert_dedup(&self, conn: &mut PgConn) -> DbResult<Option<WebhookInEventRow>> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(wi_dsl::webhook_in_event)
            .values(self)
            .on_conflict((wi_dsl::provider_config_id, wi_dsl::event_id))
            .do_nothing();
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while inserting webhook_in_event (dedup)")
            .into_db_result()
    }
}

impl WebhookInEventRow {
    pub async fn get_by_id(conn: &mut PgConn, event_uid: Uuid) -> DbResult<WebhookInEventRow> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        wi_dsl::webhook_in_event
            .filter(wi_dsl::id.eq(event_uid))
            .select(WebhookInEventRow::as_select())
            .first(conn)
            .await
            .attach("Error while fetching webhook_in_event")
            .into_db_result()
    }

    pub async fn mark_processed(
        conn: &mut PgConn,
        event_uid: Uuid,
        processed_at: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(wi_dsl::webhook_in_event)
            .filter(wi_dsl::id.eq(event_uid))
            .set((
                wi_dsl::processed_at.eq(Some(processed_at)),
                wi_dsl::error.eq(None::<String>),
            ))
            .execute(conn)
            .await
            .attach("Error while marking webhook_in_event processed")
            .into_db_result()?;

        Ok(())
    }

    pub async fn mark_failed(conn: &mut PgConn, event_uid: Uuid, error: String) -> DbResult<()> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(wi_dsl::webhook_in_event)
            .filter(wi_dsl::id.eq(event_uid))
            .set((
                wi_dsl::attempts.eq(wi_dsl::attempts + 1),
                wi_dsl::error.eq(Some(error)),
            ))
            .execute(conn)
            .await
            .attach("Error while marking webhook_in_event failed")
            .into_db_result()?;

        Ok(())
    }
}

use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use crate::enums::OutboxStatus;
use crate::outbox::{OutboxRow, OutboxRowNew, OutboxRowPatch};
use diesel::{debug_query, ExpressionMethods};
use error_stack::ResultExt;

impl OutboxRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OutboxRow> {
        use crate::schema::outbox::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(outbox).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting outbox")
            .into_db_result()
    }
}

impl OutboxRow {
    pub async fn claim_outbox_entries(
        conn: &mut PgConn,
        batch_size: i64,
        event_type: &str,
    ) -> DbResult<Vec<OutboxRow>> {
        use diesel::sql_types::{BigInt, Integer, Text};
        use diesel_async::RunQueryDsl;
        const MAX_ATTEMPTS: i32 = 5;

        let query = r#"
        UPDATE outbox
        SET status = 'PROCESSING',
            processing_started_at = NOW(),
            processing_attempts = processing_attempts + 1
        WHERE id IN (
            SELECT id FROM outbox
            WHERE
                (
                    status = 'AVAILABLE' OR
                    (status = 'PROCESSING' AND processing_started_at < NOW() - INTERVAL '30 minutes') OR
                    (status = 'FAILED' AND processing_attempts < $3)
                )
                AND event_type = $2
            ORDER BY created_at
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING *
        "#;

        diesel::sql_query(query)
            .bind::<BigInt, _>(batch_size)
            .bind::<Text, _>(event_type.to_string())
            .bind::<Integer, _>(MAX_ATTEMPTS)
            .get_results::<OutboxRow>(conn)
            .await
            .attach_printable("Error while finding organization by slug")
            .into_db_result()
    }

    pub async fn mark_outbox_entries_as_processed(
        conn: &mut PgConn,
        ids: Vec<uuid::Uuid>,
    ) -> DbResult<()> {
        use crate::schema::outbox::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::outbox)
            .filter(dsl::id.eq_any(ids))
            .set((
                dsl::status.eq(OutboxStatus::Completed),
                dsl::processing_completed_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while marking outbox entries as processed")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn mark_outbox_entries_as_failed(
        conn: &mut PgConn,
        ids: Vec<uuid::Uuid>,
        error: String,
    ) -> DbResult<()> {
        use crate::schema::outbox::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::outbox)
            .filter(dsl::id.eq_any(ids))
            .set((dsl::status.eq(OutboxStatus::Failed), dsl::error.eq(error)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while marking outbox entries as failed")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn mark_outbox_entry_as_failed(
        conn: &mut PgConn,
        id: uuid::Uuid,
        error: String,
    ) -> DbResult<()> {
        use crate::schema::outbox::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::outbox)
            .filter(dsl::id.eq(id))
            .set((dsl::status.eq(OutboxStatus::Failed), dsl::error.eq(error)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while marking outbox entry as failed")
            .into_db_result()
            .map(|_| ())
    }
}

impl OutboxRowPatch {
    pub async fn patch_outbox(&self, conn: &mut PgConn) -> DbResult<OutboxRow> {
        use crate::schema::outbox::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::outbox)
            .filter(dsl::id.eq(self.id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while patching outbox")
            .into_db_result()
    }
}

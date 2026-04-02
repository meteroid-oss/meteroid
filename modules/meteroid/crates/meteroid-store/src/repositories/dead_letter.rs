use crate::domain::dead_letter::{
    DeadLetterMessage, DeadLetterMessageNew, DeadLetterQueueStats, DeadLetterStatus,
};
use crate::domain::pgmq::PgmqQueue;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use chrono::NaiveDateTime;
use common_domain::pgmq::{Headers, Message};
use diesel_models::query::{dead_letter, pgmq};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait DeadLetterInterface {
    async fn insert_dead_letter_batch(&self, entries: Vec<DeadLetterMessageNew>)
        -> StoreResult<()>;

    async fn list_dead_letters(
        &self,
        queue: Option<&str>,
        status: Option<DeadLetterStatus>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<DeadLetterMessage>, i64)>;

    async fn get_dead_letter(&self, id: Uuid) -> StoreResult<DeadLetterMessage>;

    async fn requeue_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage>;

    async fn discard_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage>;

    async fn dead_letter_queue_stats(&self) -> StoreResult<Vec<DeadLetterQueueStats>>;

    async fn dead_letters_pending_since(
        &self,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<DeadLetterQueueStats>>;

    async fn upsert_dead_letter_alert_state(&self, queue: &str) -> StoreResult<()>;

    async fn get_dead_letter_alert_state(
        &self,
        queue: &str,
    ) -> StoreResult<Option<NaiveDateTime>>;
}

#[async_trait::async_trait]
impl DeadLetterInterface for Store {
    async fn insert_dead_letter_batch(
        &self,
        entries: Vec<DeadLetterMessageNew>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let rows = entries.into_iter().map(Into::into).collect();
        dead_letter::insert_batch(&mut conn, rows)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn list_dead_letters(
        &self,
        queue: Option<&str>,
        status: Option<DeadLetterStatus>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<(Vec<DeadLetterMessage>, i64)> {
        let mut conn = self.get_conn().await?;

        let status_db = status.map(Into::into);

        let items = dead_letter::list(&mut conn, queue, status_db.clone(), limit, offset)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect::<Vec<_>>())
            .map_err(Into::<Report<StoreError>>::into)?;

        let total = dead_letter::count(&mut conn, queue, status_db)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok((items, total))
    }

    async fn get_dead_letter(&self, id: Uuid) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;
        dead_letter::get_by_id(&mut conn, id)
            .await
            .map(Into::into)
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn requeue_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;

        let entry: DeadLetterMessage =
            dead_letter::get_by_id(&mut conn, id)
                .await
                .map(Into::into)
                .map_err(Into::<Report<StoreError>>::into)?;

        if entry.status != DeadLetterStatus::Pending {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Dead letter {} is not in PENDING status",
                id
            ))));
        }

        let queue: PgmqQueue = entry.queue.parse().map_err(|e: String| {
            Report::new(StoreError::InvalidArgument(e))
        })?;

        let msg = diesel_models::pgmq::PgmqMessageRowNew {
            message: entry.message.map(Message),
            headers: entry.headers.map(Headers),
        };

        pgmq::send_batch(&mut conn, queue.as_str(), &[msg])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let updated = dead_letter::update_status(
            &mut conn,
            id,
            diesel_models::enums::DeadLetterStatusEnum::Requeued,
            resolved_by,
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.into())
    }

    async fn discard_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;

        let entry: DeadLetterMessage =
            dead_letter::get_by_id(&mut conn, id)
                .await
                .map(Into::into)
                .map_err(Into::<Report<StoreError>>::into)?;

        if entry.status != DeadLetterStatus::Pending {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Dead letter {} is not in PENDING status",
                id
            ))));
        }

        let updated = dead_letter::update_status(
            &mut conn,
            id,
            diesel_models::enums::DeadLetterStatusEnum::Discarded,
            resolved_by,
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.into())
    }

    async fn dead_letter_queue_stats(&self) -> StoreResult<Vec<DeadLetterQueueStats>> {
        let mut conn = self.get_conn().await?;
        dead_letter::queue_stats(&mut conn)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|r| DeadLetterQueueStats {
                        queue: r.queue,
                        pending_count: r.pending_count,
                        requeued_count: r.requeued_count,
                        discarded_count: r.discarded_count,
                    })
                    .collect()
            })
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn dead_letters_pending_since(
        &self,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<DeadLetterQueueStats>> {
        let mut conn = self.get_conn().await?;
        dead_letter::pending_since(&mut conn, since)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|r| DeadLetterQueueStats {
                        queue: r.queue,
                        pending_count: r.pending_count,
                        requeued_count: r.requeued_count,
                        discarded_count: r.discarded_count,
                    })
                    .collect()
            })
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn upsert_dead_letter_alert_state(&self, queue: &str) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        dead_letter::upsert_alert_state(&mut conn, queue)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn get_dead_letter_alert_state(
        &self,
        queue: &str,
    ) -> StoreResult<Option<NaiveDateTime>> {
        let mut conn = self.get_conn().await?;
        dead_letter::get_alert_state(&mut conn, queue)
            .await
            .map(|opt| opt.map(|r| r.last_alerted_at))
            .map_err(Into::<Report<StoreError>>::into)
    }
}

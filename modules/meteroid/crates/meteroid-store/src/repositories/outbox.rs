use crate::StoreResult;
use crate::domain::outbox_event;
use crate::domain::pgmq::PgmqQueue;
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use diesel_models::query::pgmq;
use error_stack::Report;

#[async_trait::async_trait]
pub trait OutboxInterface {
    async fn insert_outbox_event(&self, event: outbox_event::OutboxEvent) -> StoreResult<()>;
    async fn insert_outbox_event_tx(&self, conn: &mut PgConn, event: outbox_event::OutboxEvent) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl OutboxInterface for Store {
    async fn insert_outbox_event(&self, event: outbox_event::OutboxEvent) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        self.internal
            .insert_outbox_events_tx(&mut conn, vec![event])
            .await
    }

    async fn insert_outbox_event_tx(&self, conn: &mut PgConn, event: outbox_event::OutboxEvent) -> StoreResult<()> {
        self.internal
            .insert_outbox_events_tx(conn, vec![event])
            .await
    }
}

impl StoreInternal {
    pub async fn insert_outbox_events_tx(
        &self,
        conn: &mut PgConn,
        events: Vec<outbox_event::OutboxEvent>,
    ) -> StoreResult<()> {
        let batch = events
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        pgmq::send_batch(conn, PgmqQueue::OutboxEvent.as_str(), &batch)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

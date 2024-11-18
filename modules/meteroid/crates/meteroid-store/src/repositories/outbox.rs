use error_stack::Report;
use uuid::Uuid;

use crate::domain::{outbox_event, Outbox, OutboxEvent, OutboxNew};
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use crate::StoreResult;
use diesel_models::outbox::{OutboxRow, OutboxRowNew};
use diesel_models::outbox_event::OutboxEventRowNew;

#[async_trait::async_trait]
pub trait OutboxInterface {
    async fn claim_outbox_entries(
        &self,
        event_types: Vec<OutboxEvent>,
        batch_size: i64,
    ) -> StoreResult<Vec<Outbox>>;

    async fn mark_outbox_entries_as_completed(&self, ids: Vec<Uuid>) -> StoreResult<()>;
    async fn mark_outbox_entries_as_failed(&self, ids: Vec<Uuid>, error: String)
        -> StoreResult<()>;
    async fn mark_outbox_entry_as_failed(&self, id: Uuid, error: String) -> StoreResult<()>;

    async fn insert_outbox_item_no_tx(&self, item: OutboxNew) -> StoreResult<Outbox>;

    async fn insert_outbox_event(&self, event: outbox_event::OutboxEvent) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl OutboxInterface for Store {
    async fn claim_outbox_entries(
        &self,
        event_types: Vec<OutboxEvent>,
        batch_size: i64,
    ) -> StoreResult<Vec<Outbox>> {
        let mut conn = self.get_conn().await?;

        let event_types: Vec<String> = event_types
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        OutboxRow::claim_outbox_entries(&mut conn, batch_size, event_types)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn mark_outbox_entries_as_completed(&self, ids: Vec<Uuid>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        OutboxRow::mark_outbox_entries_as_completed(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn mark_outbox_entries_as_failed(
        &self,
        ids: Vec<Uuid>,
        error: String,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        OutboxRow::mark_outbox_entries_as_failed(&mut conn, ids, error)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn mark_outbox_entry_as_failed(&self, id: Uuid, error: String) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        OutboxRow::mark_outbox_entry_as_failed(&mut conn, id, error)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn insert_outbox_item_no_tx(&self, item: OutboxNew) -> StoreResult<Outbox> {
        let mut conn = self.get_conn().await?;
        self.internal.insert_outbox_item(&mut conn, item).await
    }

    async fn insert_outbox_event(&self, event: outbox_event::OutboxEvent) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        self.internal
            .insert_outbox_events_tx(&mut conn, vec![event])
            .await
    }
}

impl StoreInternal {
    pub async fn insert_outbox_item(
        &self,
        conn: &mut PgConn,
        item: OutboxNew,
    ) -> StoreResult<Outbox> {
        let row: OutboxRowNew = item.try_into()?;
        let outbox_created = row.insert(conn).await?;
        outbox_created.try_into()
    }

    pub async fn insert_outbox_events_tx(
        &self,
        conn: &mut PgConn,
        events: Vec<outbox_event::OutboxEvent>,
    ) -> StoreResult<()> {
        let rows: Vec<OutboxEventRowNew> = events
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        OutboxEventRowNew::insert_batch(conn, &rows)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

use error_stack::Report;
use uuid::Uuid;

use diesel_models::outbox::{OutboxRow, OutboxRowNew, OutboxRowPatch};

use crate::domain::{Outbox, OutboxEvent, OutboxNew, OutboxPatch};
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use crate::StoreResult;

#[async_trait::async_trait]
pub trait OutboxInterface {
    async fn claim_outbox_entries(
        &self,
        event_type: OutboxEvent,
        batch_size: i64,
    ) -> StoreResult<Vec<Outbox>>;

    async fn mark_outbox_entries_as_processed(&self, ids: Vec<Uuid>) -> StoreResult<()>;
    async fn mark_outbox_entries_as_failed(&self, ids: Vec<Uuid>, error: String)
        -> StoreResult<()>;
    async fn mark_outbox_entry_as_failed(&self, id: Uuid, error: String) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl OutboxInterface for Store {
    async fn claim_outbox_entries(
        &self,
        event_type: OutboxEvent,
        batch_size: i64,
    ) -> StoreResult<Vec<Outbox>> {
        let mut conn = self.get_conn().await?;

        let event_type: String = event_type.try_into()?;

        OutboxRow::claim_outbox_entries(&mut conn, batch_size, event_type.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn mark_outbox_entries_as_processed(&self, ids: Vec<Uuid>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        OutboxRow::mark_outbox_entries_as_processed(&mut conn, ids)
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
}

impl StoreInternal {
    pub async fn insert_outbox_item(
        &self,
        conn: &mut PgConn,
        item: OutboxNew,
    ) -> StoreResult<Outbox> {
        let row: OutboxRowNew = item.try_into()?;
        let outbox_created = row.insert(conn).await?;
        Ok(outbox_created.try_into()?)
    }
}

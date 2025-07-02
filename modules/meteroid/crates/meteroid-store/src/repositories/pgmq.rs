use crate::domain::pgmq::{PgmqMessage, PgmqMessageNew, PgmqQueue};
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec};
use diesel_models::query::pgmq;
use error_stack::Report;

use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};

#[async_trait::async_trait]
pub trait PgmqInterface {
    async fn pgmq_send_batch(
        &self,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
    ) -> StoreResult<()>;

    async fn pgmq_send_batch_tx(
        &self,
        conn: &mut PgConn,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
    ) -> StoreResult<()>;

    async fn pgmq_read(
        &self,
        queue: PgmqQueue,
        qty: MessageReadQty,
        vt: MessageReadVtSec,
    ) -> StoreResult<Vec<PgmqMessage>>;

    async fn pgmq_archive(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()>;

    async fn pgmq_delete(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()>;

    async fn pgmq_list_archived(
        &self,
        queue: PgmqQueue,
        ids: Vec<MessageId>,
    ) -> StoreResult<Vec<PgmqMessage>>;
}

#[async_trait::async_trait]
impl PgmqInterface for Store {
    async fn pgmq_send_batch(
        &self,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        self.pgmq_send_batch_tx(&mut conn, queue, messages).await
    }

    async fn pgmq_send_batch_tx(
        &self,
        conn: &mut PgConn,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
    ) -> StoreResult<()> {
        let rows = messages.into_iter().map(Into::into).collect::<Vec<_>>();

        pgmq::send_batch(conn, queue.as_str(), &rows)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn pgmq_read(
        &self,
        queue: PgmqQueue,
        qty: MessageReadQty,
        vt: MessageReadVtSec,
    ) -> StoreResult<Vec<PgmqMessage>> {
        let mut conn = self.get_conn().await?;

        pgmq::read(&mut conn, queue.as_str(), qty, vt)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn pgmq_archive(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        pgmq::archive(&mut conn, queue.as_str(), &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn pgmq_delete(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        pgmq::delete(&mut conn, queue.as_str(), &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn pgmq_list_archived(
        &self,
        queue: PgmqQueue,
        ids: Vec<MessageId>,
    ) -> StoreResult<Vec<PgmqMessage>> {
        let mut conn = self.get_conn().await?;
        pgmq::list_archived(&mut conn, queue.as_str(), &ids)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::<Report<StoreError>>::into)
    }
}

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

    /// Sends messages that only become visible to consumers after `delay_seconds`.
    async fn pgmq_send_batch_delayed(
        &self,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
        delay_seconds: i32,
    ) -> StoreResult<()>;

    async fn pgmq_read(
        &self,
        queue: PgmqQueue,
        qty: MessageReadQty,
        vt: MessageReadVtSec,
    ) -> StoreResult<Vec<PgmqMessage>>;

    async fn pgmq_archive(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()>;

    async fn pgmq_delete(&self, queue: PgmqQueue, ids: Vec<MessageId>) -> StoreResult<()>;

    /// Hides each message until `now() + vt` seconds (per-message retry delay).
    async fn pgmq_set_vt(&self, queue: PgmqQueue, vts: Vec<(MessageId, i32)>) -> StoreResult<()>;
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

    async fn pgmq_send_batch_delayed(
        &self,
        queue: PgmqQueue,
        messages: Vec<PgmqMessageNew>,
        delay_seconds: i32,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let rows = messages.into_iter().map(Into::into).collect::<Vec<_>>();

        pgmq::send_batch_delayed(&mut conn, queue.as_str(), &rows, delay_seconds)
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

    async fn pgmq_set_vt(&self, queue: PgmqQueue, vts: Vec<(MessageId, i32)>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let (ids, vts): (Vec<_>, Vec<_>) = vts.into_iter().unzip();

        pgmq::set_vt(&mut conn, queue.as_str(), &ids, &vts)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

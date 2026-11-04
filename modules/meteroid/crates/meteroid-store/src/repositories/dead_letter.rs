use crate::domain::dead_letter::DeadLetterMessageNew;
use crate::domain::pgmq::PgmqQueue;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::pgmq::MessageId;
use diesel_models::query::pgmq;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait DeadLetterInterface {
    /// Moves exhausted messages out of `queue` into the dead-letter table, atomically:
    /// a message is never deleted from pgmq unless its dead-letter row is committed.
    async fn dead_letter_messages(
        &self,
        queue: PgmqQueue,
        entries: Vec<DeadLetterMessageNew>,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl DeadLetterInterface for Store {
    async fn dead_letter_messages(
        &self,
        queue: PgmqQueue,
        entries: Vec<DeadLetterMessageNew>,
    ) -> StoreResult<()> {
        let ids: Vec<MessageId> = entries.iter().map(|e| MessageId(e.pgmq_msg_id)).collect();
        let rows: Vec<_> = entries.into_iter().map(Into::into).collect();

        self.transaction(|conn| {
            async move {
                diesel_models::dead_letter::DeadLetterMessageRowNew::insert_batch(conn, &rows)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                pgmq::delete(conn, queue.as_str(), &ids)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
            }
            .scope_boxed()
        })
        .await
    }
}

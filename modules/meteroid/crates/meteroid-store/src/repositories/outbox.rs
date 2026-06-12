use crate::StoreResult;
use crate::domain::outbox_event;
use crate::domain::pgmq::PgmqQueue;
use crate::errors::StoreError;
use crate::store::{PgConn, StoreInternal};
use diesel_models::query::pgmq;
use error_stack::Report;

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

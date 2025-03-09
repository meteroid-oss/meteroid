use super::{FailedRecord, Sink};
use crate::ingest::domain::ProcessedEvent;
use crate::ingest::errors::IngestError;
use crate::ingest::metrics::{INGEST_BATCH_SIZE, INGESTED_EVENTS_TOTAL};
use async_trait::async_trait;
pub use opentelemetry::KeyValue;
use tracing::info;

pub struct PrintSink {}

#[async_trait]
impl Sink for PrintSink {
    async fn send(
        &self,
        events: Vec<ProcessedEvent>,
        attributes: &[KeyValue],
    ) -> Result<Vec<FailedRecord>, IngestError> {
        let span = tracing::span!(tracing::Level::INFO, "batch of events");
        let _enter = span.enter();

        INGEST_BATCH_SIZE.record(events.len() as u64, attributes);
        INGESTED_EVENTS_TOTAL.add(events.len() as u64, attributes);

        info!("Ingested batch: {}", events.len());

        Ok(Vec::new())
    }
}

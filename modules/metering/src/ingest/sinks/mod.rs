use crate::ingest::domain::ProcessedEvent;
use crate::ingest::errors::IngestError;
use opentelemetry::KeyValue;
use tonic::async_trait;

#[cfg(feature = "kafka")]
pub mod kafka;
pub mod print;

pub struct FailedRecord {
    pub event: ProcessedEvent,
    pub error: IngestError,
}

#[async_trait]
pub trait Sink {
    async fn send(
        &self,
        events: Vec<ProcessedEvent>,
        attributes: &[KeyValue],
    ) -> Result<Vec<FailedRecord>, IngestError>;
}

use crate::ingest::domain::RawEvent;
use crate::ingest::errors::IngestError;
use opentelemetry::KeyValue;
use tonic::async_trait;

#[cfg(feature = "kafka")]
pub mod kafka;
pub mod print;

pub struct FailedRecord {
    pub event: RawEvent,
    pub error: IngestError,
}

#[async_trait]
pub trait Sink {
    async fn send(
        &self,
        events: Vec<RawEvent>,
        attributes: &[KeyValue],
    ) -> Result<Vec<FailedRecord>, IngestError>;
}

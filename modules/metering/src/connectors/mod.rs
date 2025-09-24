mod errors;

pub mod clickhouse;
pub mod json;

use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, QueryRawEventsParams, QueryRawEventsResult, Usage};
use error_stack::Result;

use tonic::async_trait;

#[async_trait]
pub trait Connector {
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError>;

    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError>;

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, ConnectorError>;
}

pub struct PrintConnector {}

#[async_trait]
impl Connector for PrintConnector {
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError> {
        println!("Registering meter: {:?}", meter);
        Ok(())
    }

    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError> {
        println!("Querying meter: {:?}", params);
        Ok(vec![])
    }

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, ConnectorError> {
        println!("Querying raw events: {:?}", params);
        Ok(QueryRawEventsResult {
            events: vec![],
            total_count: 0,
            has_more: false,
        })
    }
}

pub(crate) mod errors;

pub mod clickhouse;
pub mod json;

use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, QueryRawEventsParams, QueryRawEventsResult, Usage};
use error_stack::Report;

use tonic::async_trait;

#[async_trait]
pub trait Connector {
    async fn register_meter(&self, meter: Meter) -> Result<(), Report<ConnectorError>>;

    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>>;

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>>;
}

pub struct PrintConnector {}

#[async_trait]
impl Connector for PrintConnector {
    async fn register_meter(&self, meter: Meter) -> Result<(), Report<ConnectorError>> {
        println!("Registering meter: {:?}", meter);
        Ok(())
    }

    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>> {
        println!("Querying meter: {:?}", params);
        Ok(vec![])
    }

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>> {
        println!("Querying raw events: {:?}", params);
        Ok(QueryRawEventsResult { events: vec![] })
    }
}

mod errors;

pub mod clickhouse;
pub mod json;

use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, Usage};
use error_stack::Result;

use tonic::async_trait;

#[async_trait]
pub trait Connector {
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError>;

    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError>;
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
}

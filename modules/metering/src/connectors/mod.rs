pub mod clickhouse;
mod errors;

use crate::connectors::errors::ConnectorError;
use crate::domain::{Meter, QueryMeterParams, Usage};
use error_stack::Result;

use tonic::async_trait;

#[async_trait]
pub trait Connector {
    async fn register_meter(&self, meter: Meter) -> Result<(), ConnectorError>;

    //
    async fn query_meter(&self, params: QueryMeterParams) -> Result<Vec<Usage>, ConnectorError>;
}

use crate::connectors::errors::ConnectorError;
use crate::domain::QueryMeterParams;
use clickhouse::Client;
use error_stack::Report;
use std::sync::Arc;

#[cfg(feature = "openstack")]
pub mod openstack_ext;

#[async_trait::async_trait]
pub trait ConnectorClickhouseExtension {
    fn prefix(&self) -> String;
    async fn init(&self, client: Arc<Client>) -> Result<(), Report<ConnectorError>>;

    // we don't yet need postprocessing, but we can refactor to run_query(pool, params) if we have this use case
    fn build_query(&self, params: &QueryMeterParams) -> Option<String>;
}

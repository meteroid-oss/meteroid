use crate::connectors::clickhouse::sql::SafeQuery;
use crate::connectors::errors::ConnectorError;
use crate::domain::QueryMeterParams;
use clickhouse::Client;
use error_stack::Report;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait ConnectorClickhouseExtension {
    fn prefix(&self) -> String;
    async fn init(&self, client: Arc<Client>) -> Result<(), Report<ConnectorError>>;

    /// `Ok(None)` when the code is not one the extension aggregates; `Err` when it is but the
    /// query cannot be served, so it never falls back to the generic SQL.
    fn build_query(&self, params: &QueryMeterParams) -> Result<Option<SafeQuery>, String>;
}

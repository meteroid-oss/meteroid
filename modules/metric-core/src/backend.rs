use crate::domain::Usage;
use crate::errors::ConnectorError;
use crate::metric::{MetricQuery, MetricRegistry};
use error_stack::Report;

/// Compile a `MetricQuery` against a `MetricRegistry` and run it. `metering` holds this as a trait object; `analytics` provides the ClickHouse impl.
#[async_trait::async_trait]
pub trait MetricBackend: Send + Sync {
    async fn query_metric(
        &self,
        registry: &MetricRegistry,
        query: &MetricQuery,
    ) -> Result<Vec<Usage>, Report<ConnectorError>>;
}

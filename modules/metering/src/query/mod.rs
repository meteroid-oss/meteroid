use crate::connectors::Connector;
use crate::query::service::UsageQueryService;
use metering_grpc::meteroid::metering::v1::usage_query_service_server::UsageQueryServiceServer;
use std::sync::Arc;

pub mod service;

pub fn service(
    connector: Arc<dyn Connector + Send + Sync>,
) -> UsageQueryServiceServer<UsageQueryService> {
    let inner = UsageQueryService::new(connector);
    UsageQueryServiceServer::new(inner)
}

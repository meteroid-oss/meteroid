use crate::connectors::Connector;
use crate::meters::service::MetersService;
use metering_grpc::meteroid::metering::v1::meters_service_server::MetersServiceServer;
use std::sync::Arc;

pub mod service;

pub fn service(connector: Arc<dyn Connector + Send + Sync>) -> MetersServiceServer<MetersService> {
    let inner = MetersService::new(connector);
    MetersServiceServer::new(inner)
}

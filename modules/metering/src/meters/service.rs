use metering_grpc::meteroid::metering::v1::meters_service_server::MetersService as MetersServiceGrpc;
use std::sync::Arc;

use metering_grpc::meteroid::metering::v1::{
    RegisterMeterRequest, RegisterMeterResponse, UnregisterMeterRequest, UnregisterMeterResponse,
};
use tonic::{Request, Response, Status};

use crate::connectors::Connector;

#[derive(Clone)]
pub struct MetersService {
    pub connector: Arc<dyn Connector + Send + Sync>,
}

impl MetersService {
    pub fn new(connector: Arc<dyn Connector + Send + Sync>) -> Self {
        MetersService { connector }
    }
}

#[tonic::async_trait]
impl MetersServiceGrpc for MetersService {
    #[tracing::instrument(skip_all)]
    async fn register_meter(
        &self,
        _request: Request<RegisterMeterRequest>,
    ) -> Result<Response<RegisterMeterResponse>, Status> {
        unimplemented!()
    }

    #[tracing::instrument(skip_all)]
    async fn unregister_meter(
        &self,
        _request: Request<UnregisterMeterRequest>,
    ) -> Result<Response<UnregisterMeterResponse>, Status> {
        unimplemented!()
    }
}

use metering_grpc::meteroid::metering::v1::meters_service_server::MetersService as MetersServiceGrpc;
use std::sync::Arc;

use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::{
    RegisterMeterRequest, RegisterMeterResponse, UnregisterMeterRequest, UnregisterMeterResponse,
};
use tonic::{Request, Response, Status};

use crate::connectors::Connector;
use crate::domain::Meter;

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
        request: Request<RegisterMeterRequest>,
    ) -> Result<Response<RegisterMeterResponse>, Status> {
        let req = request.into_inner();

        let meter = req
            .meter
            .ok_or_else(|| Status::invalid_argument("No meter provided"))?;

        let aggregation_type: AggregationType = meter
            .aggregation
            .try_into()
            .map_err(|_| Status::internal("unknown aggregation_type"))?;

        let meter_aggregation = aggregation_type.into();

        let meter = Meter {
            aggregation: meter_aggregation,
            namespace: req.tenant_id,
            id: meter.id,
            event_name: meter.event_name,
            value_property: meter.aggregation_key,
            group_by: meter.dimensions,
        };

        self.connector.register_meter(meter).await.map_err(|e| {
            Status::internal("Failed to register meter")
                .set_source(Arc::new(e.into_error()))
                .clone()
        })?;

        Ok(Response::new(RegisterMeterResponse { metadata: vec![] }))
    }

    #[tracing::instrument(skip_all)]
    async fn unregister_meter(
        &self,
        _request: Request<UnregisterMeterRequest>,
    ) -> Result<Response<UnregisterMeterResponse>, Status> {
        unimplemented!()
    }
}

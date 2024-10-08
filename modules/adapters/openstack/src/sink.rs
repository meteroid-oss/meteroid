use crate::config::Config;
use common_grpc::middleware::client::{build_api_layered_client_service, LayeredApiClientService};
use metering_grpc::meteroid::metering::v1::events_service_client::EventsServiceClient;
use tonic::transport::Channel;

pub struct MeteroidSink {
    pub client: EventsServiceClient<LayeredApiClientService>,
}

impl MeteroidSink {
    pub fn new(config: &Config) -> Self {
        let channel = Channel::from_shared(config.metering_endpoint.clone())
            .expect("Invalid ingest endpoint")
            .connect_lazy();

        let service = build_api_layered_client_service(channel, &config.api_key);

        let client = EventsServiceClient::new(service);

        MeteroidSink { client }
    }
}

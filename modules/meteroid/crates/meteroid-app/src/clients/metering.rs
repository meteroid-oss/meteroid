use tonic::transport::Channel;

use crate::config::Config;
use common_config::auth::InternalAuthConfig;
use common_grpc::middleware::client::build_layered_client_service;

use crate::clients::usage::MeteringUsageClient;
use metering_grpc::meteroid::metering::v1::internal_events_service_client::InternalEventsServiceClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

impl MeteringUsageClient {
    pub fn from_channel(channel: Channel, auth_config: &InternalAuthConfig) -> MeteringUsageClient {
        let service = build_layered_client_service(channel, auth_config);
        Self::new(
            UsageQueryServiceClient::new(service.clone()),
            InternalEventsServiceClient::new(service),
        )
    }

    /// Connects lazily: built once per process and shared, its clones share the channel.
    pub fn connect(config: &Config) -> Self {
        let channel = Channel::from_shared(config.metering_endpoint.clone())
            .expect("Invalid metering_endpoint")
            .connect_lazy();
        Self::from_channel(channel, &config.internal_auth)
    }
}

use std::sync::OnceLock;

use tonic::transport::Channel;

use crate::config::Config;
use common_config::auth::InternalAuthConfig;
use common_grpc::middleware::client::build_layered_client_service;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;

use crate::clients::usage::MeteringUsageClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

static METERING_CLIENT: OnceLock<MeteringUsageClient> = OnceLock::new();

impl MeteringUsageClient {
    pub fn from_channel(channel: Channel, auth_config: &InternalAuthConfig) -> MeteringUsageClient {
        let service = build_layered_client_service(channel, auth_config);

        Self::new(
            UsageQueryServiceClient::new(service.clone()),
            MetersServiceClient::new(service.clone()),
        )
    }

    pub fn get() -> &'static Self {
        METERING_CLIENT.get_or_init(|| {
            let config = Config::get();

            let channel = Channel::from_shared(config.metering_endpoint.clone())
                .expect("Invalid metering_endpoint")
                .connect_lazy();

            Self::from_channel(channel, &config.internal_auth)
        })
    }
}

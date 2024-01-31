use std::sync::OnceLock;

use tonic::transport::Channel;

use crate::config::Config;
use common_config::auth::InternalAuthConfig;
use common_grpc::middleware::client::{build_layered_client_service, LayeredClientService};

use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

static METERING_CLIENT: OnceLock<MeteringClient> = OnceLock::new();

#[derive(Clone)]
pub struct MeteringClient {
    // pub meters: MetersServiceClient<LayeredClientService>,
    // pub events: EventsServiceClient<LayeredClientService>,
    pub queries: UsageQueryServiceClient<LayeredClientService>,
}

impl MeteringClient {
    pub fn from_channel(channel: Channel, auth_config: &InternalAuthConfig) -> MeteringClient {
        let service = build_layered_client_service(channel, auth_config);

        Self {
            // meters: MetersServiceClient::new(service.clone()),
            // events: EventsServiceClient::new(service.clone()),
            queries: UsageQueryServiceClient::new(service.clone()),
        }
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

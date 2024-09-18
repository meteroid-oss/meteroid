use tonic::transport::Channel;

use common_config::auth::InternalAuthConfig;

use metering_grpc::meteroid::metering::v1::events_service_client::EventsServiceClient;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

use common_grpc::middleware::client::auth::create_api_auth_layer;
use common_grpc::middleware::client::auth::{
    create_admin_auth_layer, AdminAuthService, ApiAuthService,
};

pub type TestLayeredClientService = AdminAuthService<ApiAuthService<Channel>>;

pub struct AllClients {
    pub _meters: MetersServiceClient<TestLayeredClientService>,
    pub events: EventsServiceClient<TestLayeredClientService>,
    pub _usage: UsageQueryServiceClient<TestLayeredClientService>,
}

impl AllClients {
    pub fn from_channel(
        channel: Channel,
        api_token: &str,
        auth_config: &InternalAuthConfig,
    ) -> AllClients {
        let service = Self::build_layered_client_service(channel, api_token, &auth_config);

        Self {
            _meters: MetersServiceClient::new(service.clone()),
            events: EventsServiceClient::new(service.clone()),
            _usage: UsageQueryServiceClient::new(service.clone()),
        }
    }

    pub fn build_layered_client_service(
        channel: Channel,
        api_token: &str,
        auth_config: &InternalAuthConfig,
    ) -> TestLayeredClientService {
        tower::ServiceBuilder::new()
            .layer(create_admin_auth_layer(auth_config))
            .layer(create_api_auth_layer(api_token.to_string()))
            .service(channel)
    }
}

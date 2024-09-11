use tonic::transport::Channel;
use tonic_tracing_opentelemetry::middleware::client::{OtelGrpcLayer, OtelGrpcService};

use common_config::auth::InternalAuthConfig;

pub mod auth;
pub mod metric;

pub type LayeredClientService =
    OtelGrpcService<metric::MetricService<auth::AdminAuthService<Channel>>>;

pub fn build_layered_client_service(
    channel: Channel,
    auth_config: &InternalAuthConfig,
) -> LayeredClientService {
    tower::ServiceBuilder::new()
        .layer(OtelGrpcLayer) // note: should be last .. but fails to compile
        .layer(metric::create())
        .layer(auth::create_admin_auth_layer(auth_config))
        .service(channel)
}

pub type LayeredApiClientService =
    OtelGrpcService<metric::MetricService<auth::ApiAuthService<Channel>>>;

pub fn build_api_layered_client_service(
    channel: Channel,
    api_key: &str,
) -> LayeredApiClientService {
    tower::ServiceBuilder::new()
        .layer(OtelGrpcLayer) // note: should be last .. but fails to compile
        .layer(metric::create())
        .layer(auth::create_api_auth_layer(api_key.to_string()))
        .service(channel)
}

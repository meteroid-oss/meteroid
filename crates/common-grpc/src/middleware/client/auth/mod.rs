pub use admin_layer::AdminAuthLayer;
pub use admin_layer::AdminAuthService;
pub use api_layer::ApiAuthLayer;
pub use api_layer::ApiAuthService;
use common_config::auth::InternalAuthConfig;

mod admin_layer;
mod api_layer;

pub fn create_admin_auth_layer(config: &InternalAuthConfig) -> AdminAuthLayer {
    AdminAuthLayer::new(config)
}

pub fn create_api_auth_layer(api_key: String) -> ApiAuthLayer {
    ApiAuthLayer::new(api_key)
}

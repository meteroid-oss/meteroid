use secrecy::SecretString;

pub use api_layer::ApiAuthLayer;
pub use api_layer::ApiAuthMiddleware;
use meteroid_store::Store;
pub use support::{SharedSupportGrantReader, SupportGrantMode, SupportGrantReader};

mod api_layer;
pub mod rbac;
pub mod strategies;
mod support;

pub fn create(
    jwt_secret: SecretString,
    store: Store,
    support: Option<SharedSupportGrantReader>,
) -> ApiAuthLayer {
    ApiAuthLayer::new(jwt_secret, store, support)
}

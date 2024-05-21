use secrecy::SecretString;

pub use api_layer::ApiAuthLayer;
pub use api_layer::ApiAuthMiddleware;
use meteroid_store::Store;

mod api_layer;
pub mod strategies;

pub fn create(jwt_secret: SecretString, store: Store) -> ApiAuthLayer {
    ApiAuthLayer::new(jwt_secret, store)
}

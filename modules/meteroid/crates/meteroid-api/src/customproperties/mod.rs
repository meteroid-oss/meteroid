use meteroid_grpc::meteroid::api::customproperties::v1::custom_properties_service_server::CustomPropertiesServiceServer;
use meteroid_store::Store;

pub use meteroid_grpc_mapping::customproperties::{error, mapping};
mod service;

#[derive(Clone)]
pub struct CustomPropertiesServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> CustomPropertiesServiceServer<CustomPropertiesServiceComponents> {
    CustomPropertiesServiceServer::new(CustomPropertiesServiceComponents { store })
}

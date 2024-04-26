use meteroid_grpc::meteroid::api::users::v1::users_service_server::UsersServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct UsersServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> UsersServiceServer<UsersServiceComponents> {
    let inner = UsersServiceComponents { store };
    UsersServiceServer::new(inner)
}

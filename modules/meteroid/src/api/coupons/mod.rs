use meteroid_grpc::meteroid::api::coupons::v1::coupons_service_server::CouponsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct CouponsServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> CouponsServiceServer<CouponsServiceComponents> {
    let inner = CouponsServiceComponents { store };
    CouponsServiceServer::new(inner)
}

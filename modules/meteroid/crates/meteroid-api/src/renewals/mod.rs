//! Renewal notices (R-157, R-161): the tenant's policy and a subscription's next renewal.
use meteroid_grpc::meteroid::api::renewals::v1::renewals_service_server::RenewalsServiceServer;
use meteroid_store::Store;

mod service;

#[derive(Clone)]
pub struct RenewalsServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> RenewalsServiceServer<RenewalsServiceComponents> {
    RenewalsServiceServer::new(RenewalsServiceComponents { store })
}

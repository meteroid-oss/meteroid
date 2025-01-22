use meteroid_grpc::meteroid::api::bankaccounts::v1::bank_accounts_service_server::BankAccountsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct BankAccountsServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> BankAccountsServiceServer<BankAccountsServiceComponents> {
    let inner = BankAccountsServiceComponents { store };
    BankAccountsServiceServer::new(inner)
}

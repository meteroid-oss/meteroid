use meteroid_grpc::meteroid::api::listviews::v1::list_views_service_server::ListViewsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct ListViewsServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> ListViewsServiceServer<ListViewsServiceComponents> {
    let inner = ListViewsServiceComponents { store };
    ListViewsServiceServer::new(inner)
}

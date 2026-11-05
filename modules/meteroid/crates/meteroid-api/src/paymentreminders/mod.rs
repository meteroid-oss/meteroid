use std::sync::Arc;

use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::paymentreminders::v1::payment_reminders_service_server::PaymentRemindersServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

#[derive(Clone)]
pub struct PaymentRemindersServiceComponents {
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> PaymentRemindersServiceServer<PaymentRemindersServiceComponents> {
    PaymentRemindersServiceServer::new(PaymentRemindersServiceComponents {
        store,
        feature_resolver,
    })
}

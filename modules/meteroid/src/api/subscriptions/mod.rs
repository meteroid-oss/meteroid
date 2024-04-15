use crate::compute::InvoiceEngine;

use crate::eventbus::{Event, EventBus};

use meteroid_grpc::meteroid::api::subscriptions::v1_2::subscriptions_service_server::SubscriptionsServiceServer;

use meteroid_store::Store;
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

mod error;
mod mapping;
mod service;

pub struct SubscriptionServiceComponents {
    pub store: Store,
    pub compute_service: Arc<InvoiceEngine>,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

pub fn service(
    store: Store,
    subscription_billing: Arc<InvoiceEngine>,
    eventbus: Arc<dyn EventBus<Event>>,
) -> SubscriptionsServiceServer<SubscriptionServiceComponents> {
    let inner = SubscriptionServiceComponents {
        store,
        compute_service: subscription_billing,
        eventbus,
    };
    SubscriptionsServiceServer::new(inner)
}

#[derive(Debug)]
struct ErrorWrapper {
    inner: anyhow::Error,
}

impl Display for ErrorWrapper {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.inner, f)
    }
}

impl Error for ErrorWrapper {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

impl From<anyhow::Error> for ErrorWrapper {
    fn from(error: anyhow::Error) -> Self {
        ErrorWrapper { inner: error }
    }
}

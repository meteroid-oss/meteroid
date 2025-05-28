use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsServiceServer;

use meteroid_store::{Services, Store};
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};

pub mod error;
pub(crate) mod mapping;

pub use mapping::ext;

mod service;

pub struct SubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(
    store: Store,
    services: Services,
) -> SubscriptionsServiceServer<SubscriptionServiceComponents> {
    let inner = SubscriptionServiceComponents { store, services };
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

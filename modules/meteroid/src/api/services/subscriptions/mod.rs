use crate::compute::InvoiceEngine;
use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};
use deadpool_postgres::{Object, Transaction};
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsServiceServer;
use meteroid_repository::Pool;
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use tonic::Status;

mod mapping;
mod service;

pub struct SubscriptionServiceComponents {
    pub pool: Pool,
    pub compute_service: Arc<InvoiceEngine>,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl SubscriptionServiceComponents {
    pub async fn get_connection(&self) -> Result<Object, Status> {
        get_connection(&self.pool).await
    }
    pub async fn get_transaction<'a>(
        &'a self,
        client: &'a mut Object,
    ) -> Result<Transaction<'a>, Status> {
        get_transaction(client).await
    }
}

pub fn service(
    pool: Pool,
    subscription_billing: Arc<InvoiceEngine>,
    eventbus: Arc<dyn EventBus<Event>>,
) -> SubscriptionsServiceServer<SubscriptionServiceComponents> {
    let inner = SubscriptionServiceComponents {
        pool,
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

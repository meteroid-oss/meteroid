use crate::services::clients::usage::UsageClient;
use crate::{Store, StoreResult};
use std::sync::Arc;
use svix::api::Svix;

// mod billing_worker;
pub mod utils;

pub mod clients;
mod connectors;
mod credits;
mod edge;
pub mod invoice_lines;
mod invoices;
mod lifecycle;
mod orchestration;
mod payment;
mod subscriptions;
mod webhooks;

use crate::errors::StoreError;
pub use invoices::{CustomerDetailsUpdate, InvoiceBillingMode};
use stripe_client::client::StripeClient;
pub use subscriptions::insert::payment_method::PaymentSetupResult;

// INTERNAL. Share connections
#[derive(Clone)]
struct Services {
    store: Arc<Store>,
    usage_client: Arc<dyn UsageClient>,
    svix: Option<Arc<Svix>>,
    pub(crate) stripe: Arc<StripeClient>,
}

impl Services {
    pub(crate) fn svix(&self) -> StoreResult<Arc<Svix>> {
        self.svix
            .clone()
            .ok_or(StoreError::InitializationError("svix client config".into()).into())
    }
}

// EXTERNAL. Flat api, to be used in apis and workers.
#[derive(Clone)]
pub struct ServicesEdge {
    store: Arc<Store>,
    services: Services,
}

impl ServicesEdge {
    pub fn new(
        store: Arc<Store>,
        usage_client: Arc<dyn UsageClient>,
        svix: Option<Arc<Svix>>,
        stripe: Arc<StripeClient>,
    ) -> Self {
        Self {
            services: Services {
                store: store.clone(),
                usage_client,
                svix,
                stripe,
            },
            store,
        }
    }

    pub fn usage_clients(&self) -> Arc<dyn UsageClient> {
        self.services.usage_client.clone()
    }
}

use crate::Store;
use crate::services::clients::usage::UsageClient;
use std::sync::Arc;

// mod billing_worker;
pub mod utils;

pub mod clients;
mod credits;
mod edge;
mod invoice_lines;
mod invoices;
mod lifecycle;
mod payment;
mod subscriptions;

pub use invoices::InvoiceBillingMode;
pub use subscriptions::insert::payment_method::PaymentSetupResult;

// INTERNAL. Share connections
#[derive(Clone)]
struct Services {
    store: Arc<Store>,
    usage_client: Arc<dyn UsageClient>,
}

// EXTERNAL. Flat api, to be used in apis and workers.
#[derive(Clone)]
pub struct ServicesEdge {
    store: Arc<Store>,
    services: Services,
}

impl ServicesEdge {
    pub fn new(store: Arc<Store>, usage_client: Arc<dyn UsageClient>) -> Self {
        Self {
            services: Services {
                store: store.clone(),
                usage_client,
            },
            store,
        }
    }
}

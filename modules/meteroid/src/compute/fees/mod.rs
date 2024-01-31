use crate::compute::{InvoiceEngine, PriceComponent, SubscriptionDetails};
use crate::models::{InvoiceLine, InvoiceLinePeriod};
use anyhow::Error;
use async_trait::async_trait;

pub mod capacity;
pub mod onetime;
pub mod rate;
pub mod recurring;
pub mod shared;
pub mod slots;
pub mod usage_based;

pub trait PeriodCalculator {
    fn applies_this_period(&self, subscription: &SubscriptionDetails) -> Result<bool, Error>;
    fn period(&self, subscription: &SubscriptionDetails) -> Result<InvoiceLinePeriod, Error>;
}

pub trait ComputeInvoiceLine {
    fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        override_units: Option<u64>,
    ) -> Result<Option<InvoiceLine>, Error>;
}

#[async_trait]
pub trait ComputeInvoiceLineWithUsage {
    async fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        clients: &InvoiceEngine,
    ) -> Result<Option<InvoiceLine>, Error>;
}

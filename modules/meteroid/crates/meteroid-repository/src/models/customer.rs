use crate::models::stripe::InvoiceCollectionMethod;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct StripeConfig {
    pub customer_id: String,
    pub invoice_collection_method: InvoiceCollectionMethod,
}

#[derive(Deserialize, Serialize)]
pub enum BillingConfig {
    Manual,
    Stripe(StripeConfig),
}

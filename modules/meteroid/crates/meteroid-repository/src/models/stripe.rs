use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub enum InvoiceCollectionMethod {
    ChargeAutomatically,
    SendInvoice,
}

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub struct InvoiceLines {
    pub total: i64,
    pub lines: Vec<InvoiceLine>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub struct InvoiceLine {
    pub name: String,
    pub total: i64,
    pub quantity: Option<u64>,
    // TODO drop the precision 8 ?
    pub unit_price: Option<f64>,
    pub sub_lines: Vec<InvoiceSubLine>,
    pub metadata: Option<InvoiceLineMetadata>,
    // TODO required
    pub period: Option<InvoiceLinePeriod>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub struct InvoiceLineMetadata {
    pub product_id: String,
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub struct InvoiceLinePeriod {
    pub from: chrono::NaiveDate,
    // incl
    pub to: chrono::NaiveDate, // incl TODO check
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub struct InvoiceSubLine {
    pub name: String,
    pub subtotal: i64,
    pub metadata: InvoiceSubLineMetadata,
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub enum InvoiceSubLineMetadata {
    Recurring {
        charge_id: String,
        quantity: u32,
    },
    Usage {
        billable_metric_id: String,
        total_usage: Decimal,
        usage_details: Vec<UsageDetails>,
    },
    // matrix usage
}

#[derive(PartialEq, Debug, Clone, Deserialize, Serialize)]
pub struct UsageDetails {
    pub date: NaiveDate,
    pub usage: Decimal,
}

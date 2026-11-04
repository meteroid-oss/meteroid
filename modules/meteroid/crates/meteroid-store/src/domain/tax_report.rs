//! The per-invoice tax transaction trail over finalized invoices: one row per issued invoice,
//! in minor units of the invoice's own currency. The streaming CSV export emits it.

use chrono::NaiveDate;

use crate::domain::invoices::{Invoice, TaxExemptionType};

/// The per-invoice audit trail the tax reports derive from.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxTransactionRow {
    pub invoice_number: String,
    pub invoice_date: NaiveDate,
    pub customer_name: String,
    pub vat_number: Option<String>,
    pub country: Option<String>,
    pub currency: String,
    pub net_amount: i64,
    pub tax_amount: i64,
    pub gross_amount: i64,
    pub status: String,
    /// The reverse-charge / exemption reason when the invoice carried one.
    pub exemption: Option<String>,
}

/// Audit-trail vocabulary. These strings reach closed accounting periods through the
/// CSV export and the transactions API, so they are a stable wire format: `NotRegistered`
/// keeps emitting `not_registered` because saved filters and past exports match on it.
fn exemption_str(e: &TaxExemptionType) -> &'static str {
    match e {
        TaxExemptionType::ReverseCharge => "reverse_charge",
        TaxExemptionType::TaxExempt => "tax_exempt",
        TaxExemptionType::NotRegistered => "not_registered",
        TaxExemptionType::Export => "export",
        TaxExemptionType::NoVatTerritory => "no_vat_territory",
    }
}

/// The customer's billing country, as the invoice snapshotted it.
pub fn country_of(inv: &Invoice) -> Option<String> {
    inv.customer_details
        .billing_address
        .as_ref()
        .and_then(|a| a.country.as_ref())
        .map(|c| c.to_string())
}

/// The audit-trail line the reports, the streaming CSV export and the paginated transactions
/// API all emit, built from one invoice so a streamed page needs no full report.
pub fn transaction_row(inv: &Invoice) -> TaxTransactionRow {
    // The dominant exemption: the first breakdown line that carries one.
    let exemption = inv
        .tax_breakdown
        .iter()
        .find_map(|item| item.exemption_type.as_ref())
        .map(|e| exemption_str(e).to_string());

    TaxTransactionRow {
        invoice_number: inv.invoice_number.clone(),
        invoice_date: inv.invoice_date,
        customer_name: inv.customer_details.name.clone(),
        vat_number: inv.customer_details.vat_number.clone(),
        country: country_of(inv),
        currency: inv.currency.clone(),
        net_amount: inv.subtotal,
        tax_amount: inv.tax_amount,
        gross_amount: inv.total,
        status: format!("{:?}", inv.status).to_lowercase(),
        exemption,
    }
}

//! Bridging the e-invoicing library to what we store about a document.
//!
//! Both renderers — invoices and credit notes — ask the same two questions:
//! which format has this entity opted into, and what do I record when the
//! document cannot be produced in it. Neither question is document-specific.

use meteroid_invoicing::einvoicing::{EInvoiceFormat, EInvoiceRejection};
use meteroid_store::domain::einvoicing::{EInvoicingFinding, EInvoicingOutcome};
use meteroid_store::domain::invoicing_entities::EInvoicingFormat as DomainFormat;

pub fn map_format(format: DomainFormat) -> EInvoiceFormat {
    match format {
        DomainFormat::FacturX => EInvoiceFormat::FacturX,
        DomainFormat::XRechnung => EInvoiceFormat::XRechnung,
        DomainFormat::PeppolBis => EInvoiceFormat::PeppolBis,
    }
}

/// A rejection, as it is stored against the document that could not carry it.
///
/// The findings are kept in the standard's own vocabulary rather than
/// translated: `BR-11 at BG-8/BT-55` is what an operator looks up, and what
/// names the field to fill on the *next* invoice — a finalized one is frozen.
pub fn rejection_outcome(
    format: EInvoiceFormat,
    rejection: &EInvoiceRejection,
) -> EInvoicingOutcome {
    EInvoicingOutcome::failed(
        format.profile_name(),
        rejection
            .findings()
            .iter()
            .map(|finding| EInvoicingFinding {
                rule: finding.rule.clone(),
                term: finding.term.clone(),
                message: finding.message.clone(),
                hint: finding.hint.clone(),
            })
            .collect(),
        rejection.error().map(str::to_string),
    )
}

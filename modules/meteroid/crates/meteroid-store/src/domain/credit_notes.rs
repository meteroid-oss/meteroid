use super::einvoicing::{EInvoicingReport, EInvoicingStatus};
use super::enums::{CreditNoteOrigin, CreditNoteStatus, CreditType};
use super::invoice_lines::LineItem;
use super::invoices::{InlineCustomer, InlineInvoicingEntity, TaxBreakdownItem};
use crate::domain::Customer;
use crate::domain::connectors::ConnectionMeta;
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CreditNoteId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId,
    StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::credit_notes::{CreditNoteRow, CreditNoteRowNew};
use o2o::o2o;

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(CreditNoteRow, StoreErrorReport)]
pub struct CreditNote {
    pub id: CreditNoteId,
    pub credit_note_number: String,
    #[from(~.into())]
    pub status: CreditNoteStatus,
    #[from(~.into())]
    pub credit_type: CreditType,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub finalized_at: Option<NaiveDateTime>,
    pub voided_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub plan_version_id: Option<PlanVersionId>,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    /// The invoice debt the note cancelled, apart from cash refunded and credit returned.
    pub debt_cancelled_amount_cents: i64,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub pdf_document_id: Option<StoredDocumentId>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    pub invoicing_entity_id: InvoicingEntityId,
    /// User-defined property values, keyed by definition `key`.
    pub custom_properties: serde_json::Value,
    pub xml_document_id: Option<StoredDocumentId>,
    /// Whether the structured e-invoice was produced; see `Invoice`.
    #[from(~.map(| v | v.into()))]
    pub einvoicing_status: Option<EInvoicingStatus>,
    #[from(~.and_then(| v | serde_json::from_value(v).ok()))]
    pub einvoicing_report: Option<EInvoicingReport>,
    #[from(~.into())]
    pub origin: CreditNoteOrigin,
}

/// What separates a note's subtotal from its taxed base, positive: the discount it reverses and
/// the promotional credit, as the document states them (subtotal - both + tax = total, in
/// magnitude).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CreditNoteReductions {
    pub discount: i64,
    pub promotional_credit: i64,
}

impl CreditNote {
    /// `invoice_lines` are the credited invoice's: a tax-inclusive line's promotional credit is
    /// stated as its net share, which only the source line gives. Without them, the drawn share.
    pub fn reductions(&self, invoice_lines: Option<&[LineItem]>) -> CreditNoteReductions {
        note_reductions(&self.line_items, invoice_lines)
    }

    /// The promotional credit finalization gives back to the grants, as drawn (tax included on a
    /// tax-inclusive line).
    pub fn promotional_credit_restored(&self) -> i64 {
        -self
            .line_items
            .iter()
            .filter_map(|line| line.promotional_credit)
            .sum::<i64>()
    }
}

fn note_reductions(
    note_lines: &[LineItem],
    invoice_lines: Option<&[LineItem]>,
) -> CreditNoteReductions {
    let (reduction, promotional) =
        note_lines
            .iter()
            .fold((0i64, 0i64), |(reduction, promotional), line| {
                let source = invoice_lines
                    .and_then(|lines| lines.iter().find(|source| source.local_id == line.local_id));
                (
                    reduction + line.taxable_amount - line.amount_subtotal,
                    promotional - line.note_net_promotional_credit(source),
                )
            });
    CreditNoteReductions {
        discount: reduction - promotional,
        promotional_credit: promotional,
    }
}

#[derive(Debug, Clone, o2o)]
#[owned_try_into(CreditNoteRowNew, StoreErrorReport)]
#[ghosts(id: {CreditNoteId::new()}, voids_invoice: {false})]
pub struct CreditNoteNew {
    pub credit_note_number: String,
    #[into(~.into())]
    pub status: CreditNoteStatus,
    #[into(~.into())]
    pub credit_type: CreditType,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub plan_version_id: Option<PlanVersionId>,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    /// The invoice debt the note cancelled, apart from cash refunded and credit returned.
    pub debt_cancelled_amount_cents: i64,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub invoicing_entity_id: InvoicingEntityId,
    /// Validated & normalized custom property values (never null; empty object when unmanaged).
    pub custom_properties: serde_json::Value,
    #[ghost({None})]
    pub finalized_at: Option<NaiveDateTime>,
    #[into(~.into())]
    pub origin: CreditNoteOrigin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailedCreditNote {
    pub credit_note: CreditNote,
    pub customer: Customer,
}

#[cfg(test)]
mod tests {
    use super::{CreditNoteReductions, note_reductions};
    use crate::domain::LineItem;

    fn line(subtotal: i64, taxable: i64, promotional: Option<i64>) -> LineItem {
        serde_json::from_value(serde_json::json!({
            "local_id": "l1",
            "name": "Onboarding workshop",
            "amount_subtotal": subtotal,
            "taxable_amount": taxable,
            "amount_total": taxable,
            "quantity": "1",
            "unit_price": "50",
            "start_date": "2026-10-17",
            "end_date": "2026-10-18",
            "sub_lines": [],
            "is_prorated": false,
            "promotional_credit": promotional,
        }))
        .unwrap()
    }

    /// Half of a 100.00 line that took 3.33 of promotional credit (taxed base 96.67): the note
    /// credits 50.00 down to 48.34, a 1.66 reduction that is all promotional credit, as the page
    /// states it, while 1.67 goes back to the grants.
    #[test]
    fn a_partial_note_states_no_phantom_discount() {
        let source = line(10_000, 9_667, Some(333));
        assert_eq!(
            note_reductions(
                &[line(-5_000, -4_834, Some(-167))],
                Some(std::slice::from_ref(&source))
            ),
            CreditNoteReductions {
                discount: 0,
                promotional_credit: 166
            }
        );
    }

    /// QA M8: the note reverses a 50.00 line that took 5.00 of promotional credit.
    #[test]
    fn a_note_states_its_promotional_credit_and_discount_apart() {
        assert_eq!(
            note_reductions(&[line(-5_000, -4_500, Some(-500))], None),
            CreditNoteReductions {
                discount: 0,
                promotional_credit: 500
            }
        );
        assert_eq!(
            note_reductions(&[line(-10_000, -7_500, Some(-500))], None),
            CreditNoteReductions {
                discount: 2_000,
                promotional_credit: 500
            }
        );
    }
}

use crate::creditnotes::model::CreditNote;
use crate::currencies;
use crate::invoices::mapping::{map_line_item_to_rest, map_tax_breakdown_to_rest};
use meteroid_app::errors::RestApiError;
use meteroid_store::domain;

pub fn domain_to_rest(d: domain::CreditNote) -> Result<CreditNote, RestApiError> {
    Ok(CreditNote {
        id: d.id,
        credit_note_number: d.credit_note_number,
        status: d.status.into(),
        credit_type: d.credit_type.into(),
        customer_id: d.customer_id,
        invoice_id: d.invoice_id,
        invoice_number: d.invoice_number,
        subscription_id: d.subscription_id,
        plan_version_id: d.plan_version_id,
        currency: currencies::mapping::from_str(d.currency.as_str())?,
        subtotal: d.subtotal,
        tax_amount: d.tax_amount,
        total: d.total,
        refunded_amount_cents: d.refunded_amount_cents,
        credited_amount_cents: d.credited_amount_cents,
        line_items: d
            .line_items
            .into_iter()
            .map(map_line_item_to_rest)
            .collect(),
        tax_breakdown: d
            .tax_breakdown
            .into_iter()
            .map(map_tax_breakdown_to_rest)
            .collect(),
        reason: d.reason,
        memo: d.memo,
        created_at: d.created_at.into(),
        updated_at: d.updated_at.map(Into::into),
        finalized_at: d.finalized_at.map(Into::into),
        voided_at: d.voided_at.map(Into::into),
        custom_properties: d.custom_properties,
    })
}

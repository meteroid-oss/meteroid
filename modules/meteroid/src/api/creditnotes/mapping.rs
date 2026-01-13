use crate::api::invoices::mapping as invoice_mapping;
use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
use meteroid_grpc::meteroid::api::creditnotes::v1;
use meteroid_store::domain::{CreditNote as DomainCreditNote, DetailedCreditNote, CreditNoteStatus};

pub fn credit_note_status_domain_to_server(
    status: CreditNoteStatus,
) -> v1::CreditNoteStatus {
    match status {
        CreditNoteStatus::Draft => v1::CreditNoteStatus::Draft,
        CreditNoteStatus::Finalized => v1::CreditNoteStatus::Finalized,
        CreditNoteStatus::Voided => v1::CreditNoteStatus::Voided,
    }
}

pub fn credit_note_status_server_to_domain(
    status: Option<i32>,
) -> Option<CreditNoteStatus> {
    status.and_then(|s| match v1::CreditNoteStatus::try_from(s).ok()? {
        v1::CreditNoteStatus::Draft => Some(CreditNoteStatus::Draft),
        v1::CreditNoteStatus::Finalized => {
            Some(CreditNoteStatus::Finalized)
        }
        v1::CreditNoteStatus::Voided => Some(CreditNoteStatus::Voided),
    })
}

pub fn domain_to_server(credit_note: DomainCreditNote, customer_name: String) -> v1::CreditNote {
    v1::CreditNote {
        id: credit_note.id.as_proto(),
        credit_note_number: credit_note.credit_note_number,
        status: credit_note_status_domain_to_server(credit_note.status) as i32,
        created_at: credit_note.created_at.as_proto(),
        tenant_id: credit_note.tenant_id.as_proto(),
        customer_id: credit_note.customer_id.as_proto(),
        customer_name,
        invoice_id: credit_note.invoice_id.as_proto(),
        currency: credit_note.currency,
        total: credit_note.total,
        credited_amount_cents: credit_note.credited_amount_cents,
        refunded_amount_cents: credit_note.refunded_amount_cents,
        finalized_at: credit_note.finalized_at.as_proto(),
    }
}

pub fn detailed_domain_to_server(
    detailed: DetailedCreditNote,
) -> Result<v1::DetailedCreditNote, tonic::Status> {
    Ok(v1::DetailedCreditNote {
        id: detailed.credit_note.id.as_proto(),
        credit_note_number: detailed.credit_note.credit_note_number.clone(),
        status: credit_note_status_domain_to_server(detailed.credit_note.status) as i32,
        created_at: detailed.credit_note.created_at.as_proto(),
        updated_at: detailed.credit_note.updated_at.as_proto(),
        finalized_at: detailed.credit_note.finalized_at.as_proto(),
        tenant_id: detailed.credit_note.tenant_id.as_proto(),
        customer_id: detailed.credit_note.customer_id.as_proto(),
        invoice_id: detailed.credit_note.invoice_id.as_proto(),
        plan_version_id: detailed.credit_note.plan_version_id.map(|id| id.as_proto()),
        subscription_id: detailed.credit_note.subscription_id.map(|id| id.as_proto()),
        currency: detailed.credit_note.currency.clone(),
        subtotal: detailed.credit_note.subtotal,
        tax_amount: detailed.credit_note.tax_amount,
        total: detailed.credit_note.total,
        refunded_amount_cents: detailed.credit_note.refunded_amount_cents,
        credited_amount_cents: detailed.credit_note.credited_amount_cents,
        line_items: invoice_mapping::invoices::domain_invoice_lines_to_server(
            detailed.credit_note.line_items.clone(),
        ),
        tax_breakdown: detailed
            .credit_note
            .tax_breakdown
            .iter()
            .map(invoice_mapping::invoices::domain_tax_breakdown_to_server)
            .collect(),
        reason: detailed.credit_note.reason.clone(),
        memo: detailed.credit_note.memo.clone(),
        customer_details: Some(
            invoice_mapping::invoices::domain_inline_customer_to_server(
                &detailed.credit_note.customer_details,
            )
            .map_err(|e| tonic::Status::internal(e.to_string()))?,
        ),
        pdf_document_id: detailed.credit_note.pdf_document_id.map(|id| id.as_proto()),
        voided_at: detailed.credit_note.voided_at.as_proto(),
    })
}

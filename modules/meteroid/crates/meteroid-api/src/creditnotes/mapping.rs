use crate::creditnotes::error::CreditNoteApiError;
use crate::invoices::mapping as invoice_mapping;
use common_domain::ids::BaseId;
use meteroid_app::conversions::{AsProtoOpt, ProtoConv};
use meteroid_app::sharable::generate_entity_share_key;
use meteroid_grpc::meteroid::api::creditnotes::v1;
use meteroid_store::domain::{
    CreditNote as DomainCreditNote, CreditNoteStatus, CreditType as DomainCreditType,
    DetailedCreditNote, LineItem,
};
use meteroid_store::repositories::credit_notes::{CreditLineItem, CreditSubLineItem};
use meteroid_store::repositories::payment_transactions::CreditNoteRefundSummary;
use rust_decimal::Decimal;
use secrecy::SecretString;
use std::str::FromStr;

pub fn credit_note_status_domain_to_server(status: CreditNoteStatus) -> v1::CreditNoteStatus {
    match status {
        CreditNoteStatus::Draft => v1::CreditNoteStatus::Draft,
        CreditNoteStatus::Finalized => v1::CreditNoteStatus::Finalized,
        CreditNoteStatus::Voided => v1::CreditNoteStatus::Voided,
    }
}

pub fn credit_type_domain_to_server(credit_type: DomainCreditType) -> v1::CreditType {
    match credit_type {
        DomainCreditType::CreditToBalance => v1::CreditType::CreditToBalance,
        DomainCreditType::Refund => v1::CreditType::Refund,
        DomainCreditType::DebtCancellation => v1::CreditType::DebtCancellation,
    }
}

pub fn credit_note_status_server_to_domain(status: Option<i32>) -> Option<CreditNoteStatus> {
    status.and_then(|s| match v1::CreditNoteStatus::try_from(s).ok()? {
        v1::CreditNoteStatus::Draft => Some(CreditNoteStatus::Draft),
        v1::CreditNoteStatus::Finalized => Some(CreditNoteStatus::Finalized),
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
        credit_type: credit_type_domain_to_server(credit_note.credit_type) as i32,
        custom_properties: (!credit_note.custom_properties.is_null())
            .then(|| credit_note.custom_properties.to_string()),
        debt_cancelled_amount_cents: credit_note.debt_cancelled_amount_cents,
    }
}

/// `invoice_lines` are the credited invoice's, for the net share of a tax-inclusive line's
/// promotional credit; without them the share as drawn is stated.
pub fn detailed_domain_to_server(
    detailed: DetailedCreditNote,
    invoice_lines: Option<&[LineItem]>,
    refunds: Option<&CreditNoteRefundSummary>,
    jwt_secret: &SecretString,
) -> Result<v1::DetailedCreditNote, tonic::Status> {
    let reductions = detailed.credit_note.reductions(invoice_lines);
    let promotional_credit_restored = detailed.credit_note.promotional_credit_restored();
    let share_key = if detailed.credit_note.pdf_document_id.is_some() {
        Some(
            generate_entity_share_key(
                detailed.credit_note.id.as_uuid(),
                detailed.credit_note.tenant_id,
                jwt_secret,
                (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize,
            )
            .map_err(|e| tonic::Status::internal(e.to_string()))?,
        )
    } else {
        None
    };

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
            with_source_credit_units(&detailed.credit_note.line_items, invoice_lines),
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
        xml_document_id: detailed.credit_note.xml_document_id.map(|id| id.as_proto()),
        voided_at: detailed.credit_note.voided_at.as_proto(),
        document_sharing_key: share_key,
        credit_type: credit_type_domain_to_server(detailed.credit_note.credit_type) as i32,
        custom_properties: (!detailed.credit_note.custom_properties.is_null())
            .then(|| detailed.credit_note.custom_properties.to_string()),
        refundable_amount_cents: refunds.map_or(0, |r| r.refundable_cents),
        refunded_settled_cents: refunds.map_or(0, |r| r.refunded_cents),
        refunded_pending_cents: refunds.map_or(0, |r| r.pending_cents),
        debt_cancelled_amount_cents: detailed.credit_note.debt_cancelled_amount_cents,
        discount_cents: reductions.discount,
        promotional_credit_cents: reductions.promotional_credit,
        promotional_credit_restored_cents: promotional_credit_restored,
    })
}

/// A note line credits only the money a credit-priced line billed, so it stores no credit usage;
/// the page states the usage of the line it credits, read from the invoice.
fn with_source_credit_units(
    note_lines: &[LineItem],
    invoice_lines: Option<&[LineItem]>,
) -> Vec<LineItem> {
    note_lines
        .iter()
        .map(|line| {
            let mut line = line.clone();
            if line.credit_unit.is_none() {
                line.credit_unit = invoice_lines
                    .and_then(|lines| lines.iter().find(|source| source.local_id == line.local_id))
                    .and_then(|source| source.credit_unit.clone());
            }
            line
        })
        .collect()
}

/// Exactly one of quantity, sub-lines and amount says how much of the line is credited.
pub fn credit_line_item_server_to_domain(
    line: &v1::CreditLineItem,
) -> Result<CreditLineItem, CreditNoteApiError> {
    let local_id = line.line_item_local_id.clone();
    let decimal = |value: &str, what: &str| {
        Decimal::from_str(value)
            .map_err(|e| CreditNoteApiError::InvalidArgument(format!("Invalid {what}: {e}")))
    };
    match (&line.quantity, line.sub_lines.is_empty(), line.amount) {
        (Some(quantity), true, None) => Ok(CreditLineItem::Line {
            quantity: decimal(quantity, "quantity")?,
            local_id,
        }),
        (None, false, None) => Ok(CreditLineItem::SubLines {
            sub_lines: line
                .sub_lines
                .iter()
                .map(|sub| {
                    Ok(CreditSubLineItem {
                        local_id: sub.sub_line_local_id.clone(),
                        quantity: decimal(&sub.quantity, "sub-line quantity")?,
                    })
                })
                .collect::<Result<_, CreditNoteApiError>>()?,
            local_id,
        }),
        (None, true, Some(amount_cents)) => Ok(CreditLineItem::Amount {
            local_id,
            amount_cents,
        }),
        (None, true, None) => Err(CreditNoteApiError::InvalidArgument(format!(
            "Line '{local_id}': quantity, sub_lines or amount is required"
        ))),
        _ => Err(CreditNoteApiError::InvalidArgument(format!(
            "Line '{local_id}': quantity, sub_lines and amount are mutually exclusive"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(quantity: Option<&str>, sub_lines: bool, amount: Option<i64>) -> v1::CreditLineItem {
        v1::CreditLineItem {
            line_item_local_id: "l".to_owned(),
            quantity: quantity.map(str::to_owned),
            sub_lines: if sub_lines {
                vec![v1::CreditSubLineItem {
                    sub_line_local_id: "s".to_owned(),
                    quantity: "2".to_owned(),
                }]
            } else {
                Vec::new()
            },
            amount,
        }
    }

    /// Usage priced in a credit unit has no money unit price: the dashboard credits its overage
    /// by amount, which maps to an amount credit.
    #[test]
    fn a_line_is_credited_by_exactly_one_of_quantity_sub_lines_or_amount() {
        assert!(matches!(
            credit_line_item_server_to_domain(&line(None, false, Some(150))),
            Ok(CreditLineItem::Amount {
                amount_cents: 150,
                ..
            })
        ));
        assert!(matches!(
            credit_line_item_server_to_domain(&line(Some("1.5"), false, None)),
            Ok(CreditLineItem::Line { quantity, .. }) if quantity == Decimal::new(15, 1)
        ));
        assert!(matches!(
            credit_line_item_server_to_domain(&line(None, true, None)),
            Ok(CreditLineItem::SubLines { sub_lines, .. }) if sub_lines.len() == 1
        ));
        for invalid in [
            line(None, false, None),
            line(Some("1"), false, Some(150)),
            line(None, true, Some(150)),
            line(Some("1"), true, None),
        ] {
            assert!(credit_line_item_server_to_domain(&invalid).is_err());
        }
    }
}

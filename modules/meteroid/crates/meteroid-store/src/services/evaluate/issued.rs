//! What a subscription's issued invoices billed, line by line, as reconcile reads it (R-075,
//! R-080). A line attached to no fee is listed apart: reconcile never guesses its charge.

use std::collections::HashMap;

use billing_core::charge::{Denomination, Fee};
use billing_core::issued::{IssuedKind, IssuedLine};
use chrono::NaiveDate;
use common_domain::ids::{InvoiceId, SubscriptionId, TenantId};
use diesel_models::enums::InvoiceStatusEnum;
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;

use crate::StoreResult;
use crate::domain::enums::InvoiceType;
use crate::domain::{Invoice, LineItem, LineItemKind};
use crate::errors::StoreError;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::invoice_lines::charges::{Lineages, evaluated_charge_key};
use crate::services::invoice_lines::minimum::is_up_front;
use crate::services::subscriptions::amendment::load_credit_matching_context;
use crate::store::PgConn;

/// An issued line and the invoice it is on: two invoices may bill one charge's id (a correction
/// reissuing a credited invoice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedOn {
    pub invoice: InvoiceId,
    pub line: IssuedLine,
    /// Its invoice is a draft: billing once issued, credited only then.
    pub draft: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issued {
    pub lines: Vec<IssuedOn>,
    /// Lines naming no component or add-on: past the watermark they bill nothing.
    pub unattached: Vec<IssuedOn>,
    /// What each line's corrections took back, by the day their note was finalized.
    pub noted: HashMap<String, Vec<(NaiveDate, i64)>>,
}

/// What corrections dated from the watermark `w` took back from those of `lines` starting before
/// it: activity from the watermark, counted with what falls due from it (R-094).
pub fn noted_from(
    noted: &HashMap<String, Vec<(NaiveDate, i64)>>,
    lines: &[IssuedLine],
    w: NaiveDate,
) -> i64 {
    lines
        .iter()
        .filter(|line| line.from < w && line.key.denomination == Denomination::Money)
        .filter_map(|line| noted.get(&line.id))
        .flatten()
        .filter(|(day, _)| *day >= w)
        .map(|(_, amount)| amount)
        .sum()
}

/// The corrections of `invoice_id`'s lines, as `load_credit_matching_context` counts them, by the
/// day their note was finalized.
async fn corrections_by_day(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    into: &mut HashMap<String, Vec<(NaiveDate, i64)>>,
) -> StoreResult<()> {
    use diesel_models::credit_notes::CreditNoteRow;
    use diesel_models::enums::{CreditNoteOriginEnum, CreditNoteStatus};
    let notes = CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for note in notes {
        let Some(finalized) = note.finalized_at else {
            continue;
        };
        if note.status != CreditNoteStatus::Finalized
            || matches!(
                note.origin,
                CreditNoteOriginEnum::Manual | CreditNoteOriginEnum::Void
            )
        {
            continue;
        }
        let lines: Vec<LineItem> = serde_json::from_value(note.line_items).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize credit note line items".to_string(),
                e,
            ))
        })?;
        for line in lines.into_iter().filter(|line| line.amount_subtotal < 0) {
            into.entry(line.local_id)
                .or_default()
                .push((finalized.date(), -line.amount_subtotal));
        }
    }
    Ok(())
}

/// The lines of the issued invoices of `subscription_id` settling periods from `from` on, its
/// drafts but `except` too (they bill once issued); on a consolidated parent, only this
/// subscription's. A voided invoice's lines are read as voided (R-096), unless it is of
/// `reissuing`, the slot (any period when none) the document computed now bills again, or a live
/// invoice bills its slot again.
pub async fn load_billed(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    from: NaiveDate,
    except: Option<InvoiceId>,
    reissuing: Option<(InvoiceType, Option<NaiveDate>)>,
) -> StoreResult<Issued> {
    let rows = InvoiceRow::list_issued_of_subscription(
        conn,
        tenant_id,
        subscription_id,
        from,
        true,
        except,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    // A merged draft bills through its consolidated invoice: counted once, as the draft, while
    // that invoice stands; the one being computed never counts its own parent.
    let parents: HashMap<InvoiceId, bool> = rows
        .iter()
        .filter(|row| row.subscription_id.is_none())
        .map(|row| (row.id, row.status == InvoiceStatusEnum::Void))
        .collect();
    let draft_parents: Vec<InvoiceId> = rows
        .iter()
        .filter(|row| row.subscription_id.is_none() && row.status == InvoiceStatusEnum::Draft)
        .map(|row| row.id)
        .collect();
    // A voided recurring or final invoice whose slot a live invoice bills again is replaced by
    // it: never read, so its forgiveness ends where the slot is billed again (R-096).
    let reissued: Vec<(InvoiceType, Option<NaiveDate>)> = rows
        .iter()
        .filter(|row| row.subscription_id.is_some() && row.status != InvoiceStatusEnum::Void)
        .filter_map(slot_of)
        .collect();
    let rows = rows.into_iter().filter(|row| {
        row.subscription_id.is_some()
            && !(row.status == InvoiceStatusEnum::Void
                && slot_of(row).is_some_and(|slot| reissued.contains(&slot)))
            && row
                .consolidated_into_invoice_id
                .is_none_or(|parent| parents.contains_key(&parent))
            && !(row.status == InvoiceStatusEnum::Void
                && reissuing.as_ref().is_some_and(|(kind, period)| {
                    *kind == InvoiceType::from(row.invoice_type.clone())
                        && period.is_none_or(|period| period == row.billing_period_start)
                }))
    });
    let mut issued = Issued::default();
    for row in rows {
        // A draft merged into a voided parent was voided with it.
        let parent = row.consolidated_into_invoice_id;
        let parent_voided = parent
            .and_then(|parent| parents.get(&parent).copied())
            .unwrap_or(false);
        let mut invoice: Invoice = row.try_into()?;
        if parent_voided {
            invoice.status = crate::domain::InvoiceStatusEnum::Void;
        }
        if invoice.subscription_id.is_none() {
            let owners = LineOwners::load_tx(conn, tenant_id, &invoice.line_items).await?;
            invoice
                .line_items
                .retain(|line| line_subscription(line, None, &owners) == Some(subscription_id));
        }
        let (line_lineage, mut already_credited, mut corrected, _) =
            load_credit_matching_context(conn, tenant_id, &invoice).await?;
        corrections_by_day(conn, tenant_id, invoice.id, &mut issued.noted).await?;
        // A draft merged into a consolidated invoice is credited by notes against that invoice.
        if let Some(parent) = parent {
            let as_parent = Invoice {
                id: parent,
                ..invoice.clone()
            };
            let (_, credited, by_engine, _) =
                load_credit_matching_context(conn, tenant_id, &as_parent).await?;
            for (line, amount) in credited {
                *already_credited.entry(line).or_insert(0) += amount;
            }
            for (line, amount) in by_engine {
                *corrected.entry(line).or_insert(0) += amount;
            }
            corrections_by_day(conn, tenant_id, parent, &mut issued.noted).await?;
        }
        let lineages = Lineages::of_lines(&invoice.line_items, &line_lineage);
        for line in &invoice.line_items {
            let issued_line = issued_line_of(
                line,
                &invoice,
                subscription_id,
                &lineages,
                &line_lineage,
                &already_credited,
                &corrected,
            );
            let on = IssuedOn {
                invoice: invoice.id,
                line: issued_line,
                draft: match parent {
                    Some(parent) => draft_parents.contains(&parent),
                    None => invoice.status == crate::domain::InvoiceStatusEnum::Draft,
                },
            };
            if on.line.key.fee == Fee::Unattached {
                issued.unattached.push(on);
            } else {
                issued.lines.push(on);
            }
        }
    }
    Ok(issued)
}

/// The slot a recurring invoice (its period) or a final invoice bills, which one invoice holds.
fn slot_of(row: &InvoiceRow) -> Option<(InvoiceType, Option<NaiveDate>)> {
    match InvoiceType::from(row.invoice_type.clone()) {
        InvoiceType::Recurring => Some((InvoiceType::Recurring, Some(row.billing_period_start))),
        InvoiceType::Final => Some((InvoiceType::Final, None)),
        _ => None,
    }
}

/// A line as reconcile reads it. One priced in a credit unit bills the credits it consumed, in
/// that unit; the notes on it return money of its overage, never credits, so none count (R-039).
pub(crate) fn issued_line_of(
    line: &LineItem,
    invoice: &Invoice,
    subscription_id: SubscriptionId,
    lineages: &Lineages,
    line_lineage: &HashMap<String, String>,
    already_credited: &HashMap<String, i64>,
    corrected: &HashMap<String, i64>,
) -> IssuedLine {
    let mut key = evaluated_charge_key(line, lineages);
    key.subscription = key.subscription.or(Some(subscription_id));
    let in_credits = key.denomination != Denomination::Money;
    let money = |of: &HashMap<String, i64>| {
        if in_credits {
            0
        } else {
            of.get(&line.local_id).copied().unwrap_or(0)
        }
    };
    IssuedLine {
        id: line.local_id.clone(),
        kind: match (&line.kind, line.metric_id) {
            (LineItemKind::Regular, None) => IssuedKind::Fee {
                price_component: line.price_component_id,
                add_on: line.sub_add_on_id.is_some(),
            },
            (LineItemKind::MinimumShortfall, _) => IssuedKind::Shortfall {
                up_front: is_up_front(line, invoice.billing_period_start),
            },
            _ => IssuedKind::Other,
        },
        lineage: line_lineage.get(&line.local_id).cloned(),
        quantity: line.quantity,
        unit_priced: match &line.credit_unit {
            Some(unit) => unit.unit_price.is_some(),
            None => line.unit_price.is_some(),
        },
        from: line.start_date,
        until: line.end_date,
        subtotal: match &line.credit_unit {
            Some(unit) => unit.consumed,
            None => line.amount_subtotal,
        },
        quoted: line
            .tax_inclusive_quote
            .as_ref()
            .filter(|_| !in_credits)
            .map(|quote| quote.amount_subtotal),
        already_credited: money(already_credited),
        corrected: money(corrected),
        settled: None,
        voided: invoice.status == crate::domain::InvoiceStatusEnum::Void,
        key,
    }
}

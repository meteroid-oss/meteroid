//! What a subscription's invoices still owe, and the credit its unpaid credit packs add once paid.
//! Consolidated invoices bill the subscription too: they count whole, and only the pack lines the
//! subscription contributed to them count as its pending credit.
use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use common_domain::ids::{CreditUnitId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::customer_credit::CustomerCreditGrantRow;
use diesel_models::enums::InvoicePaymentStatus;
use diesel_models::invoices::{InvoiceRow, UnsettledInvoiceRow};
use diesel_models::query::account_signals::{
    self as queries, FailedPaymentRow, OpenInvoiceRow, PendingRetryRow,
};
use error_stack::Report;
use uuid::Uuid;

use crate::StoreResult;
use crate::domain::{LineItem, LineItemKind};
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::source;
use crate::repositories::credit_notes::cumulative_grant_reversal;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::ServicesEdge;
use crate::services::credit::invalid;
use crate::services::credit::mint::credit_line_source;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverdueInvoice {
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub amount_due_minor: i64,
    pub due_at: NaiveDateTime,
    pub last_failure_at: Option<NaiveDateTime>,
    pub last_failure_reason: Option<String>,
    pub next_retry_at: Option<NaiveDateTime>,
}

/// Credit a pack line of an unpaid invoice funds once the invoice is paid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInvoiceCredit {
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    /// The funded wallet's denomination: the invoice currency, or the credit unit's code.
    pub denomination: String,
    pub credit_unit_id: Option<CreditUnitId>,
    pub amount_minor: i64,
}

/// Amounts are minor units of `currency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionReceivables {
    pub currency: String,
    pub open_minor: i64,
    pub open_count: u32,
    pub overdue_minor: i64,
    pub overdue_count: u32,
    /// The soonest due date among open invoices not yet overdue.
    pub next_due_at: Option<NaiveDateTime>,
    pub oldest_overdue: Option<OverdueInvoice>,
    pub last_failure_at: Option<NaiveDateTime>,
    pub last_failure_reason: Option<String>,
    pub last_failure_invoice_id: Option<InvoiceId>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub pending_credit: Vec<PendingInvoiceCredit>,
}

/// An unpaid invoice's pack lines the subscription is owed credit for.
struct PackLines {
    invoice_id: InvoiceId,
    invoice_number: String,
    currency: String,
    lines: Vec<LineItem>,
}

impl ServicesEdge {
    pub async fn subscription_receivables(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: NaiveDateTime,
    ) -> StoreResult<SubscriptionReceivables> {
        let subscription = self
            .store
            .get_subscription(tenant_id, subscription_id)
            .await?;
        let mut conn = self.store.get_conn().await?;
        let parents = InvoiceRow::consolidated_parent_ids(&mut conn, tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let unsettled = InvoiceRow::list_unsettled_by_subscription(
            &mut conn,
            tenant_id,
            subscription_id,
            &parents,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let open = unsettled
            .iter()
            .filter(|row| is_open(row))
            .map(|row| OpenInvoiceRow {
                id: row.id,
                invoice_number: row.invoice_number.clone(),
                currency: row.currency.clone(),
                amount_due: row.amount_due,
                due_at: row.due_at,
                payment_status: format!("{:?}", row.payment_status),
            })
            .collect::<Vec<_>>();
        let open_ids = open.iter().map(|row| row.id).collect::<Vec<_>>();
        let failures = queries::list_last_failed_payments(&mut conn, tenant_id, &open_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let retries = queries::list_pending_payment_retries(&mut conn, tenant_id, &open_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let unsettled_parents = unsettled
            .iter()
            .filter(|row| row.subscription_id.is_none())
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let mut contributed: HashMap<InvoiceId, HashSet<String>> = HashMap::new();
        for member in
            InvoiceRow::list_member_lines(&mut conn, tenant_id, subscription_id, &unsettled_parents)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            let Some(parent) = member.consolidated_into_invoice_id else {
                continue;
            };
            contributed.entry(parent).or_default().extend(
                parse_lines(member.line_items)?
                    .into_iter()
                    .map(|l| l.local_id),
            );
        }

        let mut packs = Vec::new();
        for row in unsettled {
            let contribution = row
                .subscription_id
                .is_none()
                .then(|| contributed.get(&row.id).cloned().unwrap_or_default());
            let lines = owed_pack_lines(parse_lines(row.line_items)?, contribution.as_ref());
            if !lines.is_empty() {
                packs.push(PackLines {
                    invoice_id: row.id,
                    invoice_number: row.invoice_number,
                    currency: row.currency,
                    lines,
                });
            }
        }
        let pack_invoices = packs.iter().map(|p| p.invoice_id).collect::<Vec<_>>();
        let mut credited: HashMap<InvoiceId, HashMap<String, i64>> = HashMap::new();
        for (invoice_id, lines) in
            CreditNoteRow::list_finalized_lines_by_invoices(&mut conn, tenant_id, &pack_invoices)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            add_credited(credited.entry(invoice_id).or_default(), parse_lines(lines)?)?;
        }
        let sources = packs
            .iter()
            .flat_map(|p| {
                p.lines
                    .iter()
                    .map(|line| credit_line_source(p.invoice_id, &line.local_id))
            })
            .collect::<Vec<_>>();
        let funded = CustomerCreditGrantRow::list_funded_sources(
            &mut conn,
            tenant_id,
            source::CREDIT_PURCHASE,
            &sources,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .collect::<HashSet<_>>();

        Ok(summarize(
            subscription.currency,
            &open,
            &failures,
            &retries,
            pending_pack_credit(&packs, &credited, &funded)?,
            now,
        ))
    }
}

fn parse_lines(value: serde_json::Value) -> StoreResult<Vec<LineItem>> {
    serde_json::from_value(value)
        .map_err(|error| Report::new(StoreError::SerdeError("invoice lines".into(), error)))
}

/// A purchased pack line: it funds credit when its invoice is paid. Included credit funds at issue.
fn is_pending_pack(line: &LineItem) -> bool {
    line.kind == LineItemKind::Credit
        && line.amount_total >= 0
        && !line.credit_grant_included
        && line.credit_grant_amount.is_some_and(|amount| amount > 0)
}

/// The pending pack lines of an invoice: all of the subscription's own invoice, only the lines it
/// `contributed` to a consolidated one.
fn owed_pack_lines(lines: Vec<LineItem>, contributed: Option<&HashSet<String>>) -> Vec<LineItem> {
    lines
        .into_iter()
        .filter(is_pending_pack)
        .filter(|line| contributed.is_none_or(|ids| ids.contains(&line.local_id)))
        .collect()
}

/// Subtotal finalized credit notes took off each line, as minting reads it.
fn add_credited(credited: &mut HashMap<String, i64>, lines: Vec<LineItem>) -> StoreResult<()> {
    for line in lines {
        let slot = credited.entry(line.local_id).or_insert(0);
        *slot = line
            .amount_subtotal
            .checked_abs()
            .and_then(|amount| slot.checked_add(amount))
            .ok_or_else(|| invalid("credited subtotal exceeds the supported range"))?;
    }
    Ok(())
}

fn pending_pack_credit(
    packs: &[PackLines],
    credited: &HashMap<InvoiceId, HashMap<String, i64>>,
    funded: &HashSet<Uuid>,
) -> StoreResult<Vec<PendingInvoiceCredit>> {
    let mut pending = Vec::new();
    for pack in packs {
        for line in &pack.lines {
            if funded.contains(&credit_line_source(pack.invoice_id, &line.local_id)) {
                continue;
            }
            let face = line.credit_grant_amount.unwrap_or(0);
            let notes = credited
                .get(&pack.invoice_id)
                .and_then(|lines| lines.get(&line.local_id))
                .copied()
                .unwrap_or(0);
            let cancelled = match notes {
                0 => 0,
                subtotal => cumulative_grant_reversal(face, subtotal, line.amount_subtotal)
                    .ok_or_else(|| invalid("the credit notes of this pack cannot be prorated"))?,
            };
            let amount_minor = face - cancelled;
            if amount_minor <= 0 {
                continue;
            }
            pending.push(PendingInvoiceCredit {
                invoice_id: pack.invoice_id,
                invoice_number: pack.invoice_number.clone(),
                denomination: line
                    .credit_grant_unit
                    .as_ref()
                    .map_or_else(|| pack.currency.clone(), |unit| unit.code.clone()),
                credit_unit_id: line.credit_grant_unit.as_ref().map(|unit| unit.unit_id),
                amount_minor,
            });
        }
    }
    Ok(pending)
}

/// The invoices the customer receivables count: an amount still to collect, no payment in flight.
fn is_open(row: &UnsettledInvoiceRow) -> bool {
    row.amount_due > 0
        && matches!(
            row.payment_status,
            InvoicePaymentStatus::Unpaid
                | InvoicePaymentStatus::PartiallyPaid
                | InvoicePaymentStatus::Errored
        )
}

/// An invoice is overdue from the day after its due date: one due today is not late yet.
pub(crate) fn is_past_due(due_at: NaiveDateTime, now: NaiveDateTime) -> bool {
    due_at.date() < now.date()
}

fn summarize(
    currency: String,
    open: &[OpenInvoiceRow],
    failures: &[FailedPaymentRow],
    retries: &[PendingRetryRow],
    pending_credit: Vec<PendingInvoiceCredit>,
    now: NaiveDateTime,
) -> SubscriptionReceivables {
    let overdue = open
        .iter()
        .filter(|row| row.due_at.is_some_and(|due| is_past_due(due, now)))
        .collect::<Vec<_>>();
    let last_failure = failures.iter().max_by_key(|f| f.failed_at);
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);

    SubscriptionReceivables {
        currency,
        open_minor: open.iter().map(|row| row.amount_due).sum(),
        open_count: count(open.len()),
        overdue_minor: overdue.iter().map(|row| row.amount_due).sum(),
        overdue_count: count(overdue.len()),
        next_due_at: open
            .iter()
            .filter_map(|row| row.due_at)
            .filter(|due| !is_past_due(*due, now))
            .min(),
        oldest_overdue: overdue
            .iter()
            .filter_map(|row| row.due_at.map(|due| (row, due)))
            .min_by_key(|(_, due)| *due)
            .map(|(row, due_at)| {
                let failure = failures.iter().find(|f| f.invoice_id == row.id);
                OverdueInvoice {
                    invoice_id: row.id,
                    invoice_number: row.invoice_number.clone(),
                    amount_due_minor: row.amount_due,
                    due_at,
                    last_failure_at: failure.map(|f| f.failed_at),
                    last_failure_reason: failure.and_then(|f| f.error_type.clone()),
                    next_retry_at: retries
                        .iter()
                        .filter(|r| r.invoice_id == row.id)
                        .map(|r| r.next_retry_at)
                        .min(),
                }
            }),
        last_failure_at: last_failure.map(|f| f.failed_at),
        last_failure_reason: last_failure.and_then(|f| f.error_type.clone()),
        last_failure_invoice_id: last_failure.map(|f| f.invoice_id),
        next_retry_at: retries.iter().map(|r| r.next_retry_at).min(),
        pending_credit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn invoice(n: u128, amount_due: i64, due: Option<u32>) -> OpenInvoiceRow {
        OpenInvoiceRow {
            id: InvoiceId::from(Uuid::from_u128(n)),
            invoice_number: format!("INV-{n}"),
            currency: "USD".to_string(),
            amount_due,
            due_at: due.map(at),
            payment_status: "Unpaid".to_string(),
        }
    }

    fn line(local_id: &str, kind: &str, subtotal: i64, credit: Option<i64>) -> LineItem {
        serde_json::from_value(json!({
            "local_id": local_id,
            "name": local_id,
            "amount_subtotal": subtotal,
            "taxable_amount": subtotal,
            "amount_total": subtotal,
            "quantity": null,
            "unit_price": null,
            "start_date": "2026-10-01",
            "end_date": "2026-11-01",
            "sub_lines": [],
            "is_prorated": false,
            "kind": kind,
            "credit_grant_amount": credit,
        }))
        .unwrap()
    }

    fn pack(n: u128, lines: Vec<LineItem>) -> PackLines {
        PackLines {
            invoice_id: InvoiceId::from(Uuid::from_u128(n)),
            invoice_number: format!("INV-{n}"),
            currency: "USD".to_string(),
            lines: owed_pack_lines(lines, None),
        }
    }

    #[test]
    fn an_invoice_due_today_is_not_overdue_yet() {
        let open = [invoice(1, 5_000, Some(10))];
        let midday = at(10) + chrono::Duration::hours(12);
        let r = summarize("USD".to_string(), &open, &[], &[], vec![], midday);
        assert_eq!((r.overdue_minor, r.overdue_count), (0, 0));
        assert_eq!(r.next_due_at, Some(at(10)));

        let r = summarize("USD".to_string(), &open, &[], &[], vec![], at(11));
        assert_eq!((r.overdue_minor, r.overdue_count), (5_000, 1));
    }

    #[test]
    fn open_and_overdue_split_on_the_due_date_and_name_the_oldest_overdue() {
        let open = [
            invoice(1, 50_000, Some(9)),
            invoice(2, 11_500, Some(2)),
            invoice(3, 500_000, Some(31)),
            invoice(4, 2_000, None),
        ];
        let failures = [FailedPaymentRow {
            invoice_id: open[0].id,
            failed_at: at(6),
            error_type: Some("card declined".to_string()),
        }];
        let retries = [PendingRetryRow {
            invoice_id: open[0].id,
            next_retry_at: at(12),
        }];
        let r = summarize(
            "USD".to_string(),
            &open,
            &failures,
            &retries,
            vec![],
            at(10),
        );

        assert_eq!((r.open_minor, r.open_count), (563_500, 4));
        assert_eq!((r.overdue_minor, r.overdue_count), (61_500, 2));
        assert_eq!(r.next_due_at, Some(at(31)));
        let oldest = r.oldest_overdue.expect("an overdue invoice");
        assert_eq!((oldest.invoice_id, oldest.due_at), (open[1].id, at(2)));
        // The failure and retry belong to INV-1, not to the oldest overdue invoice.
        assert_eq!((oldest.last_failure_at, oldest.next_retry_at), (None, None));
        assert_eq!(r.last_failure_invoice_id, Some(open[0].id));
        assert_eq!(r.next_retry_at, Some(at(12)));
    }

    #[test]
    fn a_partially_paid_invoice_counts_what_is_still_due() {
        // $500 billed, $200 paid, then a $100 credit note: $200 left to collect.
        let open = [invoice(1, 20_000, Some(31))];
        let r = summarize("USD".to_string(), &open, &[], &[], vec![], at(10));
        assert_eq!((r.open_minor, r.overdue_minor), (20_000, 0));
    }

    #[test]
    fn nothing_open_reads_as_zero_with_no_dates() {
        let r = summarize("EUR".to_string(), &[], &[], &[], vec![], at(10));
        assert_eq!((r.open_minor, r.overdue_minor, r.open_count), (0, 0, 0));
        assert!(r.next_due_at.is_none() && r.oldest_overdue.is_none());
    }

    #[test]
    fn an_unpaid_pack_is_pending_net_of_credit_notes_and_never_twice() {
        // Scale pack: $500 paid for $550 of credit; a credit note took $250 off its subtotal.
        let scale = pack(
            1,
            vec![
                line("pack", "Credit", 50_000, Some(55_000)),
                line("usage", "Regular", 6_000, None),
            ],
        );
        let funded_already = pack(2, vec![line("pack", "Credit", 50_000, Some(55_000))]);
        let mut credited = HashMap::new();
        add_credited(
            credited.entry(scale.invoice_id).or_default(),
            vec![line("pack", "Credit", -25_000, None)],
        )
        .unwrap();
        let funded = HashSet::from([credit_line_source(funded_already.invoice_id, "pack")]);

        let pending = pending_pack_credit(&[scale, funded_already], &credited, &funded).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            (pending[0].amount_minor, pending[0].denomination.as_str()),
            (27_500, "USD")
        );
    }

    #[test]
    fn a_consolidated_invoice_owes_only_the_packs_this_subscription_merged_into_it() {
        let parent_lines = vec![
            line("ours", "Credit", 50_000, Some(55_000)),
            line("theirs", "Credit", 4_900, Some(4_900)),
        ];
        let ours = HashSet::from(["ours".to_string()]);
        let parent = PackLines {
            invoice_id: InvoiceId::from(Uuid::from_u128(9)),
            invoice_number: "INV-0009".to_string(),
            currency: "USD".to_string(),
            lines: owed_pack_lines(parent_lines.clone(), Some(&ours)),
        };
        let pending = pending_pack_credit(&[parent], &HashMap::new(), &HashSet::new()).unwrap();
        assert_eq!(
            pending.iter().map(|p| p.amount_minor).collect::<Vec<_>>(),
            vec![55_000]
        );
        assert_eq!(owed_pack_lines(parent_lines, None).len(), 2);
    }
}

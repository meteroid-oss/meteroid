//! Money a credit note or an invoice void moves in the customer credit ledger, in the document's
//! own transaction: credit a note returns (or takes back on void), purchased credit a refund
//! reverses, and credit a voided invoice drew.
use super::*;
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope, LedgerEntryKind};
use crate::domain::wallet_events::CreditUnrecoveredTrigger;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::allocation::{CreditRequest, GrantPool};
use crate::repositories::credit::customer_credit::{self as credit, CreditFunding, source};
use crate::repositories::credit::line_allocation;
use crate::services::credit::promotional::{LineScope, pretax_request};
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::credit::{invalid, precondition};
use common_domain::ids::SubscriptionId;
use diesel_models::customer_credit::{
    CustomerCreditAccountRow, CustomerCreditEntryRow, CustomerCreditGrantRow,
    HistoricalBalanceTxRow,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn finalize_tx(
    store: &Store,
    conn: &mut PgConn,
    tenant: TenantId,
    actor: &Actor,
    note_id: CreditNoteId,
    applied_credits_restored_externally: bool,
    purchased_credit_removed: bool,
    credit_at: Option<DateTime<Utc>>,
) -> StoreResult<CreditNote> {
    let observed = CreditNoteRow::find_by_id(conn, tenant, note_id).await?;
    let (lock, invoice) = lock_credit_invoice(conn, tenant, observed.invoice_id).await?;
    let lock = &lock;
    diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
    let note = finalize_credit_note_decision_tx(
        store,
        conn,
        lock,
        actor,
        &invoice,
        note_id,
        applied_credits_restored_externally,
        purchased_credit_removed,
    )
    .await?;
    if note.credit_type == CreditType::Refund {
        reverse_purchased_credit_tx(conn, &*store.hooks.credit, lock, &invoice, &note).await?;
    }
    return_promotional_credit_tx(conn, &*store.hooks.credit, lock, &invoice, &note).await?;
    if matches!(
        note.credit_type,
        CreditType::CreditToBalance | CreditType::Refund
    ) && note.credited_amount_cents > 0
    {
        return_credit_tx(
            conn,
            &*store.hooks.credit,
            lock,
            &invoice,
            &note,
            actor,
            credit_at,
        )
        .await?;
        crate::services::credit::follow_credit_given_back_tx(
            conn,
            &*store.hooks.credit,
            lock,
            invoice.id,
        )
        .await?;
    }
    // The note moved what the invoice still collects, which the purchase's missing money and its
    // reported loss stand on.
    crate::services::credit::reconcile_purchased_credit_tx(
        conn,
        &*store.hooks.credit,
        lock,
        invoice.id,
        None,
    )
    .await?;
    Ok(note)
}

/// Takes back the credit a note returned before it is voided. Refused once any of it is spent, or
/// when the note took credit back itself (that reversal cannot be undone).
pub(super) async fn take_back_returned_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    note: &CreditNoteRow,
) -> StoreResult<()> {
    let tenant = note.tenant_id;
    let credited: Vec<LineItem> = serde_json::from_value(note.line_items.clone())
        .map_err(|error| StoreError::SerdeError("credit-note lines".into(), error))?;
    let funded_net = CustomerCreditGrantRow::lock_by_invoice(conn, lock, note.invoice_id)
        .await?
        .iter()
        .filter(|grant| grant.source_kind == source::CREDIT_PURCHASE)
        .filter_map(|grant| grant.line_local_id.as_deref())
        .any(|line| {
            credited
                .iter()
                .any(|l| l.local_id == line && l.amount_subtotal != 0)
        });
    if funded_net {
        return Err(precondition(
            "this credit note changed the credit its purchase funds; it cannot be voided",
        ));
    }
    let movements = CustomerCreditEntryRow::list_by_note(conn, tenant, note.id).await?;
    if movements
        .iter()
        .any(|entry| entry.kind == LedgerEntryKind::Reversal.as_db_str())
    {
        return Err(precondition(
            "a credit note that took credit back cannot be voided; issue credit to the customer instead",
        ));
    }
    credit::redraw_note_returns_tx(conn, controls, lock, note.id, &movements, Utc::now()).await?;
    let returned = movements
        .iter()
        .filter(|entry| entry.kind == LedgerEntryKind::Fund.as_db_str())
        .map(|entry| entry.grant_id)
        .collect::<Vec<_>>();
    for grant in CustomerCreditGrantRow::lock_by_ids(conn, lock, &returned).await? {
        if grant.remaining < grant.amount {
            return Err(precondition(
                "the credit this note returned is already spent; it cannot be voided",
            ));
        }
        credit::reverse_tx(
            conn,
            controls,
            lock,
            grant.id,
            grant.amount,
            note.id,
            Utc::now(),
        )
        .await?;
    }
    if note.credited_amount_cents > 0
        && HistoricalBalanceTxRow::note_credited_retired_balance(conn, tenant, note.id).await?
    {
        take_back_retired_balance_credit_tx(conn, controls, lock, note).await?;
    }
    Ok(())
}

/// A note finalized before the ledger credited the retired scalar, carried into the customer's
/// CUSTOMER_BALANCE grant. Only a same-currency amount is exact: main converted the rest.
async fn take_back_retired_balance_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    note: &CreditNoteRow,
) -> StoreResult<()> {
    let grant = CustomerCreditGrantRow::find_by_source(
        conn,
        note.tenant_id,
        source::CUSTOMER_BALANCE,
        note.customer_id.as_uuid(),
    )
    .await?
    .ok_or_else(|| {
        precondition(
            "the credit this note added to the customer balance is already spent; it cannot be voided",
        )
    })?;
    let account = CustomerCreditAccountRow::find_by_id(conn, note.tenant_id, grant.account_id)
        .await?
        .ok_or_else(|| precondition("customer credit wallet is missing"))?;
    if account.currency != credit::normalized_currency(&note.currency) {
        return Err(precondition(
            "this note credited the customer balance in another currency before the credit ledger; it cannot be taken back exactly",
        ));
    }
    credit::reverse_tx(
        conn,
        controls,
        lock,
        grant.id,
        note.credited_amount_cents,
        note.id,
        Utc::now(),
    )
    .await
}

/// Drawn credit goes back to the grants it came from with their terms; only value beyond what the
/// invoice still holds (collected cash) becomes new paid credit, dated `credit_at` (the note's
/// business date; now for a note issued by hand). A refund of an invoice paid by the retired
/// scalar returns that share in the currency it was deducted in.
async fn return_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice: &Invoice,
    note: &CreditNote,
    actor: &Actor,
    credit_at: Option<DateTime<Utc>>,
) -> StoreResult<()> {
    let drew =
        credit::invoice_drew_credit_tx(conn, invoice.tenant_id, invoice.id, &invoice.currency)
            .await?;
    let (amount, currency) =
        if note.credit_type == CreditType::Refund && invoice.applied_credits > 0 && !drew {
            historical_refund_return_tx(conn, lock, invoice, note).await?
        } else if drew {
            // The credited lines tax included: a grant gets back only what it could pay of them.
            let (lines, owners) = owned_note_lines_tx(conn, invoice, note).await?;
            let (request, of_line) = pretax_request(
                &lines,
                LineScope {
                    owners: &owners,
                    subscription_id: invoice.subscription_id,
                },
                |line| line.amount_total.saturating_neg(),
            )?;
            // No credit pays a credit purchase (R4): what credits one never goes back to a grant
            // that paid another line of the invoice.
            let purchases = lines
                .iter()
                .filter(|line| line.kind == LineItemKind::Credit)
                .try_fold(0_i64, |sum, line| {
                    sum.checked_add(line.amount_total.saturating_neg().max(0))
                })
                .ok_or_else(|| invalid("credited amount overflow"))?;
            let request = CreditRequest {
                requested_minor: (note.credited_amount_cents - purchases).max(0),
                ..request.undated()
            };
            let (given_back, placed) = credit::return_draws_for_note_tx(
                conn,
                controls,
                lock,
                invoice.id,
                &invoice.currency,
                note.id,
                GrantPool::Paid,
                &request,
                Utc::now(),
            )
            .await?;
            line_allocation::record_line_allocation_tx(
                conn,
                invoice.tenant_id,
                invoice.id,
                Some(note.id),
                &lines,
                &of_line,
                &request,
                &placed,
                |line| line.amount_total.saturating_neg(),
            )
            .await?;
            (
                note.credited_amount_cents
                    .checked_sub(given_back)
                    .ok_or_else(|| invalid("credited amount overflow"))?,
                note.currency.clone(),
            )
        } else {
            (note.credited_amount_cents, note.currency.clone())
        };
    if amount == 0 {
        return Ok(());
    }
    let cost = CreditCost::at_face(amount, &currency);
    let now = Utc::now();
    let credited_at = credit_at.map_or(now, |at| at.min(now));
    credit::fund_tx(
        conn,
        controls,
        CreditFunding {
            tenant_id: invoice.tenant_id,
            customer_id: note.customer_id,
            currency,
            scope: GrantScope::customer_wide(),
            amount_minor: amount,
            classification: GrantClassification::Paid,
            priority: credit::DEFAULT_PRIORITY,
            effective_at: credited_at,
            expires_at: None,
            occurred_at: credited_at,
            now,
            source_kind: source::CREDIT_NOTE,
            source_id: note.id.as_uuid(),
            cost: Some(cost),
            rollover: None,
            invoice_id: Some(invoice.id),
            line_local_id: None,
            credit_note_id: Some(note.id),
            reason: credit_note_grant_reason(note.reason.as_deref()).to_owned(),
            reference: None,
            created_by: actor.as_uuid(),
            pays_usage_from: None,
        },
    )
    .await?;
    Ok(())
}

/// The promotional credit the credited lines took before tax goes back to the grants it came from,
/// whatever the note does with the money: the charge it paid is reversed. Value given back beyond
/// what the invoice still holds (an earlier note's rounding) is not recreated.
async fn return_promotional_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice: &Invoice,
    note: &CreditNote,
) -> StoreResult<()> {
    let (lines, owners) = owned_note_lines_tx(conn, invoice, note).await?;
    let (request, _) = pretax_request(
        &lines,
        LineScope {
            owners: &owners,
            subscription_id: invoice.subscription_id,
        },
        |line| line.promotional_credit.unwrap_or(0).saturating_neg(),
    )?;
    let request = request.undated();
    if request.requested_minor <= 0 {
        return Ok(());
    }
    let (given_back, _) = credit::return_draws_for_note_tx(
        conn,
        controls,
        lock,
        invoice.id,
        &invoice.currency,
        note.id,
        GrantPool::Promotional,
        &request,
        Utc::now(),
    )
    .await?;
    if given_back > 0 {
        crate::services::credit::follow_credit_given_back_tx(conn, controls, lock, invoice.id)
            .await?;
    }
    Ok(())
}

/// The note's lines, each billing the subscription of the invoice line it credits: a draft an
/// older release wrote may have dropped what the line recorded, and credit only goes back to the
/// grants that could pay the original line.
async fn owned_note_lines_tx(
    conn: &mut PgConn,
    invoice: &Invoice,
    note: &CreditNote,
) -> StoreResult<(Vec<LineItem>, LineOwners)> {
    let invoice_owners = LineOwners::load_tx(conn, invoice.tenant_id, &invoice.line_items).await?;
    let lines = owned_note_lines(
        &note.line_items,
        &invoice.line_items,
        invoice.subscription_id,
        &invoice_owners,
    );
    let owners = LineOwners::load_tx(conn, invoice.tenant_id, &lines).await?;
    Ok((lines, owners))
}

pub(crate) fn owned_note_lines(
    note_lines: &[LineItem],
    invoice_lines: &[LineItem],
    invoice_subscription: Option<SubscriptionId>,
    invoice_owners: &LineOwners,
) -> Vec<LineItem> {
    note_lines
        .iter()
        .map(|line| {
            let mut owned = line.clone();
            if let Some(original) = invoice_lines
                .iter()
                .find(|original| original.local_id == line.local_id)
            {
                owned.subscription_id =
                    line_subscription(original, invoice_subscription, invoice_owners);
            }
            owned
        })
        .collect()
}

/// Cumulative across refunds, so partial refunds never return more than the scalar deducted.
async fn historical_refund_return_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    note: &CreditNote,
) -> StoreResult<(i64, String)> {
    let deductions = HistoricalBalanceTxRow::deductions_for_invoice(
        conn,
        invoice.tenant_id,
        invoice.customer_id,
        invoice.id,
    )
    .await?;
    let (original, currency) = historical_deduction(&deductions)?.ok_or_else(|| {
        precondition("the credit this invoice drew before the ledger existed has no record")
    })?;
    let returned = CustomerCreditGrantRow::lock_by_invoice(conn, lock, invoice.id)
        .await?
        .into_iter()
        .filter(|g| g.source_kind == source::CREDIT_NOTE)
        .collect::<Vec<_>>();
    let (mut prior_invoice, mut prior_original) = (0_i64, 0_i64);
    for prior in CreditNoteRow::list_by_invoice_id(conn, invoice.tenant_id, invoice.id).await? {
        if prior.id == note.id
            || prior.status != diesel_models::enums::CreditNoteStatus::Finalized
            || prior.credit_type != diesel_models::enums::CreditTypeEnum::Refund
            || prior.credited_amount_cents == 0
        {
            continue;
        }
        // No grant means the refund either rounded to nothing or returned through the retired
        // scalar, which only the latter records; that one cannot be restored exactly.
        let restored = match returned.iter().find(|g| g.credit_note_id == Some(prior.id)) {
            Some(grant) => grant.amount,
            None if HistoricalBalanceTxRow::note_credited_retired_balance(
                conn,
                invoice.tenant_id,
                prior.id,
            )
            .await? =>
            {
                return Err(precondition(
                    "an earlier refund of this invoice has no original-currency restoration record",
                ));
            }
            None => 0,
        };
        prior_invoice = prior_invoice
            .checked_add(prior.credited_amount_cents)
            .ok_or_else(|| invalid("historical invoice credit overflow"))?;
        prior_original = prior_original
            .checked_add(restored)
            .ok_or_else(|| invalid("historical invoice credit overflow"))?;
    }
    let amount = credit_note_effects::cumulative_return(
        original,
        invoice.applied_credits,
        prior_invoice,
        prior_original,
        note.credited_amount_cents,
    )?;
    Ok((amount, currency.to_owned()))
}

/// A refund takes back the credit its purchase lines bought, from the unspent grant of each line.
async fn reverse_purchased_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice: &Invoice,
    note: &CreditNote,
) -> StoreResult<()> {
    let reversals = purchased_credit_reversals_tx(conn, lock, invoice, note).await?;
    if !reversals.is_empty() {
        crate::services::credit::suspend_top_up_of_refunded_purchase_tx(
            conn,
            controls,
            lock,
            invoice.id,
            &note.currency,
            note.refunded_amount_cents,
        )
        .await?;
    }
    for (grant, reversed) in reversals {
        credit::reverse_tx(
            conn,
            controls,
            lock,
            grant.id,
            reversed,
            note.id,
            Utc::now(),
        )
        .await?;
    }
    Ok(())
}

/// The credit each purchased grant loses to a refund note, by its purchase line. A line sold
/// below face splits its reversal: its paid credit follows the money refunded, its bonus credit
/// takes the rest, as far as it is unspent (used bonus credit is neither refunded nor owed).
async fn purchased_credit_reversals_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    note: &CreditNote,
) -> StoreResult<Vec<(CustomerCreditGrantRow, i64)>> {
    let lines = purchase_lines_tx(conn, lock, invoice).await?;
    let earlier = if lines.iter().any(|line| line.bonus.is_some()) {
        crate::services::credit::mint::credited_by_finalized_notes_tx(conn, invoice, Some(note.id))
            .await?
    } else {
        HashMap::new()
    };
    let now = Utc::now();
    let mut reversals = Vec::new();
    for PurchaseLine { grant, bonus } in lines {
        let Some(line_id) = grant.line_local_id.clone() else {
            if note.line_items.iter().any(|l| l.amount_subtotal != 0) {
                return Err(precondition(
                    "the credit bought on this invoice is not tied to a line; adjust the customer's credit directly",
                ));
            }
            continue;
        };
        let (mut reversed, mut credited) = (0_i64, 0_i64);
        for line in note.line_items.iter().filter(|l| l.local_id == line_id) {
            let amount = match (line.kind, line.credit_grant_amount) {
                (LineItemKind::Credit, Some(amount)) => amount,
                // A purchase line of the retired balance stays Regular: its subtotal is its credit.
                _ => line.amount_subtotal,
            }
            .checked_abs()
            .ok_or_else(|| invalid("credit amount overflow"))?;
            reversed = reversed
                .checked_add(amount)
                .ok_or_else(|| invalid("refund amount overflow"))?;
            credited = line
                .amount_subtotal
                .checked_abs()
                .and_then(|amount| credited.checked_add(amount))
                .ok_or_else(|| invalid("refund amount overflow"))?;
        }
        if reversed <= 0 {
            continue;
        }
        let Some(bonus) = bonus else {
            reversals.push((grant, reversed));
            continue;
        };
        let sold = invoice
            .line_items
            .iter()
            .find(|line| line.local_id == line_id)
            .ok_or_else(|| precondition("the purchase line of a credit grant is missing"))?;
        let paid = super::paid_share_of_reversal(
            sold,
            earlier.get(&line_id).copied().unwrap_or(0),
            credited,
        )
        .ok_or_else(|| invalid("the refund of this purchase line cannot be prorated"))?;
        let unspent = if bonus.expires_at.is_some_and(|at| at <= now) {
            0
        } else {
            bonus.remaining
        };
        let from_bonus = reversed.saturating_sub(paid).clamp(0, unspent);
        if paid > 0 {
            reversals.push((grant, paid));
        }
        if from_bonus > 0 {
            reversals.push((bonus, from_bonus));
        }
    }
    Ok(reversals)
}

/// Applies the credit reversal of a cash refund the previous release finalized without the
/// ledger. Credit already spent is reported; a reversal is recorded once per grant.
pub(crate) async fn reverse_missed_refund_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant: TenantId,
    note_id: CreditNoteId,
) -> StoreResult<()> {
    let observed = CreditNoteRow::find_by_id(conn, tenant, note_id).await?;
    let (lock, invoice) = lock_credit_invoice(conn, tenant, observed.invoice_id).await?;
    let lock = &lock;
    let row = CreditNoteRow::find_by_id(conn, tenant, note_id).await?;
    if row.status != diesel_models::enums::CreditNoteStatus::Finalized
        || row.credit_type != diesel_models::enums::CreditTypeEnum::Refund
    {
        return Ok(());
    }
    let note: CreditNote = row.try_into()?;
    let done = CustomerCreditEntryRow::list_by_note(conn, tenant, note.id).await?;
    let reversals = purchased_credit_reversals_tx(conn, lock, &invoice, &note).await?;
    let takes_back = reversals.iter().any(|(grant, amount)| {
        grant.remaining > 0
            && *amount > 0
            && !done
                .iter()
                .any(|e| e.grant_id == grant.id && e.kind == LedgerEntryKind::Reversal.as_db_str())
    });
    if takes_back {
        crate::services::credit::suspend_top_up_of_refunded_purchase_tx(
            conn,
            controls,
            lock,
            invoice.id,
            &note.currency,
            note.refunded_amount_cents,
        )
        .await?;
    }
    // Per wallet: a grant, the value taken back now, and what the note leaves unrecovered.
    let mut wallets: Vec<(CustomerCreditGrantRow, i64, i64)> = Vec::new();
    for (grant, amount) in reversals {
        let recorded = done
            .iter()
            .filter(|e| e.grant_id == grant.id && e.kind == LedgerEntryKind::Reversal.as_db_str())
            .try_fold(0_i64, |sum, e| sum.checked_sub(e.amount))
            .ok_or_else(|| invalid("refund amount overflow"))?;
        let take = if recorded > 0 {
            0
        } else {
            amount.min(grant.remaining.max(0))
        };
        if take > 0 {
            credit::reverse_tx(conn, controls, lock, grant.id, take, note.id, Utc::now()).await?;
        }
        let unrecovered = (amount - recorded - take).max(0);
        match wallets
            .iter_mut()
            .find(|(first, _, _)| first.account_id == grant.account_id)
        {
            Some((_, taken, lost)) => {
                *taken = taken
                    .checked_add(take)
                    .ok_or_else(|| invalid("refund amount overflow"))?;
                *lost = lost
                    .checked_add(unrecovered)
                    .ok_or_else(|| invalid("refund amount overflow"))?;
            }
            None => wallets.push((grant, take, unrecovered)),
        }
    }
    for (grant, taken, unrecovered) in wallets {
        credit::report_unrecovered_credit_tx(
            conn,
            lock,
            &grant,
            Some(invoice.id),
            None,
            CreditUnrecoveredTrigger::CreditNote(note.id),
            taken,
            unrecovered,
        )
        .await?;
    }
    Ok(())
}

/// A purchase line's grant and, sold below face, the bonus credit funded beside it.
pub(super) struct PurchaseLine {
    pub grant: CustomerCreditGrantRow,
    pub bonus: Option<CustomerCreditGrantRow>,
}

/// The invoice's credit purchases, each with its bonus credit.
pub(super) async fn purchase_lines_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
) -> StoreResult<Vec<PurchaseLine>> {
    let grants = purchase_grants_tx(conn, lock, invoice).await?;
    let bonus_sources = grants
        .iter()
        .map(|grant| credit::bonus_source(grant.source_id))
        .collect::<Vec<_>>();
    let (bonuses, paid): (Vec<_>, Vec<_>) = grants
        .into_iter()
        .partition(|grant| bonus_sources.contains(&grant.source_id));
    Ok(paid
        .into_iter()
        .map(|grant| PurchaseLine {
            bonus: bonuses
                .iter()
                .find(|bonus| bonus.source_id == credit::bonus_source(grant.source_id))
                .cloned(),
            grant,
        })
        .collect())
}

/// Grants the invoice's credit purchases funded, bonus credit included.
pub(super) async fn purchase_grants_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
) -> StoreResult<Vec<CustomerCreditGrantRow>> {
    Ok(
        CustomerCreditGrantRow::lock_by_invoice(conn, lock, invoice.id)
            .await?
            .into_iter()
            .filter(|g| g.source_kind == source::CREDIT_PURCHASE)
            .collect(),
    )
}

/// Credit the voided invoice's own finalization granted (included credit, a negative total) goes
/// with it, with what that credit rolled over into. Refused while any of it was spent; value that
/// lapsed stays lapsed.
pub(crate) async fn take_back_invoice_granted_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    void_note_id: CreditNoteId,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    let originals = CustomerCreditGrantRow::list_by_invoice_sources(
        conn,
        tenant_id,
        invoice_id,
        &[source::INCLUDED_CREDIT, source::NEGATIVE_INVOICE],
    )
    .await?
    .into_iter()
    .map(|grant| grant.id)
    .collect::<Vec<_>>();
    if originals.is_empty() {
        return Ok(());
    }
    let mut ids = originals.clone();
    ids.extend(CustomerCreditGrantRow::list_rolled_into(conn, tenant_id, &originals).await?);
    let mut granted = CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await?;
    // A grant rolls only under its own lock: one that rolled while we waited is visible now.
    let late = CustomerCreditGrantRow::list_rolled_into(conn, tenant_id, &originals)
        .await?
        .into_iter()
        .filter(|id| !ids.contains(id))
        .collect::<Vec<_>>();
    if !late.is_empty() {
        granted.extend(CustomerCreditGrantRow::lock_by_ids(conn, lock, &late).await?);
    }
    for grant in granted {
        if credit::applied_to_invoices_tx(conn, tenant_id, grant.id).await? > 0 {
            return Err(precondition(
                "credit this invoice granted already pays other invoices; void those first or cancel this invoice's debt with a credit note",
            ));
        }
        if grant.remaining > 0 {
            credit::reverse_tx(
                conn,
                controls,
                lock,
                grant.id,
                grant.remaining,
                void_note_id,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

/// A voided invoice gives back the credit it drew. One finalized before the ledger existed gives
/// back what the retired scalar deducted, in the currency it was deducted in (never converted).
pub(crate) async fn restore_invoice_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice: &Invoice,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let drawn =
        credit::reverse_invoice_draws_tx(conn, controls, lock, invoice.id, &invoice.currency, now)
            .await?;
    let expected = invoice
        .applied_credits
        .checked_add(crate::services::credit::promotional::quoted_total(
            &invoice.line_items,
        )?)
        .ok_or_else(|| invalid("invoice credit overflow"))?;
    if drawn.is_some() || expected == 0 {
        if drawn.unwrap_or(0) != expected {
            return Err(precondition(
                "the invoice's applied credit differs from the credit it drew",
            ));
        }
        return Ok(());
    }
    let deductions = HistoricalBalanceTxRow::deductions_for_invoice(
        conn,
        invoice.tenant_id,
        invoice.customer_id,
        invoice.id,
    )
    .await?;
    let (amount, currency) = historical_deduction(&deductions)?.ok_or_else(|| {
        precondition("the credit this invoice drew before the ledger existed has no record")
    })?;
    credit::fund_tx(
        conn,
        controls,
        CreditFunding {
            tenant_id: invoice.tenant_id,
            customer_id: invoice.customer_id,
            currency: currency.to_owned(),
            scope: GrantScope::customer_wide(),
            amount_minor: amount,
            classification: GrantClassification::Paid,
            priority: credit::DEFAULT_PRIORITY,
            effective_at: Utc::now(),
            expires_at: None,
            occurred_at: Utc::now(),
            now: Utc::now(),
            source_kind: source::INVOICE_VOID_RESTORE,
            source_id: invoice.id.as_uuid(),
            cost: Some(CreditCost::at_face(amount, currency)),
            rollover: None,
            invoice_id: Some(invoice.id),
            line_local_id: None,
            credit_note_id: None,
            reason: "Credit returned by a voided invoice".to_owned(),
            reference: None,
            created_by: None,
            pays_usage_from: None,
        },
    )
    .await?;
    Ok(())
}

/// Total the retired scalar deducted for one invoice, and the one currency it was taken in
/// (the customer's currency: main converted every deduction to it before recording).
pub(crate) fn historical_deduction(
    rows: &[HistoricalBalanceTxRow],
) -> StoreResult<Option<(i64, &str)>> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let currency = first.currency.as_str();
    let mut amount = 0_i64;
    for row in rows {
        if row.amount_cents >= 0 || row.currency != currency {
            return Err(precondition(
                "the invoice's historical credit deductions have inconsistent currencies or signs",
            ));
        }
        amount = row
            .amount_cents
            .checked_neg()
            .and_then(|value| amount.checked_add(value))
            .ok_or_else(|| {
                precondition("the invoice's historical credit exceeds the supported amount")
            })?;
    }
    Ok(Some((amount, currency)))
}

/// The name of the balance a credit note funds: why the system issued it, else a generic one.
/// A reason typed on a note stays out: grant names reach the customer portal.
pub(crate) fn credit_note_grant_reason(note_reason: Option<&str>) -> &'static str {
    use crate::domain::credit::{
        CANCELLATION_NOTE_REASON, LATE_PAYMENT_NOTE_REASON, PLAN_CHANGE_NOTE_REASON,
    };
    match note_reason {
        Some(LATE_PAYMENT_NOTE_REASON) => LATE_PAYMENT_NOTE_REASON,
        Some(PLAN_CHANGE_NOTE_REASON) => "Credited back on plan change",
        Some(CANCELLATION_NOTE_REASON) => "Credited back on cancellation",
        _ => "Credit returned by a credit note",
    }
}

#[cfg(test)]
mod tests {
    use super::historical_deduction;
    use diesel_models::customer_credit::HistoricalBalanceTxRow;

    fn row(amount_cents: i64, currency: &str) -> HistoricalBalanceTxRow {
        HistoricalBalanceTxRow {
            id: uuid::Uuid::new_v4(),
            amount_cents,
            currency: currency.into(),
        }
    }

    #[test]
    fn exact_original_minor_units_are_summed_without_fx() {
        assert_eq!(
            historical_deduction(&[row(-123, "JPY"), row(-456, "JPY")]).unwrap(),
            Some((579, "JPY"))
        );
        assert_eq!(historical_deduction(&[]).unwrap(), None);
    }

    #[test]
    fn mixed_currencies_signs_and_overflow_are_rejected() {
        assert!(historical_deduction(&[row(-1, "USD"), row(-1, "EUR")]).is_err());
        assert!(historical_deduction(&[row(1, "USD")]).is_err());
        assert!(historical_deduction(&[row(i64::MIN, "USD")]).is_err());
        assert!(historical_deduction(&[row(-i64::MAX, "USD"), row(-1, "USD")]).is_err());
    }
}

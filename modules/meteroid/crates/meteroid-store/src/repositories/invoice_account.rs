//! An issued invoice's account (`billing_core::settlement`), loaded under the invoice lock, and its
//! state written: the only writer of an issued invoice's `amount_due` and `payment_status` (R-125).
use crate::StoreResult;
use crate::errors::StoreError;
use crate::store::PgConn;
use billing_core::settlement::{
    Account, Carry, Charge, Collection, MoveStatus, Note, NoteKind, NoteStatus, PaymentStatus,
    Refund, State,
};
use chrono::NaiveDateTime;
use common_domain::ids::BaseId;
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::enums::{
    CreditNoteStatus, CreditTypeEnum, InvoicePaymentStatus, InvoiceStatusEnum, PaymentStatusEnum,
    PaymentTypeEnum,
};
use diesel_models::invoice_carry_forward::InvoiceCarryForwardRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;

/// What settling an invoice wrote.
#[derive(Debug)]
pub(crate) struct Settled {
    pub lock: CustomerLock,
    pub row: InvoiceRow,
    pub state: State,
    /// This settlement made the invoice paid: the only moment `invoice_paid` is owed.
    pub paid_now: bool,
    /// The invoice was paid and owes again.
    pub reopened: bool,
}

/// The account of `invoice`, read under its lock.
pub(crate) async fn load_tx(conn: &mut PgConn, invoice: &InvoiceRow) -> StoreResult<Account> {
    let payments = PaymentTransactionRow::list_by_invoice_id(conn, invoice.id, invoice.tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| row.transaction)
        .collect::<Vec<_>>();
    let notes = CreditNoteRow::list_by_invoice_id(conn, invoice.tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let carry = carry_tx(conn, invoice).await?;
    Ok(account_of(invoice, &payments, &notes, carry))
}

/// The debts `invoice` took and carried (R-129).
pub async fn carry_tx(conn: &mut PgConn, invoice: &InvoiceRow) -> StoreResult<Carry> {
    let links = InvoiceCarryForwardRow::list_links(conn, invoice.tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut carry = Carry::default();
    for link in links {
        if !link.outgoing {
            carry.taken += link.amount;
            continue;
        }
        carry.carried += link.amount;
        if let Some(later) = link.invoice_id {
            carry.carried_paid = InvoiceRow::find_by_id(conn, invoice.tenant_id, later)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .payment_status
                == InvoicePaymentStatus::Paid;
        }
    }
    Ok(carry)
}

/// What a charge of `invoice` may collect now (R-127). The caller holds the invoice lock.
pub(crate) async fn charge_tx(conn: &mut PgConn, invoice: &InvoiceRow) -> StoreResult<Charge> {
    Ok(load_tx(conn, invoice).await?.charge())
}

/// Finalized or uncollectible: what it owes is its account's (I30). A draft keeps its estimate.
pub(crate) fn is_issued(invoice: &InvoiceRow) -> bool {
    matches!(
        invoice.status,
        InvoiceStatusEnum::Finalized | InvoiceStatusEnum::Uncollectible
    )
}

/// A payment owns the invoice: one is in flight, or the cash collected leaves nothing due. Its
/// amounts are then never edited, refreshed or merged; credit alone covering it owns nothing.
pub(crate) fn owned_by_payment(state: &State) -> bool {
    state.in_flight > 0 || (state.due == 0 && state.cash > 0)
}

/// The account of `invoice` from its payment rows and credit notes, folded in the order they
/// happened (collections by creation, notes by finalization).
pub fn account_of(
    invoice: &InvoiceRow,
    payments: &[PaymentTransactionRow],
    notes: &[CreditNoteRow],
    carry: Carry,
) -> Account {
    let mut payments = payments.iter().collect::<Vec<_>>();
    payments.sort_by_key(|row| (row.created_at, row.id));
    let mut notes = notes.iter().collect::<Vec<_>>();
    notes.sort_by_key(|note| {
        (
            note.finalized_at.is_none(),
            note.finalized_at,
            note.created_at,
        )
    });
    let collections = payments
        .iter()
        .filter(|row| row.payment_type.is_collection())
        .map(|row| Collection {
            amount: row.amount,
            reversed: row.amount_reversed,
            disputed: row.amount_disputed,
            status: move_status(&row.status),
            accepted: row.accepted_at.is_some(),
            // A hosted page's attempt always has a provider id: one without a saved method or a
            // provider is a decline recorded on our side (a staged sandbox failure).
            declined: (row.payment_method_id.is_some() || row.provider_transaction_id.is_none())
                && row.decline_kind != Some(diesel_models::enums::DeclineKindEnum::Unconfirmed),
        })
        .collect();
    let refunds = payments
        .iter()
        .filter(|row| row.payment_type == PaymentTypeEnum::Refund)
        .map(|row| Refund {
            amount: row.amount,
            status: move_status(&row.status),
            note: row.credit_note_id.map(|id| id.as_uuid()),
            excess: row.refund_excess,
        })
        .collect();
    let notes = notes
        .iter()
        .map(|note| Note {
            id: note.id.as_uuid(),
            kind: match note.credit_type {
                CreditTypeEnum::CreditToBalance => NoteKind::CreditToBalance,
                CreditTypeEnum::Refund => NoteKind::Refund,
                CreditTypeEnum::DebtCancellation => NoteKind::DebtCancellation,
            },
            status: match note.status {
                CreditNoteStatus::Draft => NoteStatus::Draft,
                CreditNoteStatus::Finalized => NoteStatus::Finalized,
                CreditNoteStatus::Voided => NoteStatus::Voided,
            },
            total: note.total.saturating_abs(),
            credited: note.credited_amount_cents,
            refunded: note.refunded_amount_cents,
            debt_cancelled: note.debt_cancelled_amount_cents,
            voids_invoice: note.voids_invoice,
        })
        .collect();
    Account {
        total: invoice.total,
        drawn: invoice.applied_credits,
        collections,
        refunds,
        notes,
        carry,
    }
}

fn move_status(status: &PaymentStatusEnum) -> MoveStatus {
    match status {
        PaymentStatusEnum::Ready => MoveStatus::Ready,
        PaymentStatusEnum::Pending => MoveStatus::Pending,
        PaymentStatusEnum::Settled => MoveStatus::Settled,
        PaymentStatusEnum::Refunded => MoveStatus::Refunded,
        PaymentStatusEnum::Failed => MoveStatus::Failed,
        PaymentStatusEnum::Cancelled => MoveStatus::Cancelled,
    }
}

pub fn payment_status(status: PaymentStatus) -> InvoicePaymentStatus {
    match status {
        PaymentStatus::Unpaid => InvoicePaymentStatus::Unpaid,
        PaymentStatus::PartiallyPaid => InvoicePaymentStatus::PartiallyPaid,
        PaymentStatus::Paid => InvoicePaymentStatus::Paid,
        PaymentStatus::Errored => InvoicePaymentStatus::Errored,
        PaymentStatus::Processing => InvoicePaymentStatus::Processing,
        PaymentStatus::Carried => InvoicePaymentStatus::Carried,
    }
}

/// Locks the invoice, folds its account and writes its state. A paid invoice staying paid keeps
/// its `paid_at`; one becoming paid takes `paid_at` (now by default). Call it after writing what
/// moved the money, inside the same transaction.
pub(crate) async fn settle_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    paid_at: Option<NaiveDateTime>,
) -> StoreResult<Settled> {
    let locked = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let account = load_tx(conn, &locked.invoice).await?;
    let state = account.state();
    // A draft keeps its estimate (R-087) and a void what it was voided at: only an issued
    // invoice's money is written, so a payment landing on a void is overpayment, nothing more.
    if !is_issued(&locked.invoice) {
        return Ok(Settled {
            lock: locked.lock,
            row: locked.invoice,
            state,
            paid_now: false,
            reopened: false,
        });
    }
    let was_paid = locked.invoice.payment_status == InvoicePaymentStatus::Paid;
    let status = payment_status(state.status);
    let row = InvoiceRow::write_settlement(
        conn,
        &locked.lock,
        invoice_id,
        state.due,
        status.clone(),
        (status == InvoicePaymentStatus::Paid && !was_paid)
            .then(|| paid_at.unwrap_or_else(|| common_clock::now().naive_utc())),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let paid_now = !was_paid && status == InvoicePaymentStatus::Paid;
    let reopened = was_paid && status != InvoicePaymentStatus::Paid;
    if paid_now || reopened {
        Box::pin(settle_carried_sources_tx(conn, &row)).await?;
    }
    Ok(Settled {
        lock: locked.lock,
        row,
        state,
        paid_now,
        reopened,
    })
}

/// The invoices whose debt `invoice` took are paid with it, at its payment time, and carried
/// again once it is not (R-129).
async fn settle_carried_sources_tx(conn: &mut PgConn, invoice: &InvoiceRow) -> StoreResult<()> {
    let sources = InvoiceCarryForwardRow::sources_of(conn, invoice.tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for source in sources {
        let settled = settle_tx(conn, invoice.tenant_id, source, invoice.paid_at).await?;
        if settled.paid_now {
            let paid: crate::domain::Invoice = settled.row.try_into()?;
            crate::repositories::outbox::publish_outbox_events_tx(
                conn,
                vec![crate::domain::outbox_event::OutboxEvent::invoice_paid(
                    (&paid).into(),
                )],
            )
            .await?;
        }
    }
    Ok(())
}

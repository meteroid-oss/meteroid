//! Applies the credit reversal that instances of the previous release skipped during a rolling
//! deploy (on purchases and on payments kept as credit), and re-closes a disputed invoice one of
//! them reopened. Removed with the guard triggers on the retired balance tables, once no
//! pre-ledger instance can run.
use super::metrics::{self, ReconciliationKind};
use super::payment_reversal::reconcile_reversed_purchase_tx;
use crate::Store;
use crate::StoreResult;
use crate::repositories::credit_notes::reverse_missed_refund_credit_tx;
use common_domain::ids::{CreditNoteId, InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::query::credit_reversal_sweep::{
    DisputedUnpaidInvoice, KeptPaymentOutOfLine, ReversedPurchaseInvoice, UnreversedRefundNote,
};
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

/// Reconciles every purchase the previous release left out of line, `page` at a time; returns how
/// many were brought in line. One failing document is logged and never blocks the others.
pub(crate) async fn reconcile_reversed_credit_purchases(
    store: &Store,
    page: i64,
) -> StoreResult<usize> {
    let page = page.max(1);
    let mut applied = 0_usize;

    let mut after = (
        TenantId::from_const(Uuid::nil()),
        InvoiceId::from_const(Uuid::nil()),
    );
    loop {
        let mut conn = store.get_conn().await?;
        let batch = ReversedPurchaseInvoice::scan(&mut conn, after, page).await?;
        drop(conn);
        for candidate in &batch {
            after = (candidate.tenant_id, candidate.invoice_id);
            if !candidate.out_of_sync {
                continue;
            }
            let (tenant_id, invoice_id) = after;
            match store
                .transaction(|conn| {
                    async move {
                        reconcile_reversed_purchase_tx(
                            conn,
                            &*store.hooks.credit,
                            tenant_id,
                            invoice_id,
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await
            {
                Ok(()) => {
                    applied += 1;
                    metrics::reconciled(ReconciliationKind::ReversedPurchase);
                }
                Err(err) => log::error!(
                    "reconciling reversed credit purchase invoice {invoice_id} of tenant {tenant_id} failed: {err:?}"
                ),
            }
        }
        if (batch.len() as i64) < page {
            break;
        }
    }

    let mut after = (
        TenantId::from_const(Uuid::nil()),
        CreditNoteId::from_const(Uuid::nil()),
    );
    loop {
        let mut conn = store.get_conn().await?;
        let batch = UnreversedRefundNote::list(&mut conn, after, page).await?;
        drop(conn);
        for candidate in &batch {
            after = (candidate.tenant_id, candidate.credit_note_id);
            let (tenant_id, note_id) = after;
            match store
                .transaction(|conn| {
                    async move {
                        reverse_missed_refund_credit_tx(
                            conn,
                            &*store.hooks.credit,
                            tenant_id,
                            note_id,
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await
            {
                Ok(()) => {
                    applied += 1;
                    metrics::reconciled(ReconciliationKind::RefundNote);
                }
                Err(err) => log::error!(
                    "reversing the credit of refund credit note {note_id} of tenant {tenant_id} failed: {err:?}"
                ),
            }
        }
        if (batch.len() as i64) < page {
            break;
        }
    }

    let mut after = (
        TenantId::from_const(Uuid::nil()),
        PaymentTransactionId::from_const(Uuid::nil()),
    );
    loop {
        let mut conn = store.get_conn().await?;
        let batch = KeptPaymentOutOfLine::list(&mut conn, after, page).await?;
        drop(conn);
        for candidate in &batch {
            after = (candidate.tenant_id, candidate.payment_id);
            let (tenant_id, payment_id) = after;
            if !candidate.ledger_out {
                let mut conn = store.get_conn().await?;
                let reported = diesel_models::customer_credit::CreditUnrecoveredRow::reported(
                    &mut conn,
                    tenant_id,
                    candidate.account_id,
                    super::collected_payment::payment_flow(payment_id),
                )
                .await?;
                if reported == candidate.unrecovered {
                    continue;
                }
            }
            match store
                .transaction(|conn| {
                    async move {
                        super::collected_payment::follow_collected_payment_tx(
                            conn,
                            &*store.hooks.credit,
                            tenant_id,
                            payment_id,
                            None,
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await
            {
                Ok(()) => {
                    applied += 1;
                    metrics::reconciled(ReconciliationKind::KeptPayment);
                }
                Err(err) => log::error!(
                    "following the credit kept from payment {payment_id} of tenant {tenant_id} failed: {err:?}"
                ),
            }
        }
        if (batch.len() as i64) < page {
            break;
        }
    }

    let mut after = (
        TenantId::from_const(Uuid::nil()),
        InvoiceId::from_const(Uuid::nil()),
    );
    loop {
        let mut conn = store.get_conn().await?;
        let batch = DisputedUnpaidInvoice::list(&mut conn, after, page).await?;
        drop(conn);
        for candidate in &batch {
            after = (candidate.tenant_id, candidate.invoice_id);
            let (tenant_id, invoice_id) = after;
            match store
                .transaction(|conn| {
                    async move { reclose_disputed_invoice_tx(conn, tenant_id, invoice_id).await }
                        .scope_boxed()
                })
                .await
            {
                Ok(true) => {
                    applied += 1;
                    metrics::reconciled(ReconciliationKind::DisputedInvoice);
                }
                Ok(false) => {}
                Err(err) => log::error!(
                    "re-closing disputed invoice {invoice_id} of tenant {tenant_id} failed: {err:?}"
                ),
            }
        }
        if (batch.len() as i64) < page {
            break;
        }
    }
    Ok(applied)
}

/// Re-derives the invoice when what it shows owed is not what its money says: a dispute never
/// makes it owed again. Returns whether it changed.
async fn reclose_disputed_invoice_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> StoreResult<bool> {
    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
        .await?
        .invoice;
    let owed = InvoiceRow::collections(conn, invoice_id, tenant_id)
        .await?
        .amount_due();
    if owed == invoice.amount_due {
        return Ok(false);
    }
    crate::repositories::payment_transactions::recompute_invoice_payment_state(
        conn,
        tenant_id,
        invoice_id,
        invoice.paid_at,
    )
    .await?;
    log::warn!(
        "disputed invoice {invoice_id} of tenant {tenant_id} re-closed after a mixed-version reopen"
    );
    Ok(true)
}

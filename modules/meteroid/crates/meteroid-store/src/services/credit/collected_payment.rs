//! Collected money no invoice took is kept as paid OVERPAYMENT credit: customer-wide, for any
//! charge, never expiring, dated when the money was collected. A payment for a plan change that
//! could no longer apply is kept whole, keyed by the payment's id; what payments collect above
//! their invoice asks is kept per invoice (`reconcile_overpayment_tx`). Both follow the money.
use crate::StoreResult;
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope, LedgerEntryKind};
use crate::domain::wallet_events::{CreditUnrecoveredCause, CreditUnrecoveredTrigger};
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::customer_credit::{
    self as credit, CreditFunding, PaymentMovement, balance_changed_tx, dispute_follow_up_tx,
    follow_payment_tx, report_unrecovered_credit_tx, source,
};
use crate::store::PgConn;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::customer_credit::{CustomerCreditEntryRow, CustomerCreditGrantRow};
use diesel_models::invoices::{InvoiceLockRow, InvoiceRow};
use diesel_models::lock::CustomerLock;
use diesel_models::payments::PaymentTransactionRow;

/// A settled (or reinstated) payment and when, in business time, its money was collected.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Collection {
    pub payment: PaymentTransactionId,
    pub at: DateTime<Utc>,
}

/// Keeps `amount` of `payment` as paid credit of the customer, once per payment.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn keep_collected_payment_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment: PaymentTransactionId,
    invoice_id: Option<InvoiceId>,
    amount: i64,
    currency: &str,
    collected_at: DateTime<Utc>,
    reason: &str,
) -> StoreResult<()> {
    if amount <= 0
        || CustomerCreditGrantRow::find_by_source(
            conn,
            tenant_id,
            source::OVERPAYMENT,
            payment.as_uuid(),
        )
        .await?
        .is_some()
    {
        return Ok(());
    }
    let now = common_domain::clock::now();
    credit::fund_tx(
        conn,
        controls,
        overpayment_funding(
            tenant_id,
            customer_id,
            currency,
            amount,
            collected_at.min(now),
            now,
            payment.as_uuid(),
            invoice_id,
            reason,
            Some(payment),
        ),
    )
    .await?;
    log::warn!("payment {payment}: {amount} kept as customer credit ({reason})");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn overpayment_funding(
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    amount: i64,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    source_id: uuid::Uuid,
    invoice_id: Option<InvoiceId>,
    reason: &str,
    paid_by: Option<PaymentTransactionId>,
) -> CreditFunding {
    CreditFunding {
        tenant_id,
        customer_id,
        currency: currency.to_owned(),
        scope: GrantScope::customer_wide(),
        amount_minor: amount,
        classification: GrantClassification::Paid,
        priority: credit::DEFAULT_PRIORITY,
        effective_at: at,
        expires_at: None,
        occurred_at: at,
        now,
        source_kind: source::OVERPAYMENT,
        source_id,
        cost: Some(CreditCost::at_face(amount, currency)),
        rollover: None,
        invoice_id,
        line_local_id: None,
        credit_note_id: None,
        reason: reason.to_owned(),
        reference: paid_by.map(|payment| payment.to_string()),
        created_by: None,
        pays_usage_from: None,
    }
}

/// Aligns the credit kept whole from `payment` with what it still collects: what was reversed or
/// refunded without a note is taken back first, restored when reinstated or the refund fails;
/// spent credit is reported unrecovered, keyed by the payment. `moved` is what changed it, if any.
pub(crate) async fn follow_collected_payment_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    payment: PaymentTransactionId,
    moved: Option<&PaymentMovement>,
) -> StoreResult<()> {
    // Credit kept from an invoice's overpayment follows the invoice (`reconcile_overpayment_tx`).
    let Some(kept) = CustomerCreditGrantRow::find_by_source(
        conn,
        tenant_id,
        source::OVERPAYMENT,
        payment.as_uuid(),
    )
    .await?
    .filter(|kept| kept.invoice_id.is_none()) else {
        return Ok(());
    };
    let row = PaymentTransactionRow::get_by_id(conn, payment, tenant_id).await?;
    let lock = match row.invoice_id {
        Some(invoice_id) => {
            InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                .await?
                .lock
        }
        None => credit::lock_customer_tx(conn, tenant_id, kept.customer_id).await?,
    };
    let Some(grant) = CustomerCreditGrantRow::lock_by_ids(conn, &lock, &[kept.id])
        .await?
        .into_iter()
        .next()
    else {
        return Ok(());
    };
    let refunded =
        PaymentTransactionRow::sum_live_refunds_without_note_by_parent(conn, payment, tenant_id)
            .await?;
    let taken = taken_back(conn, tenant_id, &grant, None).await?;
    let target = row
        .amount_reversed
        .saturating_add(refunded)
        .clamp(0, grant.amount);
    let delta = if target > taken {
        -(target - taken).min(grant.remaining)
    } else {
        (taken - target).min(grant.amount - grant.remaining)
    };
    let now = common_domain::clock::now();
    if delta != 0 {
        let follow_up = match moved {
            None => dispute_follow_up_tx(conn, tenant_id, None, Some(payment), now).await?,
            Some(_) => None,
        };
        let linked = moved.or(follow_up.as_ref());
        let wallet = follow_payment_tx(
            conn,
            grant.id,
            delta,
            grant.invoice_id,
            linked,
            linked.map_or_else(
                || payment_loss_cause(&row, refunded),
                PaymentMovement::cause,
            ),
            now,
        )
        .await?;
        balance_changed_tx(conn, controls, tenant_id, &[wallet], now).await?;
    }
    let unrecovered = (target - (taken - delta)).max(0);
    let cause = moved.map_or_else(
        || payment_loss_cause(&row, refunded),
        PaymentMovement::cause,
    );
    report_unrecovered_credit_tx(
        conn,
        &lock,
        &grant,
        row.invoice_id,
        Some(payment),
        CreditUnrecoveredTrigger::Overpayment(payment_flow(payment), cause),
        (-delta).max(0),
        unrecovered,
    )
    .await
}

/// How overpayment credit moved by `linked` is worded: as a dispute or a bank reversal.
fn linked_cause(linked: Option<&PaymentMovement>) -> CreditUnrecoveredCause {
    if linked.is_some_and(|movement| movement.disputed) {
        CreditUnrecoveredCause::PaymentDisputed
    } else {
        CreditUnrecoveredCause::PaymentReversed
    }
}

/// What took a payment's money, from what the row records and `refunded` without a note.
fn payment_loss_cause(row: &PaymentTransactionRow, refunded: i64) -> CreditUnrecoveredCause {
    if row.amount_reversed <= 0 && refunded > 0 {
        CreditUnrecoveredCause::PaymentRefunded
    } else if row.amount_disputed > 0 && row.amount_disputed >= row.amount_reversed {
        CreditUnrecoveredCause::PaymentDisputed
    } else {
        CreditUnrecoveredCause::PaymentReversed
    }
}

/// The key of the money flow a payment's kept credit is reported on, apart from any invoice's.
pub(crate) fn payment_flow(payment: PaymentTransactionId) -> uuid::Uuid {
    uuid::Uuid::new_v5(&payment.as_uuid(), b"payment-credit")
}

/// Credit given back to grants kept from payments follows those payments again: what a reversal
/// left owed is taken back at once. The caller holds the customer's lock.
pub(crate) async fn follow_collected_given_back_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    grants: &[CustomerCreditGrantRow],
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    for grant in grants
        .iter()
        .filter(|grant| grant.source_kind == source::OVERPAYMENT)
    {
        if let Some(invoice_id) = grant.invoice_id {
            reconcile_overpayment_tx(
                conn,
                controls,
                tenant_id,
                invoice_id,
                None,
                CreditUnrecoveredCause::PaymentReversed,
                None,
            )
            .await?;
            continue;
        }
        let payment = PaymentTransactionId::from(grant.source_id);
        if PaymentTransactionRow::find_by_id(conn, payment, tenant_id)
            .await?
            .is_some()
        {
            follow_collected_payment_tx(conn, controls, tenant_id, payment, None).await?;
        }
    }
    Ok(())
}

/// Keeps the credit of an invoice equal to what its collections keep above what it asks (granted
/// less taken back): the difference is granted, restoring what was taken back first, or taken
/// back; what was spent is reported unrecovered, as `cause` took the money. Idempotent; locks the
/// customer, then the invoice.
pub(crate) async fn reconcile_overpayment_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    collection: Option<Collection>,
    cause: CreditUnrecoveredCause,
    moved: Option<&PaymentMovement>,
) -> StoreResult<()> {
    let InvoiceLockRow { lock, invoice } =
        InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id).await?;
    let target = InvoiceRow::overpaid_by_collections(conn, invoice_id, tenant_id).await?;
    let grants: Vec<_> = CustomerCreditGrantRow::lock_by_invoice(conn, &lock, invoice_id)
        .await?
        .into_iter()
        .filter(|grant| grant.source_kind == source::OVERPAYMENT)
        .collect();
    if grants.is_empty() && target == 0 {
        return Ok(());
    }
    let mut kept = Vec::with_capacity(grants.len());
    for grant in grants {
        let taken = taken_back(conn, tenant_id, &grant, Some(invoice_id)).await?;
        kept.push((grant, taken));
    }
    let net: i64 = kept.iter().map(|(grant, taken)| grant.amount - taken).sum();
    let mut diff = target - net;
    let now = common_domain::clock::now();
    let follow_up = match moved {
        None if diff != 0 => {
            dispute_follow_up_tx(conn, tenant_id, Some(invoice_id), None, now).await?
        }
        _ => None,
    };
    let linked = moved.or(follow_up.as_ref());
    let mut touched = Vec::new();
    let mut taken_now = 0;
    if diff < 0 {
        for (grant, _) in kept.iter().rev() {
            let take = (-diff).min(grant.remaining);
            if take > 0 {
                touched.push(
                    follow_payment_tx(
                        conn,
                        grant.id,
                        -take,
                        Some(invoice_id),
                        linked,
                        linked_cause(linked),
                        now,
                    )
                    .await?,
                );
                diff += take;
                taken_now += take;
            }
        }
    } else if diff > 0 {
        for (grant, taken) in &kept {
            let give = diff.min(*taken).min(grant.amount - grant.remaining);
            if give > 0 {
                touched.push(
                    follow_payment_tx(
                        conn,
                        grant.id,
                        give,
                        Some(invoice_id),
                        linked,
                        linked_cause(linked),
                        now,
                    )
                    .await?,
                );
                diff -= give;
            }
        }
        if diff > 0 {
            let paid_by = collection.map(|collection| collection.payment);
            let at = collection.map_or(now, |collection| collection.at.min(now));
            let source_id =
                overpayment_source_id(conn, tenant_id, invoice_id, paid_by, kept.len()).await?;
            credit::fund_tx(
                conn,
                controls,
                overpayment_funding(
                    tenant_id,
                    invoice.customer_id,
                    &invoice.currency,
                    diff,
                    at,
                    now,
                    source_id,
                    Some(invoice_id),
                    "Paid above what the invoice asked",
                    paid_by,
                ),
            )
            .await?;
            log::warn!("invoice {invoice_id}: {diff} paid above it kept as customer credit");
            diff = 0;
        }
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;
    // What the invoice still keeps above its target was spent before the money went back.
    if let Some((first, _)) = kept.first() {
        report_unrecovered_credit_tx(
            conn,
            &lock,
            first,
            Some(invoice_id),
            moved.map(|moved| moved.payment),
            CreditUnrecoveredTrigger::Overpayment(overpayment_flow(invoice_id), cause),
            taken_now,
            (-diff).max(0),
        )
        .await?;
    }
    Ok(())
}

/// What an invoice's overpayment credit already paid for: granted, less what reversals took back,
/// less what is left. A refund without a note may give back only the rest. The caller holds the
/// invoice.
pub(crate) async fn spent_overpayment_credit_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
) -> StoreResult<i64> {
    let tenant_id = lock.tenant_id();
    let mut spent = 0_i64;
    for grant in CustomerCreditGrantRow::lock_by_invoice(conn, lock, invoice_id)
        .await?
        .into_iter()
        .filter(|grant| grant.source_kind == source::OVERPAYMENT)
    {
        let taken = taken_back(conn, tenant_id, &grant, Some(invoice_id)).await?;
        spent += (grant.amount - taken - grant.remaining).max(0);
    }
    Ok(spent)
}

/// Net value reversals hold back from `grant`; for an invoice's grant, only the movements cited
/// to it, so credit an operator or a note moved stays as it is.
async fn taken_back(
    conn: &mut PgConn,
    tenant_id: TenantId,
    grant: &CustomerCreditGrantRow,
    invoice_id: Option<InvoiceId>,
) -> StoreResult<i64> {
    Ok(
        CustomerCreditEntryRow::list_by_grant(conn, tenant_id, grant.id)
            .await?
            .iter()
            .filter(|entry| invoice_id.is_none() || entry.invoice_id == invoice_id)
            .filter(|entry| entry.credit_note_id.is_none())
            .filter(|entry| {
                entry.kind == LedgerEntryKind::Reversal.as_db_str()
                    || entry.kind == LedgerEntryKind::Reinstatement.as_db_str()
            })
            .map(|entry| -entry.amount)
            .sum(),
    )
}

/// The key of the money flow an invoice's overpayment loss is reported on, apart from the loss
/// on its purchased credit (`CreditUnrecoveredTrigger::Payments`).
fn overpayment_flow(invoice_id: InvoiceId) -> uuid::Uuid {
    uuid::Uuid::new_v5(&invoice_id.as_uuid(), b"overpayment-credit")
}

/// The first credit kept for a payment is keyed by the payment, so a redelivered settle finds
/// it; any later one by the invoice, the payment and how many grants the invoice already keeps.
async fn overpayment_source_id(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    paid_by: Option<PaymentTransactionId>,
    kept: usize,
) -> StoreResult<uuid::Uuid> {
    if let Some(payment) = paid_by
        && CustomerCreditGrantRow::find_by_source(
            conn,
            tenant_id,
            source::OVERPAYMENT,
            payment.as_uuid(),
        )
        .await?
        .is_none()
    {
        return Ok(payment.as_uuid());
    }
    let cause = paid_by.map_or_else(|| "repricing".to_owned(), |payment| payment.to_string());
    Ok(uuid::Uuid::new_v5(
        &invoice_id.as_uuid(),
        format!("overpayment-credit:{cause}:{kept}").as_bytes(),
    ))
}

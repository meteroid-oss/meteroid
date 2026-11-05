//! Involuntary claw-backs of settled payments and the money handed back after them.
//!
//! A payment lost to a dispute or chargeback keeps its invoice paid: the loss is recorded on the
//! payment (`amount_disputed`) and the invoice (`payment_disputed_at`), and booked. A failed
//! collection (a direct debit returned by the bank) reopens the invoice instead. Neither ever takes
//! more than the payment still holds: what a provider withdraws above it is kept as excess.
use crate::StoreResult;
use crate::domain::PaymentIntent;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{PaymentStatusEnum as S, ReversalKind};
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::PaymentMovement;
use crate::repositories::payment_transactions::{
    PaymentTransactionInterface, ReversalAmount, TransactionReinstatement, TransactionReversal,
    recompute_invoice_payment_state,
};
use crate::services::credit::collected_payment::Collection;
use crate::services::payment_reminders::sweep::ReopenedReminders;
use crate::store::{PgConn, Store};
use chrono::{NaiveDateTime, SubsecRound, Utc};
use common_domain::ids::PaymentTransactionId;
use diesel_models::enums::{InvoicePaymentStatus, PaymentStatusEnum as DbStatus};
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payment_reversal_movements::{
    PaymentReversalMovementRow, PaymentReversalMovementRowNew,
};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use diesel_models::query::payment_reversal_movements::held_ref;
use error_stack::Report;
use uuid::Uuid;

/// What the payment still holds for a reversal: what neither a refund nor a refund in flight gave
/// back.
async fn holds_tx(conn: &mut PgConn, payment: &PaymentTransaction) -> StoreResult<i64> {
    let in_flight = PaymentTransactionRow::sum_refunds_by_parent(
        conn,
        payment.id,
        payment.tenant_id,
        &[DbStatus::Ready, DbStatus::Pending],
    )
    .await?;
    Ok((payment.amount - payment.amount_refunded - in_flight).max(0))
}

fn status_after(payment: &PaymentTransaction, reversed: i64) -> S {
    if payment.amount_refunded + reversed >= payment.amount {
        S::Refunded
    } else {
        S::Settled
    }
}

struct Movement {
    disputed: bool,
    amount: i64,
    excess: i64,
}

impl Store {
    pub(crate) async fn reverse_payment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        mut reversal: TransactionReversal,
    ) -> StoreResult<PaymentTransaction> {
        // Postgres keeps microseconds: an identical redelivered event must compare equal.
        reversal.reversed_at = reversal.reversed_at.trunc_subsecs(6);

        match transaction.status {
            // Reversed before we recorded its settlement: just a failed charge, nothing reopens.
            S::Pending | S::Ready => {
                let intent = PaymentIntent {
                    external_id: transaction
                        .provider_transaction_id
                        .clone()
                        .unwrap_or_else(|| reversal.external_transaction_id.clone()),
                    transaction_id: transaction.id,
                    tenant_id: transaction.tenant_id,
                    amount_requested: 0,
                    amount_received: None,
                    currency: String::new(),
                    next_action: None,
                    status: S::Failed,
                    last_payment_error: reversal.reason.clone(),
                    decline_kind: None,
                    processed_at: Some(reversal.reversed_at),
                };
                return self
                    .consolidate_intent_and_transaction_tx(conn, actor, transaction, intent)
                    .await;
            }
            // A refunded payment can still be disputed: what the provider takes is then excess.
            S::Settled | S::Refunded => {}
            _ => return Ok(transaction),
        }

        if self
            .already_applied_tx(
                conn,
                &transaction,
                reversal.provider_ref.as_deref(),
                reversal.reversed_at,
                true,
            )
            .await?
        {
            log::info!(
                "Transaction {} already applied this reversal ({:?}, {:?}); skipping",
                transaction.id,
                reversal.provider_ref,
                reversal.reason
            );
            return Ok(transaction);
        }

        let holds = holds_tx(conn, &transaction).await?;
        let before = transaction.amount_reversed + transaction.amount_reversed_excess;
        let mut withdrawn = match reversal.amount {
            ReversalAmount::Full => transaction.amount,
            ReversalAmount::Incremental(delta) => before.saturating_add(delta.max(0)),
        }
        .min(transaction.amount)
        .max(before);
        // Its reinstatement arrived first and was held: the two cancel out.
        if let Some(provider_ref) = reversal.provider_ref.as_deref() {
            let held =
                PaymentReversalMovementRow::held_reinstatement(conn, transaction.id, provider_ref)
                    .await?;
            let met = held.min(withdrawn - before).max(0);
            if met > 0 {
                let (lock, _) = PaymentTransactionRow::lock_with_customer(
                    conn,
                    transaction.tenant_id,
                    transaction.id,
                )
                .await?;
                record_movement_tx(
                    conn,
                    &lock,
                    &transaction,
                    &Movement {
                        disputed: reversal.disputed,
                        amount: 0,
                        excess: met,
                    },
                    Some(reversal.kind),
                    Some(held_ref(provider_ref)),
                    reversal.reversed_at,
                )
                .await?;
                log::warn!(
                    "Transaction {}: withdrawal {provider_ref} met by the reinstatement that arrived before it ({met} minor units)",
                    transaction.id
                );
                withdrawn -= met;
            }
        }
        let reversed = withdrawn.min(holds).max(transaction.amount_reversed);
        let movement = Movement {
            disputed: reversal.disputed,
            amount: reversed - transaction.amount_reversed,
            excess: (withdrawn - reversed) - transaction.amount_reversed_excess,
        };
        if movement.amount == 0 && movement.excess == 0 {
            return Ok(transaction);
        }

        let (lock, _) =
            PaymentTransactionRow::lock_with_customer(conn, transaction.tenant_id, transaction.id)
                .await?;
        let Some(moved) = record_movement_tx(
            conn,
            &lock,
            &transaction,
            &movement,
            Some(reversal.kind),
            reversal.provider_ref.clone(),
            reversal.reversed_at,
        )
        .await?
        else {
            return Ok(transaction);
        };

        let disputed = transaction.amount_disputed
            + if movement.disputed {
                movement.amount
            } else {
                0
            };
        let status = status_after(&transaction, reversed);
        let updated: PaymentTransaction = PaymentTransactionRowPatch {
            id: transaction.id,
            status: (status != transaction.status).then(|| status.clone().into()),
            // The reversal cycle's high-water mark, which redelivery guards key off.
            refunded_at: Some(Some(reversal.reversed_at)),
            amount_reversed: Some(reversed),
            amount_disputed: Some(disputed),
            amount_reversed_excess: Some(withdrawn - reversed),
            reversal_kind: Some(Some(reversal.kind.into())),
            reversal_reason: Some(reversal.reason.clone()),
            ..Default::default()
        }
        .update(conn)
        .await?
        .into();

        if movement.excess > 0 {
            log::error!(
                "Payment transaction {}: the provider withdrew {} {} minor units above what it still held ({} refunded or in flight); not applied, contest it with the provider (provider tx {})",
                updated.id,
                movement.excess,
                updated.currency,
                updated.amount - holds,
                reversal.external_transaction_id,
            );
            crate::services::credit::metrics::reversal_excess(
                &updated.currency,
                movement.disputed,
                movement.excess,
            );
        }
        log::error!(
            "Payment transaction {} {} ({:?}, {:?}): {} of {} {} minor units clawed back (provider tx {}); invoice {:?} {}",
            updated.id,
            if movement.disputed {
                "lost to a dispute"
            } else {
                "reversed"
            },
            reversal.kind,
            reversal.reason,
            reversed,
            updated.amount,
            updated.currency,
            reversal.external_transaction_id,
            updated.invoice_id,
            if movement.disputed {
                "stays paid"
            } else {
                "reopens"
            },
        );

        if movement.amount != 0 {
            self.follow_payment_movement_tx(
                conn,
                &updated,
                &moved,
                (!movement.disputed).then_some(reversal.kind),
                None,
                None,
            )
            .await?;
        }

        let mut events = vec![OutboxEvent::payment_reversed(updated.clone().into())];
        // An excess alone moved nothing the settled orchestration must see.
        if updated.amount_reversed > 0 {
            events.push(OutboxEvent::payment_transaction_saved(
                updated.clone().into(),
            ));
        }
        self.internal
            .record_outbox_batch_tx(conn, updated.tenant_id, actor, events)
            .await?;
        Ok(updated)
    }

    pub(crate) async fn reinstate_payment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        mut reinstatement: TransactionReinstatement,
    ) -> StoreResult<PaymentTransaction> {
        reinstatement.reinstated_at = reinstatement.reinstated_at.trunc_subsecs(6);

        let withdrawn = transaction.amount_reversed + transaction.amount_reversed_excess;
        // Arrived before its withdrawal: held until the withdrawal does, which it then cancels.
        if let Some(provider_ref) = reinstatement.provider_ref.as_deref()
            && matches!(transaction.status, S::Settled | S::Refunded)
            && !PaymentReversalMovementRow::applied(conn, transaction.id, provider_ref, true)
                .await?
            && !has_legacy_reversal_tx(conn, &transaction).await?
        {
            if !PaymentReversalMovementRow::applied(conn, transaction.id, provider_ref, false)
                .await?
            {
                let held = reinstatement
                    .reinstated_amount_minor
                    .unwrap_or(transaction.amount)
                    .clamp(0, transaction.amount);
                if held > 0 {
                    let (lock, _) = PaymentTransactionRow::lock_with_customer(
                        conn,
                        transaction.tenant_id,
                        transaction.id,
                    )
                    .await?;
                    record_movement_tx(
                        conn,
                        &lock,
                        &transaction,
                        &Movement {
                            disputed: true,
                            amount: 0,
                            excess: -held,
                        },
                        transaction.reversal_kind,
                        Some(provider_ref.to_owned()),
                        reinstatement.reinstated_at,
                    )
                    .await?;
                    log::warn!(
                        "Transaction {}: reinstatement {provider_ref} ({held} minor units) arrived before its withdrawal; held until it arrives",
                        transaction.id
                    );
                }
            }
            return Ok(transaction);
        }
        if !matches!(transaction.status, S::Settled | S::Refunded) || withdrawn <= 0 {
            log::info!(
                "Transaction {} has no recorded reversal (status {:?}); reinstatement ({}) is a no-op",
                transaction.id,
                transaction.status,
                reinstatement.reason,
            );
            return Ok(transaction);
        }
        if self
            .already_applied_tx(
                conn,
                &transaction,
                reinstatement.provider_ref.as_deref(),
                reinstatement.reinstated_at,
                false,
            )
            .await?
        {
            log::info!(
                "Transaction {} already applied this reinstatement ({}); skipping",
                transaction.id,
                reinstatement.reason,
            );
            return Ok(transaction);
        }

        let handed_back = reinstatement
            .reinstated_amount_minor
            .unwrap_or(withdrawn)
            .clamp(0, withdrawn);
        if handed_back == 0 {
            return Ok(transaction);
        }
        // The excess first (Stripe returns the refunded part of a dispute won on its refund), then
        // the disputed money, then a failed collection's.
        let from_excess = handed_back.min(transaction.amount_reversed_excess);
        let from_reversed = handed_back - from_excess;
        let from_disputed = from_reversed.min(transaction.amount_disputed);
        let from_failed = from_reversed - from_disputed;
        let excess_disputed = transaction.amount_disputed > 0;
        let movements = [
            Movement {
                disputed: true,
                amount: -from_disputed,
                excess: if excess_disputed { -from_excess } else { 0 },
            },
            Movement {
                disputed: false,
                amount: -from_failed,
                excess: if excess_disputed { 0 } else { -from_excess },
            },
        ];

        let (lock, _) =
            PaymentTransactionRow::lock_with_customer(conn, transaction.tenant_id, transaction.id)
                .await?;
        // The credit follows the disputed movement first; a failed collection's alone otherwise.
        let mut followed: Option<PaymentMovement> = None;
        for movement in movements.iter().filter(|m| m.amount != 0 || m.excess != 0) {
            let moved = record_movement_tx(
                conn,
                &lock,
                &transaction,
                movement,
                transaction.reversal_kind,
                reinstatement.provider_ref.clone(),
                reinstatement.reinstated_at,
            )
            .await?
            .ok_or_else(|| {
                Report::new(StoreError::Conflict(
                    "this reinstatement is being applied concurrently".to_owned(),
                ))
            })?;
            if movement.amount != 0 && followed.is_none() {
                followed = Some(moved);
            }
        }

        let reversed = transaction.amount_reversed - from_reversed;
        let status = if transaction.amount_refunded + reversed < transaction.amount {
            S::Settled
        } else {
            transaction.status.clone()
        };
        let updated: PaymentTransaction = PaymentTransactionRowPatch {
            id: transaction.id,
            status: (status != transaction.status).then(|| status.clone().into()),
            // Kept as the reversal cycle's high-water mark even when nothing stays reversed, so a
            // redelivered original reversal cannot claw the money back again.
            refunded_at: Some(Some(reinstatement.reinstated_at)),
            amount_reversed: Some(reversed),
            amount_disputed: Some(transaction.amount_disputed - from_disputed),
            amount_reversed_excess: Some(transaction.amount_reversed_excess - from_excess),
            ..Default::default()
        }
        .update(conn)
        .await?
        .into();

        log::warn!(
            "Payment transaction {} reinstated ({}): {} of {} {} minor units returned (provider tx {}, {} still reversed, {} excess)",
            updated.id,
            reinstatement.reason,
            handed_back,
            updated.amount,
            updated.currency,
            reinstatement.external_transaction_id,
            updated.amount_reversed,
            updated.amount_reversed_excess,
        );

        if let Some(moved) = followed {
            self.follow_payment_movement_tx(
                conn,
                &updated,
                &moved,
                None,
                Some(reinstatement.reinstated_at),
                Some(Collection {
                    payment: updated.id,
                    at: moved.at,
                }),
            )
            .await?;
        }

        // No `payment_transaction_saved`: its (Settled, nothing reversed) shape would re-run the
        // settled orchestration and re-apply a deferred plan change.
        self.internal
            .record_outbox_batch_tx(
                conn,
                updated.tenant_id,
                actor,
                vec![OutboxEvent::payment_reinstated(updated.clone().into())],
            )
            .await?;
        Ok(updated)
    }

    /// A refund that was in flight when a reversal arrived failed: the payment holds that money
    /// again, so the excess the reversal could not apply is applied now, up to it.
    pub(crate) async fn apply_reversal_excess_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        payment: PaymentTransactionId,
        tenant_id: common_domain::ids::TenantId,
    ) -> StoreResult<()> {
        let (lock, row) =
            PaymentTransactionRow::lock_with_customer(conn, tenant_id, payment).await?;
        let transaction: PaymentTransaction = row.into();
        if transaction.amount_reversed_excess <= 0
            || !matches!(transaction.status, S::Settled | S::Refunded)
        {
            return Ok(());
        }
        let holds = holds_tx(conn, &transaction).await?;
        let applied = (holds - transaction.amount_reversed)
            .min(transaction.amount_reversed_excess)
            .max(0);
        if applied == 0 {
            return Ok(());
        }
        let movement = Movement {
            disputed: transaction.amount_disputed > 0,
            amount: applied,
            excess: -applied,
        };
        let now = Utc::now().naive_utc();
        let Some(moved) = record_movement_tx(
            conn,
            &lock,
            &transaction,
            &movement,
            transaction.reversal_kind,
            None,
            now,
        )
        .await?
        else {
            return Ok(());
        };
        let reversed = transaction.amount_reversed + applied;
        let status = status_after(&transaction, reversed);
        let updated: PaymentTransaction = PaymentTransactionRowPatch {
            id: transaction.id,
            status: (status != transaction.status).then(|| status.clone().into()),
            amount_reversed: Some(reversed),
            amount_disputed: Some(
                transaction.amount_disputed + if movement.disputed { applied } else { 0 },
            ),
            amount_reversed_excess: Some(transaction.amount_reversed_excess - applied),
            ..Default::default()
        }
        .update(conn)
        .await?
        .into();
        log::error!(
            "Payment transaction {}: a failed refund freed {applied} {} minor units the provider had withdrawn; applied now",
            updated.id,
            updated.currency,
        );
        self.follow_payment_movement_tx(
            conn,
            &updated,
            &moved,
            (!movement.disputed).then(|| updated.reversal_kind.unwrap_or(ReversalKind::Other)),
            None,
            None,
        )
        .await?;
        self.internal
            .record_outbox_batch_tx(
                conn,
                updated.tenant_id,
                actor,
                vec![
                    OutboxEvent::payment_reversed(updated.clone().into()),
                    OutboxEvent::payment_transaction_saved(updated.into()),
                ],
            )
            .await
    }

    /// A provider reference applies once per direction. Without one, or for a payment carrying a
    /// reversal the previous release applied (it left no movement), the reversal cycle's
    /// high-water mark rejects an event at or before the last one applied.
    async fn already_applied_tx(
        &self,
        conn: &mut PgConn,
        transaction: &PaymentTransaction,
        provider_ref: Option<&str>,
        at: NaiveDateTime,
        reversal: bool,
    ) -> StoreResult<bool> {
        let marked = transaction.refunded_at.is_some_and(|mark| at <= mark);
        let Some(provider_ref) = provider_ref else {
            return Ok(marked);
        };
        if PaymentReversalMovementRow::applied(conn, transaction.id, provider_ref, reversal).await?
        {
            return Ok(true);
        }
        Ok(marked && has_legacy_reversal_tx(conn, transaction).await?)
    }

    /// What the payment's money funded follows it: credit kept from it, its invoice (reopened for
    /// a failed collection, re-closed when money comes back, never for a dispute), the credit the
    /// invoice sold and what it was overpaid.
    async fn follow_payment_movement_tx(
        &self,
        conn: &mut PgConn,
        payment: &PaymentTransaction,
        moved: &PaymentMovement,
        reopen: Option<ReversalKind>,
        paid_at: Option<NaiveDateTime>,
        collection: Option<Collection>,
    ) -> StoreResult<()> {
        let tenant_id = payment.tenant_id;
        crate::services::credit::collected_payment::follow_collected_payment_tx(
            conn,
            &*self.hooks.credit,
            tenant_id,
            payment.id,
            Some(moved),
        )
        .await?;
        let Some(invoice_id) = payment.invoice_id else {
            return Ok(());
        };
        let was_paid = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await?
            .invoice
            .payment_status
            == InvoicePaymentStatus::Paid;
        let (lock, _, _) =
            recompute_invoice_payment_state(conn, tenant_id, invoice_id, paid_at).await?;
        let collections = InvoiceRow::collections(conn, invoice_id, tenant_id).await?;
        InvoiceRow::sync_payment_disputed_at(
            conn,
            invoice_id,
            tenant_id,
            collections.disputed > 0,
            moved.at.naive_utc(),
        )
        .await?;
        if let Some(kind) = reopen {
            // Nothing automatic collects clawed-back money again: reminders take over,
            // unless the cause is unclear, then an operator decides.
            self.internal
                .on_invoice_reopened(
                    conn,
                    tenant_id,
                    invoice_id,
                    was_paid,
                    true,
                    ReopenedReminders::after_reversal(Some(kind)),
                    Utc::now().naive_utc(),
                )
                .await?;
        }
        crate::services::credit::reconcile_purchased_credit_tx(
            conn,
            &*self.hooks.credit,
            &lock,
            invoice_id,
            Some(moved),
        )
        .await?;
        crate::services::credit::collected_payment::reconcile_overpayment_tx(
            conn,
            &*self.hooks.credit,
            tenant_id,
            invoice_id,
            collection,
            moved.cause(),
            Some(moved),
        )
        .await
    }
}

/// Records one applied movement, dated at the provider's event (never in the future). `None` when
/// its provider reference was already applied in this direction.
async fn record_movement_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    transaction: &PaymentTransaction,
    movement: &Movement,
    kind: Option<ReversalKind>,
    provider_ref: Option<String>,
    at: NaiveDateTime,
) -> StoreResult<Option<PaymentMovement>> {
    let occurred_at = at.min(Utc::now().naive_utc());
    let row = PaymentReversalMovementRowNew {
        id: Uuid::now_v7(),
        tenant_id: transaction.tenant_id,
        payment_transaction_id: transaction.id,
        invoice_id: transaction.invoice_id,
        customer_id: lock.customer_id(),
        disputed: movement.disputed,
        amount: movement.amount,
        excess: movement.excess,
        reversal_kind: kind.map(Into::into),
        provider_ref,
        occurred_at,
    }
    .insert(conn, lock)
    .await?;
    Ok(row.map(|row| PaymentMovement {
        payment: transaction.id,
        movement: Some(row.id),
        at: row.occurred_at.and_utc(),
        disputed: row.disputed,
        follow_up: false,
        refunded: None,
    }))
}

/// A refund without a credit note gives back what collections kept above the invoice first:
/// records that part on it when it is issued (and returns it), so a later reversal of the payment
/// never counts it as collected again and a refund of other money never reads as one.
pub(crate) async fn record_refund_excess_tx(
    conn: &mut PgConn,
    refund: &PaymentTransaction,
) -> StoreResult<i64> {
    let Some(invoice_id) = refund
        .invoice_id
        .filter(|_| refund.credit_note_id.is_none())
    else {
        return Ok(0);
    };
    let lock = InvoiceRow::select_for_update_by_id(conn, refund.tenant_id, invoice_id)
        .await?
        .lock;
    let excess = InvoiceRow::collections(conn, invoice_id, refund.tenant_id)
        .await?
        .excess()
        .min(refund.amount)
        .max(0);
    if excess > 0 {
        PaymentTransactionRow::set_refund_excess(conn, &lock, refund.id, excess).await?;
    }
    Ok(excess)
}

/// The payment carries a reversal the previous release applied: it left no movement behind.
async fn has_legacy_reversal_tx(
    conn: &mut PgConn,
    transaction: &PaymentTransaction,
) -> StoreResult<bool> {
    let (movements, applied) =
        PaymentReversalMovementRow::applied_total(conn, transaction.id).await?;
    Ok(transaction.amount_reversed != applied
        || (movements == 0 && transaction.refunded_at.is_some()))
}

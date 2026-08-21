use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{PaymentIntent, PaymentTransactionWithMethod};
use crate::errors::StoreError;
use common_domain::ids::{InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::enums::InvoicePaymentStatus;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use error_stack::Report;

/// A settled payment being clawed back by the provider (refund, chargeback, or
/// lost dispute). Drives [`PaymentTransactionInterface::reverse_transaction_tx`].
#[derive(Debug, Clone)]
pub struct TransactionReversal {
    /// Provider charge id — surfaced in the finance-alert log / audit trail.
    pub external_transaction_id: String,
    pub amount: ReversalAmount,
    pub reason: String,
    pub reversed_at: chrono::NaiveDateTime,
}

/// How much of the transaction a reversal event claws back.
#[derive(Debug, Clone, Copy)]
pub enum ReversalAmount {
    /// Running total from the provider (Stripe `charge.amount_refunded`);
    /// monotonic — a stale/redelivered figure can never lower what we recorded.
    Cumulative(i64),
    /// A delta on top of whatever is already refunded (Stripe dispute `amount`,
    /// which excludes prior refunds); guarded by `reversed_at` against
    /// redelivery double-counting.
    Incremental(i64),
    /// The whole transaction (GoCardless chargeback / late failure).
    Full,
}

/// Previously clawed-back funds returned to the merchant (Stripe dispute funds
/// reinstated, GoCardless `chargeback_cancelled`). Inverse of
/// [`TransactionReversal`]; drives
/// [`PaymentTransactionInterface::reinstate_transaction_tx`].
#[derive(Debug, Clone)]
pub struct TransactionReinstatement {
    /// Provider charge id — surfaced in the finance log / audit trail.
    pub external_transaction_id: String,
    /// Amount handed back, in minor units. `None` means everything that was
    /// clawed back (GoCardless chargebacks are full-amount).
    pub reinstated_amount_minor: Option<i64>,
    pub reason: String,
    pub reinstated_at: chrono::NaiveDateTime,
}

#[async_trait::async_trait]
pub trait PaymentTransactionInterface {
    async fn list_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<PaymentTransactionWithMethod>>;

    async fn last_settled_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Option<PaymentTransaction>>;

    /// True when a payment attempt owns this invoice (in flight or settled).
    /// Callers must not charge, merge or mutate such an invoice.
    async fn invoice_has_live_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<bool>;

    async fn consolidate_intent_and_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    /// Apply a post-settlement reversal to a transaction and reopen its invoice.
    /// Idempotent under webhook redelivery. Must be called within a transaction
    /// with the transaction row already locked (SELECT FOR UPDATE).
    async fn reverse_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        reversal: TransactionReversal,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    /// Inverse of [`Self::reverse_transaction_tx`]: a dispute/chargeback was
    /// resolved in the merchant's favor and the clawed-back funds returned.
    /// Reduces the recorded refunded total, restores `Settled`, and re-closes
    /// the invoice. Idempotent under webhook redelivery. Must be called within
    /// a transaction with the transaction row already locked (SELECT FOR UPDATE).
    async fn reinstate_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        reinstatement: TransactionReinstatement,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction>;

    /// Correlate a provider webhook to our transaction by the provider's own id
    /// (e.g. GoCardless `PM...`), for providers whose webhook events don't echo
    /// our `meteroid.transaction_id`.
    async fn get_payment_tx_by_provider_transaction_id(
        &self,
        tenant_id: TenantId,
        provider_transaction_id: &str,
    ) -> StoreResult<Option<PaymentTransaction>>;
}

#[async_trait::async_trait]
impl PaymentTransactionInterface for Store {
    async fn list_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<PaymentTransactionWithMethod>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|rows| {
                rows.into_iter()
                    .map(std::convert::Into::into)
                    .collect::<Vec<PaymentTransactionWithMethod>>()
            })
    }

    async fn invoice_has_live_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::exists_live_for_invoice(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn last_settled_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Option<PaymentTransaction>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::last_settled_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|row_opt| row_opt.map(std::convert::Into::into))
    }

    async fn consolidate_intent_and_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        use domain::enums::PaymentStatusEnum;

        // Explicit transition table. Settled leaves ONLY via
        // `reverse_transaction_tx` (amount-carrying refund/chargeback/late-failure
        // events): a bare failure event on a Settled transaction is always stale
        // or out-of-order and must never claw money back. The one out-of-order
        // transition we do honor is Failed → Settled — the provider captured the
        // money, so a success arriving after a stale failure wins.
        let transition_allowed = match transaction.status {
            PaymentStatusEnum::Pending | PaymentStatusEnum::Ready => true,
            PaymentStatusEnum::Failed => payment_intent.status == PaymentStatusEnum::Settled,
            PaymentStatusEnum::Settled
            | PaymentStatusEnum::Cancelled
            | PaymentStatusEnum::Refunded => false,
        };

        if !transition_allowed {
            if transaction.status == payment_intent.status {
                log::info!(
                    "Transaction {} already in state {:?}; duplicate event is a no-op",
                    transaction.id,
                    transaction.status
                );
            } else {
                tracing::warn!(
                    transaction_id = %transaction.id,
                    current_status = ?transaction.status,
                    event_status = ?payment_intent.status,
                    "rejected out-of-order payment status transition; keeping current state"
                );
            }
            return Ok(transaction);
        }

        let status_changed = transaction.status != payment_intent.status;

        // An async charge (GoCardless DD, Stripe ACH) returns Pending, so the id
        // arrives with no status change; still persist it once, or reconciliation
        // (filters provider_transaction_id IS NOT NULL) can never recover it.
        let backfill_external_id =
            transaction.provider_transaction_id.is_none() && !payment_intent.external_id.is_empty();

        // Set the 3DS/SCA action when the charge needs one (status stays
        // Pending); clear it once the charge reaches a terminal state.
        let next_action_patch: Option<Option<serde_json::Value>> = match &payment_intent.next_action
        {
            // The client secret is #[serde(skip)] on PaymentNextAction, so it is
            // never written here regardless.
            Some(action) => Some(serde_json::to_value(action).ok()),
            None if status_changed
                && payment_intent.status != domain::enums::PaymentStatusEnum::Pending =>
            {
                Some(None)
            }
            None => None,
        };

        if !status_changed && !backfill_external_id && next_action_patch.is_none() {
            return Ok(transaction);
        }

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            invoice_id: None,
            status: status_changed.then(|| payment_intent.status.clone().into()),
            processed_at: status_changed.then_some(payment_intent.processed_at),
            refunded_at: None,
            error_type: status_changed.then_some(payment_intent.last_payment_error),
            provider_transaction_id: backfill_external_id
                .then(|| Some(payment_intent.external_id.clone())),
            payment_method_id: None,
            next_action: next_action_patch,
            amount_refunded: None,
        };

        let tenant_id = transaction.tenant_id;
        let updated_transaction = self
            .transaction_with(conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    let transaction: PaymentTransaction = updated_transaction.into();

                    // A pure id backfill is not a settlement event; don't broadcast it.
                    if status_changed {
                        self.internal
                            .record_outbox_batch_tx(
                                conn,
                                tenant_id,
                                actor,
                                vec![OutboxEvent::payment_transaction_saved(
                                    transaction.clone().into(),
                                )],
                            )
                            .await?;
                    }

                    // Payment method is resolved dynamically from the customer at billing time
                    // No need to update the subscription's payment method field

                    Ok(transaction)
                }
                .scope_boxed()
            })
            .await?;

        Ok(updated_transaction)
    }

    async fn reverse_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        mut reversal: TransactionReversal,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        use chrono::SubsecRound;
        use domain::enums::PaymentStatusEnum as S;

        // Postgres timestamps carry microsecond precision, so a stored
        // `refunded_at` stamp is truncated relative to an event time parsed
        // with nanoseconds. Truncate up front so the redelivery guard below
        // compares like with like (an identical redelivered event must no-op).
        reversal.reversed_at = reversal.reversed_at.trunc_subsecs(6);

        match transaction.status {
            // Reversal before we ever recorded settlement (e.g. a GoCardless late
            // failure on a not-yet-confirmed charge): this is just a failed
            // charge — reuse the existing failed path, don't reopen anything.
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
                    last_payment_error: Some(reversal.reason.clone()),
                    processed_at: Some(reversal.reversed_at),
                };
                return self
                    .consolidate_intent_and_transaction_tx(conn, actor, transaction, intent)
                    .await;
            }
            S::Settled => {}
            // Already reversed / failed / cancelled: nothing left to claw back.
            _ => return Ok(transaction),
        }

        let new_amount_refunded = match reversal.amount {
            // Monotonic refunded total: a redelivered or stale cumulative amount
            // can never lower what we already recorded.
            //
            // `amount_refunded` is a single column written by BOTH this cumulative
            // refund total (Stripe `charge.amount_refunded`) and dispute
            // `funds_withdrawn` deltas (the `Incremental` arm). Stripe's
            // `charge.amount_refunded` excludes dispute amounts, so in principle a
            // refund arriving AFTER a dispute (which already raised
            // `amount_refunded`) could be swallowed by this `.max()`. That order
            // is not reachable on Stripe: a charge with an open or lost dispute
            // cannot be refunded — the refund API rejects it because the funds
            // already left via the dispute. The reachable order,
            // refund-then-dispute, is handled correctly (the dispute delta stacks
            // on top via the `Incremental` arm below), so the two never contend
            // for the column and no separate dispute/refund totals are needed.
            ReversalAmount::Cumulative(total) => total
                .clamp(0, transaction.amount)
                .max(transaction.amount_refunded),
            // A redelivered ORIGINAL chargeback after a full reversal→reinstatement
            // finds the transaction Settled again (`amount_refunded` back to 0)
            // but with `refunded_at` still holding the reinstatement time — kept
            // as a reversal-cycle high-water mark precisely so this guard survives
            // reinstatement (see `reinstate_transaction_tx`). Reject any full
            // reversal at or before that mark; a genuinely new chargeback always
            // carries a later provider timestamp. (A first-ever chargeback has
            // `refunded_at == None` and passes; a redelivery while still Refunded
            // is already caught by the terminal-status early return above.)
            ReversalAmount::Full => {
                if let Some(refunded_at) = transaction.refunded_at
                    && reversal.reversed_at <= refunded_at
                {
                    log::info!(
                        "Transaction {} already recorded a reversal/reinstatement at {}; skipping duplicate/stale full reversal ({})",
                        transaction.id,
                        refunded_at,
                        reversal.reason
                    );
                    return Ok(transaction);
                }
                transaction.amount
            }
            // A delta is not idempotent by itself: skip events at or before the
            // last recorded reversal so a redelivery can't double-count. (A real
            // later reversal always carries a later provider timestamp.)
            ReversalAmount::Incremental(delta) => {
                if let Some(refunded_at) = transaction.refunded_at
                    && reversal.reversed_at <= refunded_at
                {
                    log::info!(
                        "Transaction {} already recorded a reversal at {}; skipping duplicate/stale incremental reversal ({})",
                        transaction.id,
                        refunded_at,
                        reversal.reason
                    );
                    return Ok(transaction);
                }
                (transaction.amount_refunded + delta.max(0)).min(transaction.amount)
            }
        };
        // Full claw-back flips to Refunded.
        let new_status = if new_amount_refunded >= transaction.amount {
            S::Refunded
        } else {
            S::Settled
        };

        // Idempotent under redelivery.
        if new_amount_refunded == transaction.amount_refunded && new_status == transaction.status {
            return Ok(transaction);
        }

        let tenant_id = transaction.tenant_id;
        let invoice_id = transaction.invoice_id;
        let status_changed = new_status != transaction.status;

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            status: status_changed.then(|| new_status.clone().into()),
            refunded_at: Some(Some(reversal.reversed_at)),
            amount_refunded: Some(new_amount_refunded),
            ..Default::default()
        };

        let updated: PaymentTransaction = patch.update(conn).await?.into();

        // Finance-alert worthy: money that had settled was clawed back.
        log::error!(
            "Payment transaction {} reversed ({}): {} of {} {} minor units clawed back (provider tx {}); reopening invoice {:?}",
            updated.id,
            reversal.reason,
            new_amount_refunded,
            updated.amount,
            updated.currency,
            reversal.external_transaction_id,
            invoice_id,
        );

        if let Some(invoice_id) = invoice_id {
            // Lock the invoice, re-derive amount_due from the now-reduced net
            // settled sum, then re-derive its payment status.
            InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id).await?;
            let recomputed =
                InvoiceRow::recompute_amount_due_from_settled_payments(conn, invoice_id, tenant_id)
                    .await?;

            let new_payment_status = if recomputed.amount_due <= 0 {
                InvoicePaymentStatus::Paid
            } else if recomputed.amount_due >= recomputed.total {
                InvoicePaymentStatus::Unpaid
            } else {
                InvoicePaymentStatus::PartiallyPaid
            };
            InvoiceRow::apply_payment_status(conn, invoice_id, tenant_id, new_payment_status, None)
                .await?;
        }

        self.internal
            .record_outbox_batch_tx(
                conn,
                tenant_id,
                actor,
                vec![OutboxEvent::payment_transaction_saved(
                    updated.clone().into(),
                )],
            )
            .await?;

        Ok(updated)
    }

    async fn reinstate_transaction_tx(
        &self,
        conn: &mut PgConn,
        _actor: &Actor,
        transaction: PaymentTransaction,
        mut reinstatement: TransactionReinstatement,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        use chrono::SubsecRound;
        use domain::enums::PaymentStatusEnum as S;

        // See reverse_transaction_tx: align event-time precision with the
        // stored (microsecond) `refunded_at` stamp before the guards compare.
        reinstatement.reinstated_at = reinstatement.reinstated_at.trunc_subsecs(6);

        // Only a reversed transaction has something to hand back. A redelivery
        // after a full reinstatement also lands here (amount_refunded == 0).
        if !matches!(transaction.status, S::Settled | S::Refunded)
            || transaction.amount_refunded <= 0
        {
            log::info!(
                "Transaction {} has no recorded reversal (status {:?}, refunded {}); reinstatement ({}) is a no-op",
                transaction.id,
                transaction.status,
                transaction.amount_refunded,
                reinstatement.reason,
            );
            return Ok(transaction);
        }

        // Applying a reinstatement stamps its event time on `refunded_at`, so a
        // redelivered (same timestamp) or out-of-order (older) event no-ops
        // instead of double-reducing the refunded total.
        if let Some(refunded_at) = transaction.refunded_at
            && reinstatement.reinstated_at <= refunded_at
        {
            log::info!(
                "Transaction {} reversal state ({}) is at or after the reinstatement event ({}); skipping duplicate/stale reinstatement",
                transaction.id,
                refunded_at,
                reinstatement.reinstated_at,
            );
            return Ok(transaction);
        }

        let handed_back = reinstatement
            .reinstated_amount_minor
            .unwrap_or(transaction.amount)
            .clamp(0, transaction.amount_refunded);
        if handed_back == 0 {
            return Ok(transaction);
        }
        let new_amount_refunded = transaction.amount_refunded - handed_back;
        // Money is back below the full amount: the transaction is settled again.
        let new_status = if new_amount_refunded < transaction.amount {
            S::Settled
        } else {
            transaction.status.clone()
        };
        let status_changed = new_status != transaction.status;

        let tenant_id = transaction.tenant_id;
        let invoice_id = transaction.invoice_id;

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            status: status_changed.then(|| new_status.clone().into()),
            // Stamp this event's time as the reversal-cycle high-water mark, even
            // on a FULL reinstatement (`amount_refunded` back to 0). Clearing it
            // would make the transaction indistinguishable from a never-reversed
            // settled payment, so a redelivered ORIGINAL reversal (its own,
            // earlier timestamp) could re-claw the funds and reopen a paid
            // invoice. Keeping it lets `reverse_transaction_tx` reject any
            // reversal event at or before the last reversal/reinstatement.
            refunded_at: Some(Some(reinstatement.reinstated_at)),
            amount_refunded: Some(new_amount_refunded),
            ..Default::default()
        };

        let updated: PaymentTransaction = patch.update(conn).await?.into();

        // Finance-relevant: previously clawed-back money was returned.
        log::warn!(
            "Payment transaction {} reinstated ({}): {} of {} {} minor units returned (provider tx {}, {} still refunded); re-closing invoice {:?}",
            updated.id,
            reinstatement.reason,
            handed_back,
            updated.amount,
            updated.currency,
            reinstatement.external_transaction_id,
            new_amount_refunded,
            invoice_id,
        );

        if let Some(invoice_id) = invoice_id {
            // Lock the invoice, re-derive amount_due from the restored net
            // settled sum, then re-derive its payment status (re-closes it).
            InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id).await?;
            let recomputed =
                InvoiceRow::recompute_amount_due_from_settled_payments(conn, invoice_id, tenant_id)
                    .await?;

            let new_payment_status = if recomputed.amount_due <= 0 {
                InvoicePaymentStatus::Paid
            } else if recomputed.amount_due >= recomputed.total {
                InvoicePaymentStatus::Unpaid
            } else {
                InvoicePaymentStatus::PartiallyPaid
            };
            InvoiceRow::apply_payment_status(
                conn,
                invoice_id,
                tenant_id,
                new_payment_status,
                Some(reinstatement.reinstated_at),
            )
            .await?;
        }

        // Deliberately NO `payment_transaction_saved` outbox event here.
        // Reinstatement already re-derived `amount_due` and re-closed the invoice
        // synchronously above, so there is nothing left for invoice-orchestration
        // to do. Emitting the event would in fact be harmful on a FULL
        // reinstatement: it carries the (status Settled, `amount_refunded == 0`)
        // shape that the settled handler's first orchestration arm matches, which
        // re-runs `on_payment_transaction_settled` — re-applying a deferred plan
        // change (that block is gated only on `pending_plan_version_id`, a field
        // never cleared from the row). No other consumer reads this event:
        // webhook-out and entity-activity both map `PaymentTransactionSaved` to
        // `None`, and the analytics stream is a table-polling projection that
        // does not depend on the outbox. Suppressing it is therefore safe and
        // keeps the reinstatement idempotent under redelivery.
        Ok(updated)
    }

    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction> {
        PaymentTransactionRow::get_by_id_for_update(conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(std::convert::Into::into)
    }

    async fn get_payment_tx_by_provider_transaction_id(
        &self,
        tenant_id: TenantId,
        provider_transaction_id: &str,
    ) -> StoreResult<Option<PaymentTransaction>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::get_by_provider_transaction_id(
            &mut conn,
            provider_transaction_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|row_opt| row_opt.map(std::convert::Into::into))
    }
}

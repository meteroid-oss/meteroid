use crate::hooks::CreditControlHooks;
use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::Actor;
use crate::domain::enums::ReversalKind;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::{ObservedRefundStatus, PaymentTransaction};
use crate::domain::{PaymentIntent, PaymentTransactionWithMethod};
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::PaymentMovement;
use common_domain::ids::{BaseId, CreditNoteId, InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::enums::InvoicePaymentStatus;
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use error_stack::Report;

/// A settled payment being INVOLUNTARILY clawed back: lost to a dispute or chargeback (the invoice
/// stays paid, the loss is booked) or a failed collection (the invoice reopens). Drives
/// [`PaymentTransactionInterface::reverse_transaction_tx`]. A merchant-issued refund is not a
/// reversal — it is a REFUND child row, see [`PaymentTransactionInterface::settle_refund_tx`].
#[derive(Debug, Clone)]
pub struct TransactionReversal {
    /// Provider charge id — surfaced in the finance-alert log / audit trail.
    pub external_transaction_id: String,
    pub amount: ReversalAmount,
    pub kind: ReversalKind,
    /// Raw provider cause or free text; `None` when the provider gave none.
    pub reason: Option<String>,
    pub reversed_at: chrono::NaiveDateTime,
    /// Lost to a dispute or chargeback; otherwise a failed collection.
    pub disputed: bool,
    /// The provider's dispute, chargeback or event id: applied once.
    pub provider_ref: Option<String>,
}

/// How much of the transaction the provider withdrew with this event. Never applied above what
/// the payment still holds (its amount less refunds, settled or in flight).
#[derive(Debug, Clone, Copy)]
pub enum ReversalAmount {
    /// A delta on top of what was already withdrawn (Stripe dispute `amount`, a Mollie chargeback).
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
    /// The provider's dispute, chargeback or event id: applied once.
    pub provider_ref: Option<String>,
}

/// What a provider says about the refunds on one payment, as observed through a webhook or a
/// `fetch_refund` poll. Drives
/// [`PaymentTransactionInterface::record_observed_refunds_tx`].
#[derive(Debug, Clone)]
pub struct ObservedRefunds {
    /// Provider charge id of the parent payment.
    pub external_transaction_id: String,
    /// The parent's cumulative refunded total at the provider. Monotonic: a redelivered or
    /// stale figure can never lower what we recorded.
    pub cumulative_refunded_minor: i64,
    /// The individual refunds the event named, if any; used to settle rows we already issued
    /// before recording the remaining delta as an observed refund.
    pub refund_ids: Vec<ObservedRefund>,
    pub observed_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct ObservedRefund {
    pub external_refund_id: String,
    pub amount_minor: Option<i64>,
    pub status: ObservedRefundStatus,
}

/// Terminal (or intermediate) outcome of a refund we issued, applied to its REFUND row by
/// [`PaymentTransactionInterface::settle_refund_tx`].
#[derive(Debug, Clone)]
pub struct RefundSettlement {
    pub status: RefundSettlementStatus,
}

#[derive(Debug, Clone)]
pub enum RefundSettlementStatus {
    Settled {
        processed_at: chrono::NaiveDateTime,
        external_refund_id: String,
    },
    Failed {
        error: String,
        /// The provider may have named the refund before refusing it.
        external_refund_id: Option<String>,
    },
    Pending {
        external_refund_id: String,
    },
}

/// What a finalized Refund credit note has drawn and still can. `refundable_cents` is the
/// credit note's `refunded_amount_cents` minus everything already drawn or in flight.
#[derive(Debug, Clone)]
pub struct CreditNoteRefundSummary {
    pub refundable_cents: i64,
    pub refunded_cents: i64,
    pub pending_cents: i64,
    pub refunds: Vec<PaymentTransaction>,
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

    /// The provider processes the payment without the customer now: no action is awaited.
    async fn clear_awaited_payment_action(
        &self,
        tenant_id: TenantId,
        transaction_id: PaymentTransactionId,
    ) -> StoreResult<()>;

    /// True when a payment attempt owns this invoice (in flight or settled).
    /// Callers must not charge, merge or mutate such an invoice.
    async fn invoice_has_live_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<bool>;

    async fn invoice_has_any_payment(
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

    /// Apply a post-settlement reversal: a dispute keeps the invoice paid and books the loss, a
    /// failed collection reopens it. Idempotent under webhook redelivery. Must be called within a
    /// transaction with the transaction row already locked (SELECT FOR UPDATE).
    async fn reverse_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        reversal: TransactionReversal,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    /// Inverse of [`Self::reverse_transaction_tx`]: a dispute/chargeback was
    /// resolved in the merchant's favor and the clawed-back funds returned.
    /// Idempotent under webhook redelivery. Must be called within a transaction
    /// with the transaction row already locked (SELECT FOR UPDATE).
    async fn reinstate_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        reinstatement: TransactionReinstatement,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    /// Record what the provider says about a payment's refunds — a VOLUNTARY movement, so the
    /// invoice is never touched. Settles any REFUND row named in `observed.refund_ids`, then
    /// inserts one Settled row for the delta between `cumulative_refunded_minor` and what is
    /// already known, and bumps the parent's `amount_refunded` monotonically (clamped to
    /// `amount`), flipping it to `Refunded` once fully given back. A delta of zero records
    /// nothing, which makes redelivery a no-op. Must be called within a transaction with the
    /// parent row already locked (SELECT FOR UPDATE).
    async fn record_observed_refunds_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        parent: PaymentTransaction,
        observed: ObservedRefunds,
    ) -> StoreResult<PaymentTransaction>;

    /// Apply the outcome of a refund we issued to its REFUND row. `Settled` also bumps the
    /// parent's `amount_refunded` by the refund amount (and flips it to `Refunded` when fully
    /// refunded) and emits `RefundSettled`; `Failed` emits `RefundFailed`; `Pending` only
    /// records the provider refund id. Never touches the invoice. Must be called within a
    /// transaction; it locks the parent row itself.
    async fn settle_refund_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        refund: PaymentTransaction,
        outcome: RefundSettlement,
    ) -> StoreResult<PaymentTransaction>;

    /// What is still refundable on a settled payment:
    /// `amount - amount_refunded - amount_reversed - Σ(READY|PENDING refund rows)`.
    async fn refundable_amount_tx(
        &self,
        conn: &mut PgConn,
        parent: &PaymentTransaction,
    ) -> StoreResult<i64>;

    /// Locks the payment behind its customer (`diesel_models::lock`).
    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction>;

    async fn get_payment_tx_by_id(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<PaymentTransaction>;

    /// The persisted next action; the domain type ghosts it, so read the row.
    async fn get_payment_tx_next_action(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<Option<serde_json::Value>>;

    /// The raw row, for the columns the domain type ghosts (`initiated_by_customer_id`, `next_action`).
    async fn get_payment_tx_row(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<PaymentTransactionRow>;

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

    async fn invoice_has_any_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::exists_for_invoice(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn clear_awaited_payment_action(
        &self,
        tenant_id: TenantId,
        transaction_id: PaymentTransactionId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::clear_awaited_action(&mut conn, transaction_id, tenant_id)
            .await
            .map(|_| ())
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
            status: status_changed.then(|| payment_intent.status.clone().into()),
            processed_at: status_changed.then_some(payment_intent.processed_at),
            error_type: status_changed.then_some(payment_intent.last_payment_error),
            decline_kind: status_changed.then(|| payment_intent.decline_kind.map(Into::into)),
            provider_transaction_id: backfill_external_id
                .then(|| Some(payment_intent.external_id.clone())),
            next_action: next_action_patch,
            ..Default::default()
        };

        let tenant_id = transaction.tenant_id;
        // A challenge is being set on a charge that was merely pending: it enters the
        // awaiting-authentication state now, whichever path delivered the intent.
        let enters_authentication = transaction.status == PaymentStatusEnum::Pending
            && matches!(patch.next_action, Some(Some(_)));
        let updated_transaction = self
            .transaction_with(conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    if enters_authentication
                        && updated_transaction.initiated_by_customer_id.is_none()
                        && domain::payment_transactions::row_awaits_authentication(
                            &updated_transaction,
                        )
                        && let Some(invoice_id) = updated_transaction.invoice_id
                    {
                        use crate::domain::customer_emails::{EmailContext, EmailKind};
                        use crate::repositories::customer_emails::CustomerEmailsInterface;
                        self.enqueue_customer_email_tx(
                            conn,
                            tenant_id,
                            EmailKind::AuthenticationRequired,
                            EmailContext::AuthenticationRequired {
                                invoice_id,
                                payment_transaction_id: updated_transaction.id,
                            },
                            format!("authentication_required:{}", updated_transaction.id),
                        )
                        .await?;
                    }

                    let transaction: PaymentTransaction = updated_transaction.into();

                    // A pure id backfill is not a settlement event; don't broadcast it.
                    if status_changed {
                        let mut event: crate::domain::outbox_event::PaymentTransactionEvent =
                            transaction.clone().into();
                        // A failed top-up discards its purchase document and detaches the payment.
                        if matches!(
                            event.status,
                            PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled
                        ) && matches!(
                            event.payment_type,
                            domain::PaymentTypeEnum::TopUp | domain::PaymentTypeEnum::AutoTopup
                        ) {
                            event.invoice_id = None;
                        }
                        self.internal
                            .record_outbox_batch_tx(
                                conn,
                                tenant_id,
                                actor,
                                vec![OutboxEvent::payment_transaction_saved(event)],
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
        reversal: TransactionReversal,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        self.reverse_payment_tx(conn, actor, transaction, reversal)
            .await
    }

    async fn reinstate_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        reinstatement: TransactionReinstatement,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        self.reinstate_payment_tx(conn, actor, transaction, reinstatement)
            .await
    }

    async fn record_observed_refunds_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        parent: PaymentTransaction,
        observed: ObservedRefunds,
    ) -> StoreResult<PaymentTransaction> {
        use diesel_models::enums::PaymentStatusEnum as DbStatus;

        let tenant_id = parent.tenant_id;
        // The caller holds the parent, so its customer: this re-lock only yields the proof.
        let (lock, _) =
            PaymentTransactionRow::lock_with_customer(conn, tenant_id, parent.id).await?;
        let mut events = Vec::new();
        // Refunds the provider named that we hold no row for; each becomes its own row so a
        // later status change can still find it by id.
        let mut unmatched: Vec<&ObservedRefund> = Vec::new();
        let mut settled_against_note = false;
        // What refunds without a note this event issued gave back of the purchase (beyond the
        // excess), and what failed: the credit the payment bought follows both.
        let (mut refunded_now, mut failed_now) = (0_i64, 0_i64);

        for seen in &observed.refund_ids {
            let refunds: Vec<PaymentTransaction> =
                PaymentTransactionRow::find_refund_by_provider_id(
                    conn,
                    &lock,
                    &seen.external_refund_id,
                )
                .await?
                .into_iter()
                .map(Into::<PaymentTransaction>::into)
                .filter(|r| r.parent_transaction_id == Some(parent.id))
                .collect();

            if refunds.is_empty() {
                // A failed refund moved no money and is not in the provider's cumulative total.
                if seen.status != ObservedRefundStatus::Failed {
                    unmatched.push(seen);
                }
                continue;
            }

            let status = match seen.status {
                ObservedRefundStatus::Succeeded => RefundSettlementStatus::Settled {
                    processed_at: observed.observed_at,
                    external_refund_id: seen.external_refund_id.clone(),
                },
                ObservedRefundStatus::Pending => RefundSettlementStatus::Pending {
                    external_refund_id: seen.external_refund_id.clone(),
                },
                ObservedRefundStatus::Failed => RefundSettlementStatus::Failed {
                    error: "Refused by the payment provider".to_string(),
                    external_refund_id: Some(seen.external_refund_id.clone()),
                },
            };

            for refund in &refunds {
                let Some(updated) = patch_refund_row(conn, refund, &status).await? else {
                    continue;
                };
                settled_against_note |= updated.credit_note_id.is_some()
                    && seen.status == ObservedRefundStatus::Succeeded;
                if updated.credit_note_id.is_none() && seen.status == ObservedRefundStatus::Failed {
                    failed_now += updated.amount;
                }
                events.push(OutboxEvent::payment_transaction_saved(
                    updated.clone().into(),
                ));
                match seen.status {
                    ObservedRefundStatus::Succeeded => {
                        events.push(OutboxEvent::refund_settled(updated.into()))
                    }
                    ObservedRefundStatus::Failed => {
                        events.push(OutboxEvent::refund_failed(updated.into()))
                    }
                    ObservedRefundStatus::Pending => {}
                }
            }
        }

        // The provider's cumulative figure counts refunds still in flight, ours included: a
        // `Ready` row we issued a moment ago must not be recorded a second time as observed. It
        // covers provider-side refunds only, so an offline refund must not be counted against it.
        let mut known = PaymentTransactionRow::sum_online_refunds_by_parent(
            conn,
            parent.id,
            tenant_id,
            &[DbStatus::Settled, DbStatus::Ready, DbStatus::Pending],
        )
        .await?;
        // A figure the backfill moved out of `amount_refunded` has no refund rows behind it, but
        // the provider still counts it: without this, the first new refund of a charge that was
        // already refunded before the migration records that old money a second time.
        if parent.reversal_kind == Some(domain::enums::ReversalKind::Migrated) {
            known += parent.amount_reversed;
        }

        // Bounded by what the payment still holds: a provider figure that overshoots (a charge
        // refunded before this data model existed, a provider counting differently) must not
        // record money that was never ours to give back.
        let in_flight_all = PaymentTransactionRow::sum_refunds_by_parent(
            conn,
            parent.id,
            tenant_id,
            &[DbStatus::Ready, DbStatus::Pending],
        )
        .await?;
        let headroom =
            parent.amount - parent.amount_refunded - parent.amount_reversed - in_flight_all;
        let mut delta = (observed.cumulative_refunded_minor - known).min(headroom);
        for seen in &unmatched {
            // Clamped rather than skipped: dropping it would re-record the same money in the
            // anonymous row below and lose the provider's refund id with it, leaving later
            // status changes for that refund unmatchable.
            let Some(amount) = seen.amount_minor.map(|a| a.min(delta)).filter(|a| *a > 0) else {
                continue;
            };
            let status = match seen.status {
                ObservedRefundStatus::Pending => DbStatus::Pending,
                _ => DbStatus::Settled,
            };
            let inserted = insert_observed_refund(
                conn,
                &parent,
                amount,
                status.clone(),
                Some(seen.external_refund_id.clone()),
                observed.observed_at,
            )
            .await?;
            delta -= amount;
            for row in inserted {
                refunded_now += row.purchase_minor;
                settled_against_note |=
                    row.refund.credit_note_id.is_some() && status == DbStatus::Settled;
                events.extend(row.events(&status));
            }
        }

        // What the event did not attribute to a named refund. It only carries a provider id when
        // exactly one refund is unaccounted for, so a later status change can still match it.
        if delta > 0 {
            let residual: Vec<&&ObservedRefund> = unmatched
                .iter()
                .filter(|r| r.amount_minor.is_none())
                .collect();
            let only_pending = !residual.is_empty()
                && residual
                    .iter()
                    .all(|r| r.status == ObservedRefundStatus::Pending);
            let status = if only_pending {
                DbStatus::Pending
            } else {
                DbStatus::Settled
            };
            let named = (residual.len() == 1).then(|| residual[0].external_refund_id.clone());
            let inserted = insert_observed_refund(
                conn,
                &parent,
                delta,
                status.clone(),
                named,
                observed.observed_at,
            )
            .await?;

            log::info!(
                "Recorded a provider-side {:?} refund of {} {} on transaction {} (provider tx {}, cumulative {})",
                status,
                delta,
                parent.currency,
                parent.id,
                observed.external_transaction_id,
                observed.cumulative_refunded_minor,
            );

            for row in inserted {
                refunded_now += row.purchase_minor;
                settled_against_note |=
                    row.refund.credit_note_id.is_some() && status == DbStatus::Settled;
                events.extend(row.events(&status));
            }
        }

        // Voluntary money movement: the invoice keeps its balance, the Refund credit note is
        // what reduces it. Only the parent's denormalized total moves.
        let updated = sync_parent_refunded(conn, &parent).await?;

        // A refund of ours the provider just settled may be drawn on an avoir, which already
        // shrank the invoice; re-derive the balance exactly as the `refund.updated` path does.
        // One the provider made without a note takes back the credit the payment bought.
        let refunded = (refunded_now > 0).then(|| {
            PaymentMovement::refund(parent.id, refunded_now, observed.observed_at.and_utc())
        });
        if let Some(invoice_id) = parent.invoice_id
            && (settled_against_note || refunded.is_some())
        {
            let lock = if settled_against_note {
                recompute_invoice_payment_state(conn, tenant_id, invoice_id, None)
                    .await?
                    .0
            } else {
                InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                    .await?
                    .lock
            };
            crate::services::credit::reconcile_purchased_credit_tx(
                conn,
                &*self.hooks.credit,
                &lock,
                invoice_id,
                refunded.as_ref(),
            )
            .await?;
        }
        // A refund with no note gives back what the invoice kept above its total first.
        if let Some(invoice_id) = parent.invoice_id {
            crate::services::credit::collected_payment::reconcile_overpayment_tx(
                conn,
                &*self.hooks.credit,
                tenant_id,
                invoice_id,
                None,
                crate::domain::wallet_events::CreditUnrecoveredCause::PaymentRefunded,
                None,
            )
            .await?;
        }
        // Money a payment collected kept whole as credit: refunded, it takes that credit back.
        if refunded_now > 0 || failed_now > 0 {
            let moved = PaymentMovement::refund(
                parent.id,
                if refunded_now > 0 {
                    refunded_now
                } else {
                    failed_now
                },
                observed.observed_at.and_utc(),
            );
            crate::services::credit::collected_payment::follow_collected_payment_tx(
                conn,
                &*self.hooks.credit,
                tenant_id,
                parent.id,
                Some(&moved),
            )
            .await?;
        }

        if !events.is_empty() {
            self.internal
                .record_outbox_batch_tx(conn, tenant_id, actor, events)
                .await?;
        }
        // A refund the provider failed frees money a reversal's excess was held off.
        self.apply_reversal_excess_tx(conn, actor, parent.id, tenant_id)
            .await?;
        if failed_now > 0 {
            restore_after_failed_refund_tx(conn, &*self.hooks.credit, &parent, failed_now).await?;
        }

        Ok(updated.unwrap_or(parent))
    }

    async fn settle_refund_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        refund: PaymentTransaction,
        outcome: RefundSettlement,
    ) -> StoreResult<PaymentTransaction> {
        // Parent before child, the order `record_observed_refunds_tx` takes: the two paths race
        // whenever `refund.updated` and `charge.refunded` arrive together, and locking the other
        // way round deadlocks one of them into a webhook retry.
        let (lock, parent) = match refund.parent_transaction_id {
            Some(parent_id) => {
                let (lock, parent) =
                    PaymentTransactionRow::lock_with_customer(conn, refund.tenant_id, parent_id)
                        .await?;
                (lock, Some(PaymentTransaction::from(parent)))
            }
            None => {
                let customer =
                    PaymentTransactionRow::customer_of(conn, refund.tenant_id, refund.id).await?;
                let lock =
                    diesel_models::customers::CustomerRow::lock(conn, refund.tenant_id, customer)
                        .await?;
                (lock, None)
            }
        };
        // Re-read under that lock: the row this outcome was built from may have reached a
        // terminal state since, and a stale Settled must not overwrite a Failed.
        let refund: PaymentTransaction =
            PaymentTransactionRow::get_by_id_for_update(conn, &lock, refund.id)
                .await?
                .into();

        let Some(updated) = patch_refund_row(conn, &refund, &outcome.status).await? else {
            return Ok(refund);
        };
        // A provider refund drawn on refund notes in parts is one refund at the provider.
        let mut moved = vec![updated.clone()];
        if let (Some(parent_id), Some(provider_id)) = (
            updated.parent_transaction_id,
            updated.provider_transaction_id.clone(),
        ) {
            for part in PaymentTransactionRow::find_refund_by_provider_id(conn, &lock, &provider_id)
                .await?
                .into_iter()
                .map(Into::<PaymentTransaction>::into)
                .filter(|part| {
                    part.id != updated.id && part.parent_transaction_id == Some(parent_id)
                })
            {
                if let Some(part) = patch_refund_row(conn, &part, &outcome.status).await? {
                    moved.push(part);
                }
            }
        }

        let tenant_id = updated.tenant_id;
        let invoice_id = updated.invoice_id;
        let mut events = moved
            .iter()
            .map(|row| OutboxEvent::payment_transaction_saved(row.clone().into()))
            .collect::<Vec<_>>();

        match outcome.status {
            RefundSettlementStatus::Settled { .. } => {
                if let Some(parent) = parent.as_ref() {
                    // No `payment_transaction_saved` for the parent: it stays Settled and its
                    // invoice is untouched, so re-broadcasting it would re-run the settled
                    // orchestration (see `reinstate_transaction_tx`).
                    let _ = sync_parent_refunded(conn, parent).await?;
                }
                // Its avoir already shrank the invoice; now the money is gone the payment no
                // longer covers what it did, so re-derive the balance. A refund with no avoir
                // leaves the invoice alone.
                if let Some(invoice_id) =
                    invoice_id.filter(|_| moved.iter().any(|row| row.credit_note_id.is_some()))
                {
                    let (lock, _, _) =
                        recompute_invoice_payment_state(conn, tenant_id, invoice_id, None).await?;
                    crate::services::credit::reconcile_purchased_credit_tx(
                        conn,
                        &*self.hooks.credit,
                        &lock,
                        invoice_id,
                        None,
                    )
                    .await?;
                }
                if let Some(invoice_id) = invoice_id {
                    crate::services::credit::collected_payment::reconcile_overpayment_tx(
                        conn,
                        &*self.hooks.credit,
                        tenant_id,
                        invoice_id,
                        None,
                        crate::domain::wallet_events::CreditUnrecoveredCause::PaymentRefunded,
                        None,
                    )
                    .await?;
                }
                events.extend(
                    moved
                        .iter()
                        .map(|row| OutboxEvent::refund_settled(row.clone().into())),
                );
            }
            RefundSettlementStatus::Failed { .. } => {
                // A failed refund gave nothing back: a reversal's excess it held off applies now.
                if let Some(parent) = parent.as_ref() {
                    self.apply_reversal_excess_tx(conn, actor, parent.id, parent.tenant_id)
                        .await?;
                    let failed: i64 = moved
                        .iter()
                        .filter(|row| row.credit_note_id.is_none())
                        .map(|row| row.amount)
                        .sum();
                    if failed > 0 {
                        restore_after_failed_refund_tx(conn, &*self.hooks.credit, parent, failed)
                            .await?;
                    }
                    if failed > 0 {
                        let moved = PaymentMovement::refund(parent.id, failed, chrono::Utc::now());
                        crate::services::credit::collected_payment::follow_collected_payment_tx(
                            conn,
                            &*self.hooks.credit,
                            tenant_id,
                            parent.id,
                            Some(&moved),
                        )
                        .await?;
                    }
                }
                // The overpayment credit it held back returns.
                if let Some(invoice_id) = invoice_id {
                    crate::services::credit::collected_payment::reconcile_overpayment_tx(
                        conn,
                        &*self.hooks.credit,
                        tenant_id,
                        invoice_id,
                        None,
                        crate::domain::wallet_events::CreditUnrecoveredCause::PaymentRefunded,
                        None,
                    )
                    .await?;
                }
                events.extend(
                    moved
                        .iter()
                        .map(|row| OutboxEvent::refund_failed(row.clone().into())),
                );
            }
            RefundSettlementStatus::Pending { .. } => {}
        }

        self.internal
            .record_outbox_batch_tx(conn, tenant_id, actor, events)
            .await?;

        Ok(updated)
    }

    async fn refundable_amount_tx(
        &self,
        conn: &mut PgConn,
        parent: &PaymentTransaction,
    ) -> StoreResult<i64> {
        use diesel_models::enums::PaymentStatusEnum as DbStatus;

        let in_flight = PaymentTransactionRow::sum_refunds_by_parent(
            conn,
            parent.id,
            parent.tenant_id,
            &[DbStatus::Ready, DbStatus::Pending],
        )
        .await?;

        Ok((parent.amount - parent.amount_refunded - parent.amount_reversed - in_flight).max(0))
    }

    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction> {
        PaymentTransactionRow::lock_with_customer(conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|(_, row)| row.into())
    }

    async fn get_payment_tx_by_id(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<PaymentTransaction> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::get_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(std::convert::Into::into)
    }

    async fn get_payment_tx_next_action(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        self.get_payment_tx_row(tenant_id, id)
            .await
            .map(|row| row.next_action)
    }

    async fn get_payment_tx_row(
        &self,
        tenant_id: TenantId,
        id: PaymentTransactionId,
    ) -> StoreResult<PaymentTransactionRow> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::get_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
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

/// Re-derive an invoice's `amount_due` from its net settled payments and finalized
/// invoice-reducing credit notes, then re-derive its payment status. The invoice row is locked
/// first; callers must already be inside a transaction.
/// Returns the invoice and whether this call is what turned it Paid — the only moment an
/// `invoice_paid` event is owed. An invoice that was already Paid is left alone entirely, so a
/// recompute never re-stamps `paid_at` with a later refund's or credit note's timestamp.
pub(crate) async fn recompute_invoice_payment_state(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
    paid_at: Option<chrono::NaiveDateTime>,
) -> StoreResult<(CustomerLock, InvoiceRow, bool)> {
    let before = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id).await?;
    let was_paid = before.invoice.payment_status == InvoicePaymentStatus::Paid;
    let recomputed =
        InvoiceRow::recompute_amount_due_from_settled_payments(conn, invoice_id, tenant_id).await?;

    // Measured against what the invoice still collects: its issued `total` never moves, so a
    // fully credited invoice with nothing paid would otherwise read as partially paid.
    let collectible = InvoiceRow::collectible_total(conn, invoice_id, tenant_id).await?;
    let payment_status = if recomputed.amount_due <= 0 {
        InvoicePaymentStatus::Paid
    } else if recomputed.amount_due >= collectible {
        InvoicePaymentStatus::Unpaid
    } else {
        InvoicePaymentStatus::PartiallyPaid
    };

    if was_paid && payment_status == InvoicePaymentStatus::Paid {
        return Ok((before.lock, recomputed, false));
    }

    let updated = InvoiceRow::apply_payment_status(
        conn,
        invoice_id,
        tenant_id,
        payment_status.clone(),
        paid_at,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok((
        before.lock,
        updated,
        payment_status == InvoicePaymentStatus::Paid,
    ))
}

/// A refund without a note failed: the money it held back backs the credit it took again. Run
/// after applying the reversal excess the refund held off, which may take that money instead.
async fn restore_after_failed_refund_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    parent: &PaymentTransaction,
    failed_minor: i64,
) -> StoreResult<()> {
    let Some(invoice_id) = parent.invoice_id else {
        return Ok(());
    };
    let lock = InvoiceRow::select_for_update_by_id(conn, parent.tenant_id, invoice_id)
        .await?
        .lock;
    let moved = PaymentMovement::refund(parent.id, failed_minor, chrono::Utc::now());
    crate::services::credit::reconcile_purchased_credit_tx(
        conn,
        controls,
        &lock,
        invoice_id,
        Some(&moved),
    )
    .await
}

/// Apply an outcome to a REFUND row, leaving its parent alone. `None` means the row was already
/// terminal (or already in the reported Pending state), which makes redelivery a no-op.
async fn patch_refund_row(
    conn: &mut PgConn,
    refund: &PaymentTransaction,
    outcome: &RefundSettlementStatus,
) -> StoreResult<Option<PaymentTransaction>> {
    use diesel_models::enums::PaymentStatusEnum as DbStatus;
    use domain::enums::PaymentStatusEnum as S;

    if !matches!(refund.status, S::Ready | S::Pending) {
        log::info!(
            "Refund {} is already {:?}; outcome is a no-op",
            refund.id,
            refund.status
        );
        return Ok(None);
    }

    let patch = match outcome {
        RefundSettlementStatus::Settled {
            processed_at,
            external_refund_id,
        } => PaymentTransactionRowPatch {
            id: refund.id,
            status: Some(DbStatus::Settled),
            processed_at: Some(Some(*processed_at)),
            provider_transaction_id: Some(Some(external_refund_id.clone())),
            error_type: Some(None),
            ..Default::default()
        },
        RefundSettlementStatus::Failed {
            error,
            external_refund_id,
        } => PaymentTransactionRowPatch {
            id: refund.id,
            status: Some(DbStatus::Failed),
            processed_at: Some(Some(chrono::Utc::now().naive_utc())),
            provider_transaction_id: external_refund_id.clone().map(Some),
            error_type: Some(Some(error.clone())),
            ..Default::default()
        },
        RefundSettlementStatus::Pending { external_refund_id } => {
            let already_known = refund.status == S::Pending
                && refund.provider_transaction_id.as_deref() == Some(external_refund_id.as_str());
            if already_known {
                return Ok(None);
            }
            PaymentTransactionRowPatch {
                id: refund.id,
                status: Some(DbStatus::Pending),
                provider_transaction_id: Some(Some(external_refund_id.clone())),
                ..Default::default()
            }
        }
    };

    Ok(Some(patch.update(conn).await?.into()))
}

/// Re-derive a payment's voluntary refunded total from its settled REFUND rows and flip it to
/// Refunded once nothing is left in the merchant's hands. Deriving it rather than adding a delta
/// keeps it exact under redelivery and when online and offline refunds mix. `refunded_at` is
/// deliberately NOT stamped: it is the involuntary reversal cycle's high-water mark and the
/// redelivery guards in `reverse_transaction_tx` key off it.
pub(crate) async fn sync_parent_refunded(
    conn: &mut PgConn,
    parent: &PaymentTransaction,
) -> StoreResult<Option<PaymentTransaction>> {
    use domain::enums::PaymentStatusEnum as S;

    let new_refunded = PaymentTransactionRow::sum_refunds_by_parent(
        conn,
        parent.id,
        parent.tenant_id,
        &[diesel_models::enums::PaymentStatusEnum::Settled],
    )
    .await?
    .clamp(0, parent.amount);
    let new_status = if new_refunded + parent.amount_reversed >= parent.amount {
        S::Refunded
    } else {
        parent.status.clone()
    };

    if new_refunded == parent.amount_refunded && new_status == parent.status {
        return Ok(None);
    }

    let status_changed = new_status != parent.status;
    let patch = PaymentTransactionRowPatch {
        id: parent.id,
        status: status_changed.then(|| new_status.into()),
        amount_refunded: Some(new_refunded),
        ..Default::default()
    };
    Ok(Some(patch.update(conn).await?.into()))
}

/// A refund recorded from the provider, and what it gave back of the purchase (beyond the excess,
/// not drawn on a note).
struct ObservedRow {
    refund: PaymentTransaction,
    purchase_minor: i64,
}

impl ObservedRow {
    fn events(&self, status: &diesel_models::enums::PaymentStatusEnum) -> [OutboxEvent; 2] {
        [
            OutboxEvent::payment_transaction_saved(self.refund.clone().into()),
            match status {
                diesel_models::enums::PaymentStatusEnum::Pending => {
                    OutboxEvent::refund_issued(self.refund.clone().into())
                }
                _ => OutboxEvent::refund_settled(self.refund.clone().into()),
            },
        ]
    }
}

/// Record a refund the provider made that we hold no row for (a dashboard refund, or the part of
/// a cumulative total nothing else accounts for). It draws first on what the invoice's finalized
/// refund notes still owe back, oldest first, so that sum is never refunded twice; each part keeps
/// the provider's refund id.
async fn insert_observed_refund(
    conn: &mut PgConn,
    parent: &PaymentTransaction,
    amount: i64,
    status: diesel_models::enums::PaymentStatusEnum,
    provider_transaction_id: Option<String>,
    observed_at: chrono::NaiveDateTime,
) -> StoreResult<Vec<ObservedRow>> {
    use diesel_models::credit_notes::CreditNoteRow;
    use diesel_models::enums::PaymentStatusEnum as DbStatus;
    let mut parts: Vec<(Option<CreditNoteId>, i64)> = Vec::new();
    let mut rest = amount;
    if let Some(invoice_id) = parent.invoice_id {
        let notes =
            CreditNoteRow::list_finalized_refund_ids(conn, invoice_id, parent.tenant_id).await?;
        if !notes.is_empty() {
            let invoice_lock =
                InvoiceRow::select_for_update_by_id(conn, parent.tenant_id, invoice_id)
                    .await?
                    .lock;
            for note_id in notes {
                if rest <= 0 {
                    break;
                }
                let note =
                    CreditNoteRow::select_for_update_by_id(conn, &invoice_lock, note_id).await?;
                let drawn = PaymentTransactionRow::sum_refunds_by_credit_note(
                    conn,
                    note_id,
                    parent.tenant_id,
                    &[DbStatus::Settled, DbStatus::Pending, DbStatus::Ready],
                )
                .await?;
                let take = rest.min(note.refunded_amount_cents - drawn).max(0);
                if take > 0 {
                    parts.push((Some(note_id), take));
                    rest -= take;
                }
            }
        }
    }
    if rest > 0 {
        parts.push((None, rest));
    }
    let settled = status == DbStatus::Settled;
    let mut rows = Vec::with_capacity(parts.len());
    for (credit_note_id, amount) in parts {
        let row = PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id: parent.tenant_id,
            invoice_id: parent.invoice_id,
            provider_transaction_id: provider_transaction_id.clone(),
            amount,
            currency: parent.currency.clone(),
            payment_method_id: parent.payment_method_id,
            status: status.clone(),
            payment_type: diesel_models::enums::PaymentTypeEnum::Refund,
            error_type: None,
            processed_at: settled.then_some(observed_at),
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: Some(parent.id),
            credit_note_id,
            refund_mode: Some(diesel_models::enums::RefundModeEnum::Online),
            reversal_reason: None,
        };
        let refund: PaymentTransaction = row.insert(conn).await?.into();
        let excess =
            crate::repositories::payment_reversal::record_refund_excess_tx(conn, &refund).await?;
        let purchase_minor = if credit_note_id.is_none() {
            amount - excess
        } else {
            0
        };
        rows.push(ObservedRow {
            refund,
            purchase_minor,
        });
    }
    Ok(rows)
}

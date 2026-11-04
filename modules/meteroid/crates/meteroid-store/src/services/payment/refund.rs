//! Voluntary refunds: giving settled money back on purpose.
//!
//! A refund is a child REFUND row on the settled parent payment; it never moves the invoice's
//! `amount_due` or payment status. What reduces the invoice is the finalized `Refund` credit
//! note the refund is drawn against — the only exception is an overpayment ("trop-perçu"),
//! which may be refunded with no credit note, bounded by what was collected above the invoice
//! total.
//!
//! An INVOLUNTARY claw-back (chargeback, bank recall, lost dispute, failed collection) is the
//! other mechanism: it mutates the parent row's `amount_reversed`, keeping the invoice paid for a
//! dispute and reopening it for a failed collection — see `reverse_transaction_tx`, whose
//! manual-payment entry point is `reverse_manual_payment` below.

use crate::StoreResult;
use crate::adapters::payment::error::ConnectorError;
use crate::adapters::payment::model::{IdempotencyKey, RefundOutcome, RefundReason, RefundRequest};
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{CreditNoteStatus, CreditType, ReversalKind};
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::repositories::payment_transactions::{
    PaymentTransactionInterface, RefundSettlement, RefundSettlementStatus, ReversalAmount,
    TransactionReversal, sync_parent_refunded,
};
use crate::services::Services;
use crate::services::payment::process_payment::PAYMENT_PROVIDER_TIMEOUT;
use crate::store::PgConn;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, CreditNoteId, PaymentTransactionId, TenantId};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum, RefundModeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;

/// Refund a settled online payment through its provider.
#[derive(Debug, Clone)]
pub struct RefundCommand {
    pub parent_transaction_id: PaymentTransactionId,
    /// Minor units, in the parent's currency (no FX).
    pub amount: i64,
    /// The finalized Refund credit note the refund draws on; `None` is only allowed for an
    /// overpayment.
    pub credit_note_id: Option<CreditNoteId>,
    pub reason: Option<String>,
}

/// Record a refund that moved outside Meteroid (wire, cheque, cash), on any parent.
#[derive(Debug, Clone)]
pub struct OfflineRefundCommand {
    pub parent_transaction_id: PaymentTransactionId,
    pub amount: i64,
    pub credit_note_id: Option<CreditNoteId>,
    pub refund_date: NaiveDateTime,
    /// Wire/cheque reference, stored like a manual payment's.
    pub reference: Option<String>,
    pub reason: Option<String>,
}

/// Claw back a manually recorded payment. A chargeback, recall or dispute keeps the invoice paid
/// and books the loss; any other kind means the money never landed, so the invoice reopens.
#[derive(Debug, Clone)]
pub struct ReverseManualPaymentCommand {
    pub transaction_id: PaymentTransactionId,
    pub kind: ReversalKind,
    pub reason: Option<String>,
    pub reversed_at: NaiveDateTime,
}

impl Services {
    /// Issue an online refund: validate and commit a `Ready` REFUND row, call the provider with
    /// an idempotency key derived from that row, then apply the outcome.
    pub(in crate::services) async fn refund_transaction(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: RefundCommand,
    ) -> StoreResult<PaymentTransaction> {
        // Committed before the provider call so the idempotency key `refund:{id}` is derived
        // from a row that already exists — the same reasoning as a charge.
        let (parent, refund) = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                let command = command.clone();
                async move {
                    let parent = self
                        .store
                        .get_payment_tx_by_id_for_update(
                            conn,
                            command.parent_transaction_id,
                            tenant_id,
                        )
                        .await?;

                    validate_refund_draw(
                        self,
                        conn,
                        tenant_id,
                        &parent,
                        command.amount,
                        command.credit_note_id,
                    )
                    .await?;

                    if parent.payment_method_id.is_none() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "This payment was recorded manually; use an offline refund".into(),
                        )));
                    }

                    let refund = insert_refund_row(
                        conn,
                        tenant_id,
                        &parent,
                        command.amount,
                        command.credit_note_id,
                        RefundModeEnum::Online,
                        PaymentStatusEnum::Ready,
                        None,
                        None,
                        command.reason.clone(),
                    )
                    .await?;
                    // Issued without a note, it takes its share of the overpayment credit back
                    // at once, so the credit cannot be spent while the money is on its way out.
                    if command.credit_note_id.is_none()
                        && let Some(invoice_id) = parent.invoice_id
                    {
                        crate::services::credit::collected_payment::reconcile_overpayment_tx(
                            conn,
                            &*self.store.hooks.credit,
                            tenant_id,
                            invoice_id,
                            None,
                            crate::domain::wallet_events::CreditUnrecoveredCause::PaymentRefunded,
                            None,
                        )
                        .await?;
                    }

                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![
                                OutboxEvent::refund_issued(refund.clone().into()),
                                OutboxEvent::payment_transaction_saved(refund.clone().into()),
                            ],
                        )
                        .await?;

                    Ok((parent, refund))
                }
                .scope_boxed()
            })
            .await?;

        self.dispatch_refund(&actor, tenant_id, &parent, refund, command.reason)
            .await
    }

    /// Asks the provider for a committed `Ready` refund and records the outcome on its row.
    async fn dispatch_refund(
        &self,
        actor: &Actor,
        tenant_id: TenantId,
        parent: &PaymentTransaction,
        refund: PaymentTransaction,
        reason: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        let outcome = self
            .call_provider_refund(tenant_id, parent, &refund, reason)
            .await;

        // `failure` is the error to surface once the row carries the outcome: the caller must
        // not read a refused refund as an accepted one.
        let (status, failure) = match outcome {
            Ok(RefundOutcome::Succeeded(receipt)) => (
                RefundSettlementStatus::Settled {
                    processed_at: receipt.processed_at,
                    external_refund_id: receipt.external_refund_id,
                },
                None,
            ),
            Ok(RefundOutcome::Pending(ack)) => (
                RefundSettlementStatus::Pending {
                    external_refund_id: ack.external_refund_id,
                },
                None,
            ),
            Ok(RefundOutcome::Failed(refusal)) => (
                RefundSettlementStatus::Failed {
                    error: refusal.message.clone(),
                    external_refund_id: None,
                },
                Some(StoreError::PaymentError(refusal.message)),
            ),
            // A transport error says nothing about whether the provider took the refund. Leave
            // the row Ready — the sweep only polls Pending rows, and re-issuing it on the same
            // row (hence the same idempotency key) is a follow-up.
            Err(err) if matches!(err.current_context(), ConnectorError::Transport(_)) => {
                log::error!(
                    "Refund {} on transaction {} could not reach the provider; left Ready for manual retry: {err:?}",
                    refund.id,
                    parent.id,
                );
                return Err(err.change_context(StoreError::PaymentProviderError));
            }
            Err(err) => {
                let message = err.current_context().to_string();
                // A capability or configuration refusal is the caller's mistake, not the
                // provider's failure — the dialog should never have offered the action.
                let error = match err.current_context() {
                    ConnectorError::Unsupported { .. } | ConnectorError::Configuration(_) => {
                        StoreError::InvalidArgument(message.clone())
                    }
                    _ => StoreError::PaymentError(message.clone()),
                };
                (
                    RefundSettlementStatus::Failed {
                        error: message,
                        external_refund_id: None,
                    },
                    Some(error),
                )
            }
        };

        let settled = self
            .store
            .transaction(|conn| {
                let refund = refund.clone();
                let status = status.clone();
                async move {
                    self.store
                        .settle_refund_tx(conn, actor, refund, RefundSettlement { status })
                        .await
                }
                .scope_boxed()
            })
            .await?;

        match failure {
            Some(error) => Err(Report::new(error)),
            None => Ok(settled),
        }
    }

    /// Refunds a captured top-up that no purchase will fund. A refusal leaves a failed refund and
    /// an error log; it never fails the settlement pipeline.
    pub(in crate::services) async fn refund_detached_capture(
        &self,
        tenant_id: TenantId,
        payment_id: PaymentTransactionId,
    ) -> StoreResult<()> {
        const REASON: &str = "Duplicate top-up capture: the credit was funded by another charge";
        let prepared = self
            .store
            .transaction(|conn| {
                async move {
                    let parent = self
                        .store
                        .get_payment_tx_by_id_for_update(conn, payment_id, tenant_id)
                        .await?;
                    let detached = matches!(
                        parent.payment_type,
                        crate::domain::enums::PaymentTypeEnum::TopUp
                            | crate::domain::enums::PaymentTypeEnum::AutoTopup
                    ) && parent.invoice_id.is_none()
                        && parent.status == crate::domain::enums::PaymentStatusEnum::Settled;
                    if !detached {
                        return Ok(None);
                    }
                    let refundable = self.store.refundable_amount_tx(conn, &parent).await?;
                    if refundable <= 0 {
                        return Ok(None);
                    }
                    let refund = insert_refund_row(
                        conn,
                        tenant_id,
                        &parent,
                        refundable,
                        None,
                        RefundModeEnum::Online,
                        PaymentStatusEnum::Ready,
                        None,
                        None,
                        Some(REASON.to_owned()),
                    )
                    .await?;
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            vec![
                                OutboxEvent::refund_issued(refund.clone().into()),
                                OutboxEvent::payment_transaction_saved(refund.clone().into()),
                            ],
                        )
                        .await?;
                    Ok(Some((parent, refund)))
                }
                .scope_boxed()
            })
            .await?;
        let Some((parent, refund)) = prepared else {
            return Ok(());
        };
        let refund_id = refund.id;
        match self
            .dispatch_refund(
                &Actor::System,
                tenant_id,
                &parent,
                refund,
                Some(REASON.to_owned()),
            )
            .await
        {
            Ok(_) => log::warn!(
                "top-up payment {payment_id} of tenant {tenant_id} was captured without a purchase to fund; refund {refund_id} issued"
            ),
            Err(err) => log::error!(
                "top-up payment {payment_id} of tenant {tenant_id} was captured without a purchase to fund and its refund {refund_id} did not go through; refund it at the payment provider: {err:?}"
            ),
        }
        Ok(())
    }

    /// Re-issue a refund whose provider call never came back. The row still carries the
    /// idempotency key it was created with, so a provider that honours it returns the original
    /// refund rather than moving money a second time — which is why only such a provider is
    /// swept. Idempotent: a row that already reached a terminal state is left alone.
    pub(in crate::services) async fn replay_unconfirmed_refund(
        &self,
        refund_id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let row = PaymentTransactionRow::get_by_id(&mut conn, refund_id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        if row.status != PaymentStatusEnum::Ready {
            return Ok(());
        }
        let Some(parent_id) = row.parent_transaction_id else {
            log::warn!("Refund {refund_id} has no parent payment; cannot replay it");
            return Ok(());
        };

        let (_, connector_impl) =
            super::resolve_connector_for_transaction(&self.store, &mut conn, &row).await?;
        let replayable = connector_impl
            .capabilities()
            .refund_replay_window
            .and_then(|window| chrono::Duration::from_std(window).ok())
            .is_some_and(|window| chrono::Utc::now() - row.created_at < window);
        if !replayable {
            log::warn!(
                "Refund {refund_id} was never confirmed and can no longer be replayed safely at \
                 this provider; it needs to be resolved by hand"
            );
            return Ok(());
        }
        drop(conn);

        let parent = self
            .store
            .get_payment_tx_by_id(tenant_id, parent_id)
            .await?;
        let refund: PaymentTransaction = row.into();
        let reason = refund.reversal_reason.clone();

        let outcome = self
            .call_provider_refund(tenant_id, &parent, &refund, reason)
            .await;

        let status = match outcome {
            Ok(RefundOutcome::Succeeded(receipt)) => RefundSettlementStatus::Settled {
                processed_at: receipt.processed_at,
                external_refund_id: receipt.external_refund_id,
            },
            Ok(RefundOutcome::Pending(ack)) => RefundSettlementStatus::Pending {
                external_refund_id: ack.external_refund_id,
            },
            Ok(RefundOutcome::Failed(refusal)) => RefundSettlementStatus::Failed {
                error: refusal.message,
                external_refund_id: None,
            },
            // Still unreachable: leave it Ready for the next sweep rather than guessing.
            Err(err) => return Err(err.change_context(StoreError::PaymentProviderError)),
        };

        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let refund = refund.clone();
                let status = status.clone();
                async move {
                    store
                        .settle_refund_tx(conn, &Actor::System, refund, RefundSettlement { status })
                        .await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await?;
        Ok(())
    }

    /// Record a refund that moved outside Meteroid. Allowed on any settled parent, provider
    /// backed or manual, since no provider call is involved.
    pub(in crate::services) async fn record_offline_refund(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: OfflineRefundCommand,
    ) -> StoreResult<PaymentTransaction> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let command = command.clone();
                async move {
                    let parent = self
                        .store
                        .get_payment_tx_by_id_for_update(
                            conn,
                            command.parent_transaction_id,
                            tenant_id,
                        )
                        .await?;

                    validate_refund_draw(
                        self,
                        conn,
                        tenant_id,
                        &parent,
                        command.amount,
                        command.credit_note_id,
                    )
                    .await?;

                    let refund = insert_refund_row(
                        conn,
                        tenant_id,
                        &parent,
                        command.amount,
                        command.credit_note_id,
                        RefundModeEnum::Offline,
                        PaymentStatusEnum::Settled,
                        Some(command.refund_date),
                        command.reference.clone(),
                        command.reason.clone(),
                    )
                    .await?;

                    let _ = sync_parent_refunded(conn, &parent).await?;
                    if let (Some(_), Some(invoice_id)) = (command.credit_note_id, parent.invoice_id)
                    {
                        let (lock, _, _) = crate::repositories::payment_transactions::recompute_invoice_payment_state(
                            conn, tenant_id, invoice_id, None,
                        )
                        .await?;
                        crate::services::credit::reconcile_purchased_credit_tx(
                            conn,
                            &*self.store.hooks.credit, &lock, invoice_id, None,
                        )
                        .await?;
                    }
                    if let Some(invoice_id) = parent.invoice_id {
                        crate::services::credit::collected_payment::reconcile_overpayment_tx(
                            conn,
                            &*self.store.hooks.credit, tenant_id, invoice_id, None, crate::domain::wallet_events::CreditUnrecoveredCause::PaymentRefunded, None
                        )
                        .await?;
                    }

                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![
                                OutboxEvent::refund_issued(refund.clone().into()),
                                OutboxEvent::refund_settled(refund.clone().into()),
                                OutboxEvent::payment_transaction_saved(refund.clone().into()),
                            ],
                        )
                        .await?;

                    Ok(refund)
                }
                .scope_boxed()
            })
            .await
    }

    /// Claw back a manually recorded payment in full (see [`ReverseManualPaymentCommand`]).
    pub(in crate::services) async fn reverse_manual_payment(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: ReverseManualPaymentCommand,
    ) -> StoreResult<PaymentTransaction> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let command = command.clone();
                async move {
                    let transaction = self
                        .store
                        .get_payment_tx_by_id_for_update(conn, command.transaction_id, tenant_id)
                        .await?;

                    use crate::domain::enums::PaymentTypeEnum;
                    let top_up = matches!(
                        transaction.payment_type,
                        PaymentTypeEnum::TopUp | PaymentTypeEnum::AutoTopup
                    );
                    if transaction.payment_type != PaymentTypeEnum::Payment && !top_up {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Only a payment can be reversed".into(),
                        )));
                    }

                    // A provider-backed payment is clawed back by the provider's own event. A top-up
                    // charge is the exception: its credit is spendable at once, so a chargeback the
                    // provider does not report must still be recordable (a later event is a no-op).
                    if transaction.payment_method_id.is_some() && !top_up {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Only a manually recorded payment can be reversed here".into(),
                        )));
                    }

                    let reversal = TransactionReversal {
                        external_transaction_id: transaction
                            .provider_transaction_id
                            .clone()
                            .unwrap_or_default(),
                        amount: ReversalAmount::Full,
                        kind: command.kind,
                        reason: command.reason.clone(),
                        reversed_at: command.reversed_at,
                        disputed: command.kind.is_dispute(),
                        provider_ref: None,
                    };

                    self.store
                        .reverse_transaction_tx(conn, actor, transaction, reversal)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// Resolve the connector from the parent's payment method → customer connection, exactly as
    /// a charge does, then ask it to refund.
    async fn call_provider_refund(
        &self,
        tenant_id: TenantId,
        parent: &PaymentTransaction,
        refund: &PaymentTransaction,
        reason: Option<String>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        let mut conn = self
            .store
            .get_conn()
            .await
            .change_context(ConnectorError::Database)?;

        let method_id = parent.payment_method_id.ok_or_else(|| {
            Report::new(ConnectorError::Configuration(
                "This payment was recorded manually; use an offline refund".into(),
            ))
        })?;

        let (connector, connector_impl) =
            super::resolve_connector_for_method(&self.store, &mut conn, tenant_id, method_id)
                .await
                .change_context(ConnectorError::Database)?;
        connector
            .ensure_active_locked(&mut conn)
            .await
            .change_context(ConnectorError::Database)?;

        let capabilities = connector_impl.capabilities();
        if !capabilities.supports_refunds() {
            return Err(Report::new(ConnectorError::Unsupported {
                provider: connector.provider(),
                capability: "refunds",
            }));
        }
        if refund.amount < parent.amount && !capabilities.supports_partial_refunds() {
            return Err(Report::new(ConnectorError::Unsupported {
                provider: connector.provider(),
                capability: "partial refunds",
            }));
        }

        let external_transaction_id = parent.provider_transaction_id.clone().ok_or_else(|| {
            Report::new(ConnectorError::Configuration(
                "This payment has no provider reference; use an offline refund".into(),
            ))
        })?;

        let request = RefundRequest {
            external_transaction_id: &external_transaction_id,
            amount_minor: refund.amount,
            currency: &refund.currency,
            reason: Some(match reason {
                Some(reason) => RefundReason::Other(reason),
                None => RefundReason::RequestedByCustomer,
            }),
            idempotency_key: IdempotencyKey::new(format!("refund:{}", refund.id.as_base62())),
            metadata: vec![
                ("meteroid.refund_id".to_string(), refund.id.as_base62()),
                ("meteroid.tenant_id".to_string(), tenant_id.as_base62()),
                ("meteroid.transaction_id".to_string(), parent.id.as_base62()),
            ],
        };

        let outcome =
            tokio::time::timeout(PAYMENT_PROVIDER_TIMEOUT, connector_impl.refund(request))
                .await
                .map_err(|_| {
                    Report::new(ConnectorError::Transport(
                        "Refund request timed out".to_string(),
                    ))
                })?;

        self.store
            .note_rejected_credentials(&connector, &outcome)
            .await;

        outcome
    }
}

/// Shared gate for both refund entry points: the parent must be a settled payment with enough
/// left to give back, and the draw must be covered either by a finalized Refund credit note or
/// by an overpayment on the invoice.
async fn validate_refund_draw(
    services: &Services,
    conn: &mut PgConn,
    tenant_id: TenantId,
    parent: &PaymentTransaction,
    amount: i64,
    credit_note_id: Option<CreditNoteId>,
) -> StoreResult<()> {
    use crate::domain::enums::PaymentStatusEnum as S;

    if !parent.payment_type.is_collection() {
        return Err(Report::new(StoreError::InvalidArgument(
            "Only a payment can be refunded".into(),
        )));
    }
    if parent.status != S::Settled {
        return Err(Report::new(StoreError::InvalidArgument(
            "Only a settled payment can be refunded".into(),
        )));
    }
    if amount <= 0 {
        return Err(Report::new(StoreError::InvalidArgument(
            "Refund amount must be positive".into(),
        )));
    }

    let refundable = services.store.refundable_amount_tx(conn, parent).await?;
    if amount > refundable {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Refund amount ({amount}) exceeds what is still refundable on this payment ({refundable})"
        ))));
    }

    match credit_note_id {
        Some(credit_note_id) => {
            validate_credit_note_draw(conn, tenant_id, parent, amount, credit_note_id).await
        }
        None => validate_overpayment_draw(conn, tenant_id, parent, amount).await,
    }
}

async fn validate_credit_note_draw(
    conn: &mut PgConn,
    tenant_id: TenantId,
    parent: &PaymentTransaction,
    amount: i64,
    credit_note_id: CreditNoteId,
) -> StoreResult<()> {
    let another_invoice = || {
        Report::new(StoreError::InvalidArgument(
            "The credit note belongs to another invoice than this payment".into(),
        ))
    };
    // Locked (behind its customer and invoice, as a void) before anything is read off it: a void
    // would otherwise slip in between the status check and the draw, and two refunds on different
    // payments of the same invoice would each see the full remainder.
    let invoice_id = parent.invoice_id.ok_or_else(another_invoice)?;
    let lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .lock;
    let credit_note = CreditNoteRow::select_for_update_by_id(conn, &lock, credit_note_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if CreditNoteStatus::from(credit_note.status) != CreditNoteStatus::Finalized {
        return Err(Report::new(StoreError::InvalidArgument(
            "The credit note must be finalized before it can be refunded".into(),
        )));
    }
    if CreditType::from(credit_note.credit_type) != CreditType::Refund {
        return Err(Report::new(StoreError::InvalidArgument(
            "Only a refund credit note can back a refund".into(),
        )));
    }
    if credit_note.invoice_id != invoice_id {
        return Err(another_invoice());
    }

    let drawn = PaymentTransactionRow::sum_refunds_by_credit_note(
        conn,
        credit_note_id,
        tenant_id,
        &[
            PaymentStatusEnum::Settled,
            PaymentStatusEnum::Pending,
            PaymentStatusEnum::Ready,
        ],
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let remaining = credit_note.refunded_amount_cents - drawn;
    if amount > remaining {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Refund amount ({amount}) exceeds what this credit note still allows ({remaining})"
        ))));
    }

    Ok(())
}

/// The only refund allowed without an avoir: handing back what was collected above the invoice
/// total (duplicate charge, overpayment).
async fn validate_overpayment_draw(
    conn: &mut PgConn,
    tenant_id: TenantId,
    parent: &PaymentTransaction,
    amount: i64,
) -> StoreResult<()> {
    let missing_credit_note = || {
        Report::new(StoreError::InvalidArgument(
            "a finalized refund credit note is required".to_string(),
        ))
    };

    let Some(invoice_id) = parent.invoice_id else {
        return Err(missing_credit_note());
    };

    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let transactions = PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let collected: i64 = transactions
        .iter()
        .map(|t| &t.transaction)
        .filter(|t| {
            t.payment_type.is_collection()
                && matches!(
                    t.status,
                    PaymentStatusEnum::Settled | PaymentStatusEnum::Refunded
                )
        })
        .map(|t| t.amount - t.amount_reversed)
        .sum();

    // Refunds that claim no avoir: they come out of the same overpaid money this draw wants.
    // A refund drawn on an avoir is already reserved by `entitlement` below, whether or not it
    // has settled, so counting it here too would charge it twice.
    let taken: i64 = transactions
        .iter()
        .map(|t| &t.transaction)
        .filter(|t| {
            t.payment_type == PaymentTypeEnum::Refund
                && t.credit_note_id.is_none()
                && matches!(
                    t.status,
                    PaymentStatusEnum::Ready
                        | PaymentStatusEnum::Pending
                        | PaymentStatusEnum::Settled
                )
        })
        .map(|t| t.amount)
        .sum();

    // Every finalized avoir's refundable amount is spoken for even before it is drawn: it is
    // owed to the customer through the credit note, so it can never double as overpaid money.
    let entitlement = CreditNoteRow::sum_finalized_refundable(conn, invoice_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let cancelled = CreditNoteRow::sum_finalized_invoice_reducing(conn, invoice_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let owed = invoice.invoice.total - invoice.invoice.applied_credits - cancelled;

    let overpaid = collected - owed - entitlement - taken;
    if amount > overpaid {
        return Err(missing_credit_note());
    }
    // Overpayment credit already spent cannot be refunded as well: only its unspent rest can.
    let spent = crate::services::credit::collected_payment::spent_overpayment_credit_tx(
        conn,
        &invoice.lock,
        invoice_id,
    )
    .await?;
    if amount > overpaid - spent {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Refund amount ({amount}) exceeds the unspent overpayment ({}): {spent} of it was already used as customer credit",
            (overpaid - spent).max(0)
        ))));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_refund_row(
    conn: &mut PgConn,
    tenant_id: TenantId,
    parent: &PaymentTransaction,
    amount: i64,
    credit_note_id: Option<CreditNoteId>,
    refund_mode: RefundModeEnum,
    status: PaymentStatusEnum,
    processed_at: Option<NaiveDateTime>,
    provider_transaction_id: Option<String>,
    reason: Option<String>,
) -> StoreResult<PaymentTransaction> {
    let row = PaymentTransactionRowNew {
        id: PaymentTransactionId::new(),
        tenant_id,
        // The parent's invoice, so the refund lists with it; the invoice balance is untouched.
        invoice_id: parent.invoice_id,
        provider_transaction_id,
        amount,
        currency: parent.currency.clone(),
        payment_method_id: parent.payment_method_id,
        status,
        payment_type: PaymentTypeEnum::Refund,
        error_type: None,
        processed_at,
        checkout_session_id: None,
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: None,
        pending_provider_intent_id: None,
        pending_connection_id: None,
        parent_transaction_id: Some(parent.id),
        credit_note_id,
        refund_mode: Some(refund_mode),
        reversal_reason: reason,
    };

    let refund: PaymentTransaction = row
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into();
    crate::repositories::payment_reversal::record_refund_excess_tx(conn, &refund).await?;
    Ok(refund)
}

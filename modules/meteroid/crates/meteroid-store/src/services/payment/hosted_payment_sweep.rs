//! Pending-intent sweeper for hosted in-flow captures on `PollingRequired`
//! providers (no webhooks) — the lost-return backstop over hosted CHECKOUTS
//! and INVOICE payments: a customer who pays but never returns has had money
//! captured while the pre-created transaction stays Pending, so a worker
//! re-runs the SAME completion routine as the return handler for every
//! transaction still carrying a pending intent id.
//!
//! The sweeper never charges — it only records what the hosted page captured,
//! or closes out an abandoned attempt (cancelling its provider intent FIRST
//! so its hosted page can never capture afterwards). Declined attempts stay
//! in the scan until the abandonment cutoff (their page can still capture on
//! a retry); the marker is released only once the attempt is finished, so a
//! settled-but-unmaterialized checkout is re-swept.

use crate::StoreResult;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::payment::hosted_setup::HostedSetupOutcome;
use chrono::{DateTime, Utc};
use common_domain::ids::{
    CheckoutSessionId, CustomerConnectionId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// One hosted payment attempt awaiting completion, projected so the worker
/// stays decoupled from `diesel-models`. Exactly one of
/// `checkout_session_id` / `invoice_id` is set.
#[derive(Debug, Clone)]
pub struct PendingHostedPaymentRef {
    pub tenant_id: TenantId,
    pub transaction_id: PaymentTransactionId,
    pub connection_id: CustomerConnectionId,
    pub intent_id: String,
    pub created_at: DateTime<Utc>,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub invoice_id: Option<InvoiceId>,
}

/// What one sweep pass did for one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedPaymentSweepOutcome {
    /// Captured payment recorded; checkout materialized / invoice settled.
    Completed,
    /// Captured payment declined; left for a retry until the abandonment cutoff.
    Declined,
    /// Nothing to record yet; re-checked on the next sweep.
    StillPending,
    /// The provider ended the attempt before the cutoff (a Mollie page expires within minutes):
    /// intent and transaction cancelled, checkout session reopened for a retry.
    AttemptClosed,
    /// No capture by the cutoff: intent and transaction cancelled (or marker cleared), checkout
    /// session expired.
    CheckoutExpired,
}

/// Pure decision for one sweep pass, kept free of IO so the money-path table
/// is testable. Nothing here ever initiates a charge.
pub(crate) fn sweep_action(
    outcome: &HostedSetupOutcome,
    past_abandon_cutoff: bool,
) -> HostedPaymentSweepOutcome {
    match outcome {
        HostedSetupOutcome::CheckoutActivated(_) => HostedPaymentSweepOutcome::Completed,
        HostedSetupOutcome::InvoiceCharged(_) => HostedPaymentSweepOutcome::Completed,
        // Captured money that could not be reconciled: NEVER expired away,
        // regardless of age — completion logs the manual-review error every pass.
        HostedSetupOutcome::HeldForReview { .. } => HostedPaymentSweepOutcome::StillPending,
        HostedSetupOutcome::PaymentFailed { .. } if !past_abandon_cutoff => {
            HostedPaymentSweepOutcome::Declined
        }
        // Final at the provider: the page can never capture, so close it out now.
        HostedSetupOutcome::SetupFailed { definitive: true } if !past_abandon_cutoff => {
            HostedPaymentSweepOutcome::AttemptClosed
        }
        // Processing / SetupFailed / MethodSaved — and a declined attempt —
        // close out once past the cutoff.
        _ if past_abandon_cutoff => HostedPaymentSweepOutcome::CheckoutExpired,
        _ => HostedPaymentSweepOutcome::StillPending,
    }
}

impl Services {
    /// List up to `limit` attempts (all tenants) still carrying a pending
    /// intent id, initiated before `older_than`. Keyset-ordered on
    /// `(created_at, id)`; the worker rotates the `after` cursor across passes
    /// so a wall of old-but-alive attempts can never starve newer ones.
    pub async fn list_pending_hosted_payments(
        &self,
        older_than: DateTime<Utc>,
        after: Option<(DateTime<Utc>, PaymentTransactionId)>,
        limit: i64,
    ) -> StoreResult<Vec<PendingHostedPaymentRef>> {
        let mut conn = self.store.get_conn().await?;
        let rows = PaymentTransactionRow::list_sweepable_with_pending_intent(
            &mut conn, older_than, after, limit,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let (Some(intent_id), Some(connection_id)) =
                    (row.pending_provider_intent_id, row.pending_connection_id)
                else {
                    // Initiation writes both atomically; half-written data is
                    // a bug, not sweepable work.
                    log::error!(
                        "payment transaction {} has a pending intent without a connection id; skipping",
                        row.id
                    );
                    return None;
                };
                if row.checkout_session_id.is_none() && row.invoice_id.is_none() {
                    log::error!(
                        "payment transaction {} has a pending intent but neither a checkout \
                         session nor an invoice; skipping",
                        row.id
                    );
                    return None;
                }
                Some(PendingHostedPaymentRef {
                    tenant_id: row.tenant_id,
                    transaction_id: row.id,
                    connection_id,
                    intent_id,
                    created_at: row.created_at,
                    checkout_session_id: row.checkout_session_id,
                    invoice_id: row.invoice_id,
                })
            })
            .collect())
    }

    /// Sweep one attempt: run the SAME completion routine as the return
    /// handler (record a captured payment — never charge), then close out
    /// abandoned attempts past `abandoned_before`.
    pub async fn sweep_hosted_payment(
        &self,
        item: &PendingHostedPaymentRef,
        abandoned_before: DateTime<Utc>,
    ) -> StoreResult<HostedPaymentSweepOutcome> {
        // Single attempt: no customer is waiting; the next sweep is the retry.
        let outcome = self
            .complete_hosted_setup_with_attempts(item.connection_id, item.intent_id.clone(), 1)
            .await?;

        let past_cutoff = item.created_at < abandoned_before;
        let action = sweep_action(&outcome, past_cutoff);
        if matches!(
            action,
            HostedPaymentSweepOutcome::CheckoutExpired | HostedPaymentSweepOutcome::AttemptClosed
        ) {
            // The close-out can abort (lost race, uncancelable intent):
            // report the truth — still pending, not expired.
            let expire_session = action == HostedPaymentSweepOutcome::CheckoutExpired;
            if !self.close_out_hosted_attempt(item, expire_session).await? {
                return Ok(HostedPaymentSweepOutcome::StillPending);
            }
        }
        Ok(action)
    }

    /// Closes out a finished attempt: cancels the provider intent, then cancels the transaction
    /// with a status-guarded update (a settled row or captured money is never cancelled) or clears
    /// the marker of a terminal one. The checkout session is expired, or reopened for a retry if
    /// only the attempt failed. The anchor row is locked FOR UPDATE; losing any race aborts.
    async fn close_out_hosted_attempt(
        &self,
        item: &PendingHostedPaymentRef,
        expire_session: bool,
    ) -> StoreResult<bool> {
        use crate::services::payment::method::CancelPendingIntentOutcome;

        let tenant_id = item.tenant_id;
        let transaction_id = item.transaction_id;
        let checkout_session_id = item.checkout_session_id;
        let invoice_id = item.invoice_id;
        let swept_connection_id = item.connection_id;
        let swept_intent_id = item.intent_id.clone();
        self.store
            .transaction(|conn| {
                async move {
                    // ── lock the anchor, dispatching on the tx's linkage ──
                    let session = match (checkout_session_id, invoice_id) {
                        (Some(session_id), _) => {
                            let session = CheckoutSessionRow::get_by_id_for_update(
                                conn, tenant_id, session_id,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                            if !matches!(
                                session.status,
                                diesel_models::enums::CheckoutSessionStatusEnum::Created
                                    | diesel_models::enums::CheckoutSessionStatusEnum::AwaitingPayment
                            ) {
                                // Completed/expired/cancelled since we looked.
                                return Ok(false);
                            }
                            Some(session_id)
                        }
                        (None, Some(inv_id)) => {
                            // Locks customer then invoice — serializes with
                            // initiation and the settlement pipeline.
                            InvoiceRow::select_for_update_by_id(conn, tenant_id, inv_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            None
                        }
                        (None, None) => {
                            // The listing filters these out already.
                            return Ok(false);
                        }
                    };

                    let row =
                        PaymentTransactionRow::get_by_id(conn, transaction_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    // The attempt re-initiated onto a NEWER intent since this
                    // item was listed: never close out on stale evidence.
                    if row.pending_provider_intent_id.as_deref() != Some(swept_intent_id.as_str())
                    {
                        log::info!(
                            "not closing out hosted payment transaction {transaction_id}: pending \
                             intent changed since sweep listed {swept_intent_id}"
                        );
                        return Ok(false);
                    }

                    // Money moved / a provider payment is bound — never cancel
                    // over it; completion/reconciliation owns this attempt.
                    if row.status == diesel_models::enums::PaymentStatusEnum::Settled
                        || row.status == diesel_models::enums::PaymentStatusEnum::Refunded
                        || (row.provider_transaction_id.is_some()
                            && matches!(
                                row.status,
                                diesel_models::enums::PaymentStatusEnum::Pending
                                    | diesel_models::enums::PaymentStatusEnum::Ready
                            ))
                    {
                        log::warn!(
                            "not closing out hosted payment transaction {transaction_id}: already \
                             progressed ({:?}, provider id {:?})",
                            row.status,
                            row.provider_transaction_id
                        );
                        return Ok(false);
                    }

                    // Kill the intent at the provider FIRST: a closed-out
                    // attempt must never leave a live hosted page that can
                    // still capture. Not-cancelable means completion owns it.
                    match self
                        .cancel_pending_hosted_intent(
                            conn,
                            &tenant_id,
                            &swept_connection_id,
                            &swept_intent_id,
                        )
                        .await?
                    {
                        CancelPendingIntentOutcome::Cancelled => {}
                        CancelPendingIntentOutcome::NotCancelable => {
                            log::warn!(
                                "not closing out hosted payment transaction {transaction_id}: \
                                 intent {swept_intent_id} has a payment underway; completion will \
                                 pick it up"
                            );
                            return Ok(false);
                        }
                    }

                    if matches!(
                        row.status,
                        diesel_models::enums::PaymentStatusEnum::Pending
                            | diesel_models::enums::PaymentStatusEnum::Ready
                    ) {
                        let cancelled = PaymentTransactionRow::cancel_if_awaiting(
                            conn,
                            tenant_id,
                            row.id,
                            if session.is_some() {
                                "checkout_abandoned"
                            } else {
                                "invoice_payment_abandoned"
                            },
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        if cancelled == 0 {
                            // The row progressed between our read and the
                            // guarded update: settlement won — do NOT close out.
                            log::warn!(
                                "not closing out hosted payment transaction {transaction_id}: \
                                 progressed concurrently; leaving it to completion"
                            );
                            return Ok(false);
                        }
                    } else {
                        // Declined attempt past the cutoff: the intent is now
                        // dead at the provider — stop watching it.
                        let cleared = PaymentTransactionRow::clear_pending_intent_if_matches(
                            conn,
                            tenant_id,
                            row.id,
                            &swept_intent_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        if cleared == 0 {
                            return Ok(false);
                        }
                    }

                    if let Some(session_id) = session {
                        if expire_session {
                            CheckoutSessionRow::mark_expired_single(conn, tenant_id, session_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            log::info!(
                                "expired abandoned hosted checkout session {session_id} \
                                 (transaction {transaction_id})"
                            );
                        } else {
                            CheckoutSessionRow::reopen_after_failed_payment(
                                conn, tenant_id, session_id,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                            log::info!(
                                "closed out dead hosted attempt (transaction {transaction_id}); \
                                 checkout session {session_id} left open for a retry"
                            );
                        }
                    } else {
                        log::info!(
                            "closed out abandoned hosted invoice payment attempt \
                             (transaction {transaction_id}, invoice {invoice_id:?})"
                        );
                    }
                    Ok(true)
                }
                .scope_boxed()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CustomerPaymentMethod, PaymentMethodTypeEnum};
    use common_domain::ids::{
        BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, TenantId,
    };

    fn method() -> CustomerPaymentMethod {
        let now = chrono::Utc::now().naive_utc();
        CustomerPaymentMethod {
            id: CustomerPaymentMethodId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            connection_id: CustomerConnectionId::new(),
            external_payment_method_id: "card_x".into(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            payment_method_type: PaymentMethodTypeEnum::Card,
            account_number_hint: None,
            card_brand: Some("visa".into()),
            card_last4: Some("4242".into()),
            card_exp_month: Some(12),
            card_exp_year: Some(2030),
            fingerprint: None,
        }
    }

    /// Invariants: a completed attempt is final regardless of age (never
    /// expired away); only the abandonment cutoff closes an attempt out; no
    /// branch initiates a charge.
    #[test]
    fn sweep_action_table() {
        let activated = HostedSetupOutcome::CheckoutActivated(method());
        // Even past the cutoff, a payment just before the sweep must converge
        // on the captured payment, never on an expiry.
        assert_eq!(
            sweep_action(&activated, false),
            HostedPaymentSweepOutcome::Completed
        );
        assert_eq!(
            sweep_action(&activated, true),
            HostedPaymentSweepOutcome::Completed
        );

        let invoice_settled = HostedSetupOutcome::InvoiceCharged(method());
        assert_eq!(
            sweep_action(&invoice_settled, false),
            HostedPaymentSweepOutcome::Completed
        );
        assert_eq!(
            sweep_action(&invoice_settled, true),
            HostedPaymentSweepOutcome::Completed
        );

        let declined = HostedSetupOutcome::PaymentFailed {
            payment_method: method(),
            code: Some("51".into()),
        };
        // Declined: left for a saved-card retry until the cutoff.
        assert_eq!(
            sweep_action(&declined, false),
            HostedPaymentSweepOutcome::Declined
        );
        assert_eq!(
            sweep_action(&declined, true),
            HostedPaymentSweepOutcome::CheckoutExpired
        );

        // No card / no payment on the intent yet: wait, then expire.
        assert_eq!(
            sweep_action(&HostedSetupOutcome::Processing, false),
            HostedPaymentSweepOutcome::StillPending
        );
        assert_eq!(
            sweep_action(&HostedSetupOutcome::Processing, true),
            HostedPaymentSweepOutcome::CheckoutExpired
        );
        let failed = |definitive| HostedSetupOutcome::SetupFailed { definitive };
        assert_eq!(
            sweep_action(&failed(false), false),
            HostedPaymentSweepOutcome::StillPending
        );
        assert_eq!(
            sweep_action(&failed(false), true),
            HostedPaymentSweepOutcome::CheckoutExpired
        );
        // Final at the provider (Mollie canceled/expired): closed out now, but the session only
        // expires after the cutoff.
        assert_eq!(
            sweep_action(&failed(true), false),
            HostedPaymentSweepOutcome::AttemptClosed
        );
        assert_eq!(
            sweep_action(&failed(true), true),
            HostedPaymentSweepOutcome::CheckoutExpired
        );

        assert_eq!(
            sweep_action(&HostedSetupOutcome::MethodSaved(method()), false),
            HostedPaymentSweepOutcome::StillPending
        );
        assert_eq!(
            sweep_action(&HostedSetupOutcome::MethodSaved(method()), true),
            HostedPaymentSweepOutcome::CheckoutExpired
        );

        // Captured-but-unreconciled money is NEVER expired away — expiring
        // would cancel the transaction under captured funds.
        let held = HostedSetupOutcome::HeldForReview {
            payment_method: method(),
        };
        assert_eq!(
            sweep_action(&held, false),
            HostedPaymentSweepOutcome::StillPending
        );
        assert_eq!(
            sweep_action(&held, true),
            HostedPaymentSweepOutcome::StillPending
        );
    }
}

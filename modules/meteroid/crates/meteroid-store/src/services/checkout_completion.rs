use crate::StoreResult;
use crate::domain::CheckoutSession;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::errors::StoreError;
use crate::repositories::purchase_links::checkout_accepts_coupon_codes_tx;
use crate::services::Services;
use crate::services::payment::process_payment::{ChargeAnswer, CheckoutChargeClaim};
use crate::store::PgConn;
use common_domain::ids::{
    CheckoutSessionId, CouponId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::DbResult;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::PaymentStatusEnum;
use diesel_models::errors::DatabaseError;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::sync::atomic::{AtomicBool, Ordering};

/// Result of charging a payment method directly (before invoice/subscription creation)
#[derive(Debug, Clone)]
pub struct DirectChargeResult {
    pub payment_intent: PaymentIntent,
    pub transaction_id: PaymentTransactionId,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: CustomerPaymentMethodId,
}

/// Facts about a synchronously-declined checkout charge, attached to the returned
/// error so `complete_checkout` can record the failed attempt once its transaction
/// (and the session's lock) has unwound — see [`Services::persist_declined_checkout_charge`].
#[derive(Debug, Clone)]
pub(crate) struct DeclinedCheckoutCharge {
    pub transaction_id: PaymentTransactionId,
    pub provider_transaction_id: Option<String>,
    pub error_message: String,
    pub processed_at: Option<chrono::NaiveDateTime>,
    pub status: PaymentStatusEnum,
}

/// What a checkout charges, found by its first run, which stops there: the charge is committed
/// and sent outside the checkout's transaction (a provider holding the money never outlives the
/// row that records it), then the checkout runs again with the answer.
#[derive(Debug, Clone)]
pub(crate) struct ChargeNeeded {
    pub payment_method_id: CustomerPaymentMethodId,
    pub amount: i64,
    pub currency: String,
    /// The coupons the charge is priced with, kept on the session for a deferred completion.
    pub coupons: Option<PaymentCoupons>,
}

#[derive(Debug, Clone)]
pub struct PaymentCoupons {
    pub code: Option<String>,
    pub ids: Vec<Option<CouponId>>,
}

/// How an invoice billed after payment is charged: in the caller's transaction, or as a run of a
/// checkout (see [`CheckoutCharge`]).
#[allow(clippy::large_enum_variant)]
pub enum InvoiceCheckoutCharge {
    InTransaction,
    Price(Option<PaymentCoupons>),
    Answered(DirectChargeResult),
}

/// How a run of a checkout meets its charge (see [`ChargeNeeded`]).
#[allow(clippy::large_enum_variant)]
pub(crate) enum CheckoutCharge {
    Price,
    /// `used` is set once the run recorded the answer.
    Answered {
        result: DirectChargeResult,
        used: AtomicBool,
    },
}

impl CheckoutCharge {
    pub(crate) fn answer(result: DirectChargeResult) -> Self {
        Self::Answered {
            result,
            used: AtomicBool::new(false),
        }
    }

    /// A run's pass that rolls back records nothing: the next starts over.
    pub(crate) fn reset_used(&self) {
        if let Self::Answered { used, .. } = self {
            used.store(false, Ordering::Relaxed);
        }
    }

    pub(crate) fn mark_used(&self) {
        if let Self::Answered { used, .. } = self {
            used.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn answered(&self) -> Option<&DirectChargeResult> {
        match self {
            Self::Price => None,
            Self::Answered { result, .. } => Some(result),
        }
    }

    pub(crate) fn used(&self) -> bool {
        match self {
            Self::Price => false,
            Self::Answered { used, .. } => used.load(Ordering::Relaxed),
        }
    }
}

pub(crate) fn charge_needed_error(needed: ChargeNeeded) -> Report<StoreError> {
    Report::new(StoreError::CheckoutError)
        .attach("The checkout stops at its charge")
        .attach_opaque(needed)
}

/// The answer is to the charge this run asks for, within the confirmation's one minor unit.
fn ensure_answer_of(
    answer: &DirectChargeResult,
    payment_method_id: CustomerPaymentMethodId,
    amount: i64,
    currency: &str,
) -> StoreResult<()> {
    if answer.payment_method_id != payment_method_id
        || answer.currency != currency
        || (answer.amount - amount).abs() > 1
    {
        return Err(Report::new(StoreError::CheckoutError).attach(format!(
            "The checkout now charges {amount} {currency}; its payment took {} {}",
            answer.amount, answer.currency
        )));
    }
    Ok(())
}

pub(crate) fn charge_needed(error: &Report<StoreError>) -> Option<ChargeNeeded> {
    error
        .frames()
        .find_map(|f| f.downcast_ref::<ChargeNeeded>())
        .cloned()
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum CheckoutChargeSent {
    /// A charge of the session already in flight, or this one with no answer back yet.
    InFlight(PaymentTransaction, Option<PaymentNextAction>),
    Answered(DirectChargeResult),
}

/// Declined charges a checkout session tolerates before it is cancelled (card testing).
pub const MAX_DECLINED_CHECKOUT_ATTEMPTS: i64 = 5;

/// Marks a charge refused because its payment method can't be used: unknown, archived, or
/// held by another customer. Retrying the same charge can't succeed.
#[derive(Debug, Clone, Copy)]
struct PaymentMethodUnusable;

/// Unknown, foreign and archived methods get the same answer, so ids can't be probed.
pub(crate) fn payment_method_not_found() -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(
        "Payment method not found".to_string(),
    ))
    .attach_opaque(PaymentMethodUnusable)
}

/// True when the charge was refused because of its payment method, not because of a failure
/// that a retry could get past.
pub fn is_payment_method_unusable(error: &Report<StoreError>) -> bool {
    error
        .frames()
        .any(|frame| frame.downcast_ref::<PaymentMethodUnusable>().is_some())
}

/// `None` only when no such method exists. Any other database error is returned as is, so a
/// transient failure is retried instead of being taken for an unusable method.
pub(crate) async fn find_payment_method(
    conn: &mut PgConn,
    tenant_id: TenantId,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<Option<CustomerPaymentMethodRow>> {
    missing_as_none(CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id).await)
}

fn missing_as_none<T>(result: DbResult<T>) -> StoreResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if matches!(err.error.current_context(), DatabaseError::NotFound) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

impl Services {
    /// A customer is only charged on a live payment method of their own.
    pub(crate) async fn ensure_payment_method_of_customer(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> StoreResult<()> {
        match find_payment_method(conn, tenant_id, payment_method_id).await? {
            Some(m) if m.customer_id == customer_id && m.archived_at.is_none() => Ok(()),
            _ => Err(payment_method_not_found()),
        }
    }

    pub(crate) async fn ensure_connection_of_customer(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        connection_id: CustomerConnectionId,
    ) -> StoreResult<()> {
        let connection = CustomerConnectionRow::get_by_id(conn, &tenant_id, &connection_id)
            .await
            .ok();
        match connection {
            Some(c) if c.customer_id == customer_id => Ok(()),
            _ => Err(Report::new(StoreError::InvalidArgument(
                "Payment connection not found".to_string(),
            ))),
        }
    }

    /// Refuses a buyer-entered coupon code on sessions from purchase links that don't take
    /// them. A code the merchant set on the session itself is always honored.
    pub(crate) async fn ensure_coupon_code_accepted(
        &self,
        conn: &mut PgConn,
        session: &CheckoutSession,
        coupon_code: Option<&str>,
    ) -> StoreResult<()> {
        let Some(code) = coupon_code.map(str::trim).filter(|c| !c.is_empty()) else {
            return Ok(());
        };
        if session.coupon_code.as_deref() == Some(code) {
            return Ok(());
        }
        if checkout_accepts_coupon_codes_tx(conn, session.tenant_id, session.id).await? {
            Ok(())
        } else {
            Err(Report::new(StoreError::InvalidArgument(
                "Coupon codes are not accepted for this purchase".to_string(),
            )))
        }
    }

    /// Cancels the session once it collected [`MAX_DECLINED_CHECKOUT_ATTEMPTS`] declines.
    /// Call after [`Self::persist_declined_checkout_charge`], with the same error.
    pub(crate) async fn cancel_checkout_after_repeated_declines(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        error: &Report<StoreError>,
    ) {
        if !error
            .frames()
            .any(|f| f.downcast_ref::<DeclinedCheckoutCharge>().is_some())
        {
            return;
        }

        let result = async {
            let mut conn = self.store.get_conn().await?;
            let declined =
                diesel_models::payments::PaymentTransactionRow::count_failed_by_checkout_session_id(
                    &mut conn,
                    checkout_session_id,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            if declined < MAX_DECLINED_CHECKOUT_ATTEMPTS {
                return Ok(false);
            }
            CheckoutSessionRow::mark_cancelled(&mut conn, tenant_id, checkout_session_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Ok::<_, Report<StoreError>>(true)
        }
        .await;

        match result {
            Ok(true) => log::warn!(
                "Checkout session {checkout_session_id} cancelled after \
                 {MAX_DECLINED_CHECKOUT_ATTEMPTS} declined payments"
            ),
            Ok(false) => {}
            Err(e) => log::error!(
                "Could not check declined attempts of checkout session {checkout_session_id}: {e:?}"
            ),
        }
    }

    /// The charge a run of a checkout makes: on the first run none, the run stops there with
    /// [`ChargeNeeded`] and rolls back; on the second, the provider's answer to that charge.
    pub(crate) fn take_checkout_charge(
        &self,
        charge: &CheckoutCharge,
        payment_method_id: CustomerPaymentMethodId,
        amount: i64,
        currency: String,
        coupons: Option<PaymentCoupons>,
    ) -> StoreResult<DirectChargeResult> {
        match charge {
            CheckoutCharge::Price => Err(charge_needed_error(ChargeNeeded {
                payment_method_id,
                amount,
                currency,
                coupons,
            })),
            CheckoutCharge::Answered { result, used } => {
                ensure_answer_of(result, payment_method_id, amount, &currency)?;
                used.store(true, Ordering::Relaxed);
                Ok(result.clone())
            }
        }
    }

    /// Records a checkout charge's answer on its committed row, linked to the invoice it pays,
    /// through the same pipeline as an invoice charge answered in its transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn record_checkout_charge_on_invoice_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount_due: i64,
        currency: &str,
        payment_method_id: CustomerPaymentMethodId,
        answer: &DirectChargeResult,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        use crate::repositories::payment_transactions::PaymentTransactionInterface;

        ensure_answer_of(answer, payment_method_id, amount_due, currency)?;
        let row = PaymentTransactionRowPatch {
            id: answer.transaction_id,
            invoice_id: Some(Some(invoice_id)),
            ..Default::default()
        }
        .patch_unsent(conn, tenant_id, answer.transaction_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .ok_or_else(|| {
            Report::new(StoreError::CheckoutError).attach(format!(
                "Checkout charge {} is no longer awaiting its answer",
                answer.transaction_id
            ))
        })?;
        let transaction = self
            .store
            .consolidate_intent_and_transaction_tx(
                conn,
                &crate::domain::entity_activity::Actor::System,
                row.into(),
                answer.payment_intent.clone(),
            )
            .await?;
        Ok((transaction, answer.payment_intent.next_action.clone()))
    }

    /// Commits a checkout's charge, then sends it with no connection held. A decline errors with
    /// [`DeclinedCheckoutCharge`]; an answer that never came back leaves the charge in flight,
    /// for the reconciliation worker to re-send under the same key.
    pub(crate) async fn send_checkout_charge(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        needed: &ChargeNeeded,
    ) -> StoreResult<CheckoutChargeSent> {
        let charge = match self
            .claim_checkout_charge(tenant_id, checkout_session_id, needed)
            .await?
        {
            CheckoutChargeClaim::InFlight(transaction, next_action) => {
                return Ok(CheckoutChargeSent::InFlight(transaction, next_action));
            }
            CheckoutChargeClaim::Claimed(charge) => charge,
        };
        let row = charge.transaction().clone();

        match self.send_invoice_charge(&charge).await {
            ChargeAnswer::Outcome { intent, amount } => direct_charge_result(
                *intent,
                row.id,
                amount,
                row.currency.clone(),
                needed.payment_method_id,
            )
            .map(CheckoutChargeSent::Answered),
            ChargeAnswer::Refused(error) => {
                // Nothing was created at the provider: the attempt is undone.
                if let Err(e) = self
                    .undo_refused_checkout_charge(tenant_id, checkout_session_id, row.id)
                    .await
                {
                    log::error!("Could not undo refused checkout charge {}: {e:?}", row.id);
                }
                Err(error)
            }
            // A fresh key is never past its replay window: an unconfirmed answer is no answer.
            ChargeAnswer::Unknown(error) => {
                log::warn!(
                    "Checkout charge {} for session {checkout_session_id} got no answer; left in \
                     flight for reconciliation: {error:?}",
                    row.id
                );
                Ok(CheckoutChargeSent::InFlight(row.into(), None))
            }
            ChargeAnswer::Unconfirmed => Ok(CheckoutChargeSent::InFlight(row.into(), None)),
        }
    }

    async fn undo_refused_checkout_charge(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        transaction_id: PaymentTransactionId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    PaymentTransactionRow::delete_unsent_charge(conn, transaction_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    CheckoutSessionRow::reopen_after_failed_payment(
                        conn,
                        tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Records the answer of a checkout charge that the checkout's second run did not record (it
    /// failed, or no longer charged), through the settlement pipeline: a settlement then
    /// completes the checkout from its session, as an asynchronous payment does.
    pub(crate) async fn settle_unrecorded_checkout_charge(
        &self,
        tenant_id: TenantId,
        result: &DirectChargeResult,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        let row = {
            let mut conn = self.store.get_conn().await?;
            PaymentTransactionRow::get_by_id(&mut conn, result.transaction_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        self.settle_charge_row(
            &row,
            ChargeAnswer::Outcome {
                intent: Box::new(result.payment_intent.clone()),
                amount: result.amount,
            },
            false,
        )
        .await
    }

    /// Records a checkout charge's answer on its committed row, linked to the invoice it paid.
    pub(crate) async fn create_transaction_for_direct_charge(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        charge_result: &DirectChargeResult,
        // How a settled charge's method becomes the default.
        adoption: crate::services::DefaultMethodAdoption,
    ) -> StoreResult<PaymentTransaction> {
        let status: PaymentStatusEnum = charge_result.payment_intent.status.clone().into();
        let settled = status == PaymentStatusEnum::Settled;
        let inserted =
            record_checkout_answer(conn, tenant_id, Some(invoice_id), charge_result).await?;

        // A checkout charge is paid by the customer on-session and records no settlement event:
        // its method is adopted here.
        if settled {
            let method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &charge_result.payment_method_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            self.adopt_default_payment_method_tx(
                conn,
                &crate::domain::entity_activity::Actor::Customer {
                    id: method.customer_id,
                },
                tenant_id,
                method.customer_id,
                charge_result.payment_method_id,
                adoption,
            )
            .await?;
        }

        Ok(inserted.into())
    }

    /// Records a checkout charge's answer on its committed row (no invoice yet).
    pub(crate) async fn create_transaction_for_checkout(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        charge_result: &DirectChargeResult,
    ) -> StoreResult<PaymentTransaction> {
        Ok(record_checkout_answer(conn, tenant_id, None, charge_result)
            .await?
            .into())
    }

    /// Returns the existing Pending/Ready/Settled transaction for a checkout session,
    /// if any, alongside its persisted next_action. Backs the checkout-completion
    /// idempotency guard: a re-completion while a charge is in flight or settled but
    /// not yet fulfilled returns this instead of issuing a second charge.
    ///
    /// The transient `client_secret` on `UseSdk` is `#[serde(skip)]`'d, so a
    /// re-fetched next_action carries none — the portal re-fetches it from the
    /// provider to resume 3DS. The domain conversion ghosts next_action to None,
    /// so it is re-hydrated from the row's JSONB here.
    pub(crate) async fn find_active_checkout_transaction(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> StoreResult<Option<(PaymentTransaction, Option<PaymentNextAction>)>> {
        let Some(row) =
            diesel_models::payments::PaymentTransactionRow::get_active_by_checkout_session_id(
                conn,
                checkout_session_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };

        let next_action: Option<PaymentNextAction> = row
            .next_action
            .clone()
            .and_then(|v| serde_json::from_value(v).ok());

        let mut transaction: PaymentTransaction = row.into();
        transaction.next_action = next_action.clone();

        Ok(Some((transaction, next_action)))
    }

    /// Records a declined checkout charge (from the error's [`DeclinedCheckoutCharge`]
    /// attachment) on its committed row, and reopens the session for another attempt. Call ONLY
    /// after the checkout tx has returned.
    pub(crate) async fn persist_declined_checkout_charge(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        error: &Report<StoreError>,
    ) {
        let Some(declined) = error
            .frames()
            .find_map(|f| f.downcast_ref::<DeclinedCheckoutCharge>())
        else {
            return;
        };

        let answer = PaymentTransactionRowPatch {
            id: declined.transaction_id,
            provider_transaction_id: Some(declined.provider_transaction_id.clone()),
            status: Some(declined.status.clone()),
            error_type: Some(Some(declined.error_message.clone())),
            processed_at: Some(declined.processed_at),
            ..Default::default()
        };

        // A webhook may have recorded the decline first; either way the session reopens.
        let answer = &answer;
        let recorded = self
            .store
            .transaction(|conn| {
                async move {
                    answer
                        .patch_unsent(conn, tenant_id, declined.transaction_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    CheckoutSessionRow::reopen_after_failed_payment(
                        conn,
                        tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    Ok::<_, Report<StoreError>>(())
                }
                .scope_boxed()
            })
            .await;
        if let Err(e) = recorded {
            log::error!(
                "Failed to persist declined checkout charge {}: {e:?}",
                declined.transaction_id
            );
        }
    }

    pub(crate) async fn link_transaction_to_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        transaction_id: PaymentTransactionId,
        invoice_id: InvoiceId,
    ) -> StoreResult<PaymentTransaction> {
        let updated = PaymentTransactionRowPatch {
            invoice_id: Some(Some(invoice_id)),
            ..Default::default()
        }
        .patch(conn, tenant_id, transaction_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.into())
    }
}

/// The answer on the committed row; the checkout's lock keeps any other writer off it.
async fn record_checkout_answer(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: Option<InvoiceId>,
    charge_result: &DirectChargeResult,
) -> StoreResult<PaymentTransactionRow> {
    // Persist the 3DS/redirect action so a re-completion's idempotency guard
    // (find_active_checkout_transaction) can re-hydrate it. The client_secret
    // is #[serde(skip)]'d, so only the non-secret identity is stored; the
    // portal re-fetches the secret from the provider to resume.
    let next_action = charge_result
        .payment_intent
        .next_action
        .as_ref()
        .and_then(|a| serde_json::to_value(a).ok());
    PaymentTransactionRowPatch {
        id: charge_result.transaction_id,
        invoice_id: invoice_id.map(Some),
        provider_transaction_id: Some(Some(charge_result.payment_intent.external_id.clone())),
        amount: Some(charge_result.amount),
        status: Some(charge_result.payment_intent.status.clone().into()),
        error_type: Some(charge_result.payment_intent.last_payment_error.clone()),
        processed_at: Some(charge_result.payment_intent.processed_at),
        next_action: Some(next_action),
        ..Default::default()
    }
    .patch_unsent(conn, tenant_id, charge_result.transaction_id)
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .ok_or_else(|| {
        Report::new(StoreError::CheckoutError).attach(format!(
            "Checkout charge {} is no longer awaiting its answer",
            charge_result.transaction_id
        ))
    })
}

/// A captured or pending answer as the checkout records it; a decline errors with the facts
/// [`Services::persist_declined_checkout_charge`] records.
fn direct_charge_result(
    payment_intent: PaymentIntent,
    transaction_id: PaymentTransactionId,
    amount: i64,
    currency: String,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<DirectChargeResult> {
    match payment_intent.status {
        crate::domain::PaymentStatusEnum::Settled | crate::domain::PaymentStatusEnum::Pending => {
            Ok(DirectChargeResult {
                payment_intent,
                transaction_id,
                amount,
                currency,
                payment_method_id,
            })
        }
        status @ (crate::domain::PaymentStatusEnum::Failed
        | crate::domain::PaymentStatusEnum::Cancelled) => {
            let cancelled = status == crate::domain::PaymentStatusEnum::Cancelled;
            let error_message = payment_intent
                .last_payment_error
                .clone()
                .unwrap_or_else(|| {
                    if cancelled {
                        "Payment was cancelled"
                    } else {
                        "Payment failed"
                    }
                    .to_string()
                });
            // Decline facts travel on the error; recorded after the tx unwinds.
            Err(
                Report::new(StoreError::PaymentError(error_message.clone())).attach_opaque(
                    DeclinedCheckoutCharge {
                        transaction_id,
                        provider_transaction_id: Some(payment_intent.external_id.clone())
                            .filter(|id| !id.is_empty()),
                        error_message,
                        processed_at: payment_intent.processed_at,
                        status: if cancelled {
                            PaymentStatusEnum::Cancelled
                        } else {
                            PaymentStatusEnum::Failed
                        },
                    },
                ),
            )
        }
        crate::domain::PaymentStatusEnum::Ready | crate::domain::PaymentStatusEnum::Refunded => {
            // A fresh charge is never Ready or already-Refunded; treat as unexpected.
            Err(Report::new(StoreError::PaymentError(
                "Payment intent in unexpected state".to_string(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_models::errors::DatabaseErrorContainer;

    #[test]
    fn only_a_missing_method_is_unusable() {
        let missing: DbResult<()> = Err(DatabaseErrorContainer::from(DatabaseError::NotFound));
        assert!(missing_as_none(missing).unwrap().is_none());

        let outage: DbResult<()> = Err(DatabaseErrorContainer::from(
            DatabaseError::DatabaseConnectionError,
        ));
        let err = missing_as_none(outage).unwrap_err();
        assert!(!is_payment_method_unusable(&err));
        assert!(is_payment_method_unusable(&payment_method_not_found()));
    }
}

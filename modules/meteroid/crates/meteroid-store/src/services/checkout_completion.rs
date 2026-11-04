use crate::StoreResult;
use crate::domain::CheckoutSession;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::errors::StoreError;
use crate::repositories::purchase_links::checkout_accepts_coupon_codes_tx;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customer_connection::{CustomerConnectionDetailsRow, CustomerConnectionRow};
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::{PaymentTransactionRowNew, PaymentTransactionRowPatch};
use error_stack::{Report, ResultExt};

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
    pub payment_method_id: CustomerPaymentMethodId,
    pub amount: i64,
    pub currency: String,
    pub error_message: String,
    pub processed_at: Option<chrono::NaiveDateTime>,
    pub status: PaymentStatusEnum,
}

/// Declined charges a checkout session tolerates before it is cancelled (card testing).
pub const MAX_DECLINED_CHECKOUT_ATTEMPTS: i64 = 5;

impl Services {
    /// A checkout only charges methods held by its own customer. Unknown and foreign
    /// methods get the same answer, so ids can't be probed.
    pub(crate) async fn ensure_payment_method_of_customer(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> StoreResult<()> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id)
            .await
            .ok();
        match method {
            Some(m) if m.customer_id == customer_id && m.archived_at.is_none() => Ok(()),
            _ => Err(Report::new(StoreError::InvalidArgument(
                "Payment method not found".to_string(),
            ))),
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

    /// Charges a payment method directly without an existing invoice.
    /// This is used in the self-serve checkout flow to charge the customer
    pub(crate) async fn charge_payment_method_directly(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_method_id: CustomerPaymentMethodId,
        amount: i64,
        currency: String,
    ) -> StoreResult<DirectChargeResult> {
        use crate::adapters::payment::bridge::payment_intent_from_outcome;
        use crate::adapters::payment::initialize_payment_connector;
        use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey};
        use crate::domain::connectors::Connector;

        if amount <= 0 {
            return Err(Report::new(StoreError::InvalidArgument(
                "Amount must be positive".to_string(),
            )));
        }

        let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, &tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        connector.ensure_active_locked(conn).await?;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let transaction_id = PaymentTransactionId::new();

        let request = ChargeRequest {
            transaction_id,
            customer_external_id: &connection.external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor: amount,
            currency: &currency,
            idempotency_key: IdempotencyKey::new(format!("charge:{}", transaction_id.as_base62())),
            // Checkout is always customer-present, so 3DS can be completed inline.
            on_session: true,
            descriptor: None,
            webhook_url: crate::adapters::payment::model::connector_webhook_url(
                self.store.settings.webhook_base_url(),
                &connector,
            ),
        };

        let outcome = tokio::time::timeout(
            crate::services::payment::process_payment::PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.charge_off_session(request),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?;
        self.store
            .note_rejected_credentials(&connector, &outcome)
            .await;
        let outcome = outcome.change_context_lazy(|| StoreError::PaymentProviderError)?;

        let payment_intent = payment_intent_from_outcome(
            outcome,
            transaction_id,
            tenant_id,
            amount,
            currency.clone(),
        );

        match payment_intent.status {
            crate::domain::PaymentStatusEnum::Settled
            | crate::domain::PaymentStatusEnum::Pending => Ok(DirectChargeResult {
                payment_intent,
                transaction_id,
                amount,
                currency,
                payment_method_id,
            }),
            status @ (crate::domain::PaymentStatusEnum::Failed
            | crate::domain::PaymentStatusEnum::Cancelled) => {
                let cancelled = status == crate::domain::PaymentStatusEnum::Cancelled;
                let error_message =
                    payment_intent
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
                            payment_method_id,
                            amount,
                            currency: currency.clone(),
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
            crate::domain::PaymentStatusEnum::Ready
            | crate::domain::PaymentStatusEnum::Refunded => {
                // A fresh charge is never Ready or already-Refunded; treat as unexpected.
                Err(Report::new(StoreError::PaymentError(
                    "Payment intent in unexpected state".to_string(),
                )))
            }
        }
    }

    /// Creates a transaction record for a direct charge result and links it to an invoice.
    pub(crate) async fn create_transaction_for_direct_charge(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        charge_result: &DirectChargeResult,
        pending_plan_version_id: Option<common_domain::ids::PlanVersionId>,
        // How a settled charge's method becomes the default.
        adoption: crate::services::DefaultMethodAdoption,
        checkout_session_id: Option<CheckoutSessionId>,
    ) -> StoreResult<PaymentTransaction> {
        let status: PaymentStatusEnum = charge_result.payment_intent.status.clone().into();
        let settled = status == PaymentStatusEnum::Settled;

        let transaction = PaymentTransactionRowNew {
            id: charge_result.transaction_id,
            tenant_id,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some(charge_result.payment_intent.external_id.clone()),
            amount: charge_result.amount,
            currency: charge_result.currency.clone(),
            payment_method_id: Some(charge_result.payment_method_id),
            status,
            payment_type: PaymentTypeEnum::Payment,
            error_type: charge_result.payment_intent.last_payment_error.clone(),
            processed_at: charge_result.payment_intent.processed_at,
            checkout_session_id,
            pending_plan_version_id,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        };

        let inserted = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

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

    /// Creates a transaction record for a checkout payment (no invoice yet).
    pub(crate) async fn create_transaction_for_checkout(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        charge_result: &DirectChargeResult,
    ) -> StoreResult<PaymentTransaction> {
        let status: PaymentStatusEnum = charge_result.payment_intent.status.clone().into();

        // Persist the 3DS/redirect action so a re-completion's idempotency guard
        // (find_active_checkout_transaction) can re-hydrate it. The client_secret
        // is #[serde(skip)]'d, so only the non-secret identity is stored; the
        // portal re-fetches the secret from the provider to resume.
        let next_action = charge_result
            .payment_intent
            .next_action
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok());

        let transaction = PaymentTransactionRowNew {
            id: charge_result.transaction_id,
            tenant_id,
            invoice_id: None,
            provider_transaction_id: Some(charge_result.payment_intent.external_id.clone()),
            amount: charge_result.amount,
            currency: charge_result.currency.clone(),
            payment_method_id: Some(charge_result.payment_method_id),
            status,
            payment_type: PaymentTypeEnum::Payment,
            error_type: charge_result.payment_intent.last_payment_error.clone(),
            processed_at: charge_result.payment_intent.processed_at,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
            next_action,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        };

        let inserted = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted.into())
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

    /// Persist a declined checkout charge (from the error's [`DeclinedCheckoutCharge`]
    /// attachment) as a committed `Failed` row. Call ONLY after the checkout tx has
    /// returned: the failed attempt is kept whatever the checkout's transaction does.
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

        let row = PaymentTransactionRowNew {
            id: declined.transaction_id,
            tenant_id,
            invoice_id: None,
            provider_transaction_id: declined.provider_transaction_id.clone(),
            amount: declined.amount,
            currency: declined.currency.clone(),
            payment_method_id: Some(declined.payment_method_id),
            status: declined.status.clone(),
            payment_type: PaymentTypeEnum::Payment,
            error_type: Some(declined.error_message.clone()),
            processed_at: declined.processed_at,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        };

        match self.store.get_conn().await {
            Ok(mut conn) => {
                if let Err(e) = row.insert(&mut conn).await {
                    log::error!(
                        "Failed to persist declined checkout charge {}: {e:?}",
                        declined.transaction_id
                    );
                }
            }
            Err(e) => log::error!(
                "Could not acquire a connection to persist declined checkout charge {}: {e:?}",
                declined.transaction_id
            ),
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

use crate::StoreResult;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::{Invoice, InvoicePaymentStatus, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::outbox::OutboxInterface;
use crate::services::Services;
use crate::services::subscriptions::{PaymentActivationParams, PaymentMethodInfo};
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, Utc};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{CycleActionEnum, SubscriptionActivationConditionEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

impl Services {
    pub async fn on_payment_transaction_settled(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        match (event.invoice_id, event.checkout_session_id) {
            (Some(invoice_id), _) => self.on_invoice_payment_settled(event, invoice_id).await,
            (None, Some(checkout_session_id)) => {
                self.on_checkout_payment_settled(event, checkout_session_id)
                    .await
            }
            (None, None) => {
                log::warn!(
                    "Payment transaction {} has neither invoice_id nor checkout_session_id",
                    event.payment_transaction_id
                );
                Err(Report::new(StoreError::InvalidArgument(
                    "Payment transaction must have either invoice_id or checkout_session_id"
                        .to_string(),
                )))
            }
        }
    }

    /// Handle payment settlement for a standard invoice payment
    async fn on_invoice_payment_settled(
        &self,
        event: PaymentTransactionEvent,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let invoice =
                        InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let subscription_id = invoice.invoice.subscription_id;

                    let should_finalize =
                        invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft;

                    // if the invoice is not finalized nor void, finalize it (no line data refresh)
                    if should_finalize {
                        self.finalize_invoice_tx(
                            conn,
                            invoice.invoice.id,
                            invoice.invoice.tenant_id,
                            false,
                            &None,
                        )
                        .await?;
                    }

                    // update the invoice amount due to amount_due - transaction.amount
                    let res = InvoiceRow::apply_transaction(
                        conn,
                        invoice_id,
                        event.tenant_id,
                        event.amount,
                    )
                    .await?;

                    let completed = res.amount_due == 0;

                    if completed {
                        let invoice: Invoice = res.try_into()?;

                        if invoice.payment_status != InvoicePaymentStatus::Paid {
                            InvoiceRow::apply_payment_status(
                                conn,
                                invoice_id,
                                event.tenant_id,
                                diesel_models::enums::InvoicePaymentStatus::Paid,
                                event.processed_at,
                            )
                            .await?;

                            self.store
                                .insert_outbox_event_tx(
                                    conn,
                                    OutboxEvent::invoice_paid((&invoice).into()),
                                )
                                .await?;
                        }

                        // todo should this be on_invoice_paid?
                        if let Some(subscription_id) = subscription_id.as_ref() {
                            let subscription = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                *subscription_id,
                            )
                            .await?;

                            // Activate subscription if pending checkout
                            let should_activate = subscription.subscription.activated_at.is_none()
                                && subscription.subscription.activation_condition
                                    == SubscriptionActivationConditionEnum::OnCheckout;

                            // Transition TrialExpired to Active when invoice is paid
                            let should_activate_from_trial_expired =
                                subscription.subscription.status
                                    == diesel_models::enums::SubscriptionStatusEnum::TrialExpired;

                            if should_activate_from_trial_expired {
                                // Trial expired subscription paid - transition to Active
                                // Use current_period_start (trial end date), not billing_start_date
                                let period_start = subscription.subscription.current_period_start;

                                let range = calculate_advance_period_range(
                                    period_start,
                                    subscription.subscription.billing_day_anchor as u32,
                                    true, // Align to billing_day_anchor (prorates for fixed day, full period for anniversary)
                                    &subscription.subscription.period.into(),
                                );

                                SubscriptionRow::transition_trial_expired_to_active(
                                    conn,
                                    subscription_id,
                                    &event.tenant_id,
                                    range.start,
                                    Some(range.end),
                                    Some(CycleActionEnum::RenewSubscription),
                                    Some(0),
                                )
                                .await?;
                            } else if should_activate {
                                let billing_start_date = subscription
                                    .subscription
                                    .billing_start_date
                                    .unwrap_or(chrono::Utc::now().date_naive());

                                let current_period_start;
                                let current_period_end;
                                let next_cycle_action;
                                let mut cycle_index = None;
                                let status;

                                if subscription.subscription.trial_duration.is_some() {
                                    status = SubscriptionStatusEnum::TrialActive;
                                    current_period_start = billing_start_date;
                                    current_period_end = Some(
                                        current_period_start
                                            + chrono::Duration::days(i64::from(
                                                subscription.subscription.trial_duration.unwrap(),
                                            )),
                                    );
                                    next_cycle_action = Some(CycleActionEnum::EndTrial);
                                } else {
                                    let range = calculate_advance_period_range(
                                        billing_start_date,
                                        subscription.subscription.billing_day_anchor as u32,
                                        true,
                                        &subscription.subscription.period.into(),
                                    );

                                    status = SubscriptionStatusEnum::Active;
                                    cycle_index = Some(0);
                                    current_period_start = range.start;
                                    current_period_end = Some(range.end);
                                    next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                                }

                                // TODO send a subscription_activated event
                                SubscriptionRow::activate_subscription(
                                    conn,
                                    subscription_id,
                                    &event.tenant_id,
                                    current_period_start,
                                    current_period_end,
                                    next_cycle_action,
                                    cycle_index,
                                    status.into(),
                                )
                                .await?;
                            }
                        }
                    } else {
                        InvoiceRow::apply_payment_status(
                            conn,
                            invoice_id,
                            event.tenant_id,
                            diesel_models::enums::InvoicePaymentStatus::PartiallyPaid,
                            event.processed_at,
                        )
                        .await?;
                    }

                    // TODO payment receipt

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }

    /// Handle payment settlement for a checkout session (async payment that was pending)
    ///
    /// For SelfServe checkouts: Creates the subscription, bills it, and marks checkout complete.
    /// For SubscriptionActivation checkouts: Bills the existing subscription and marks checkout complete.
    async fn on_checkout_payment_settled(
        &self,
        event: PaymentTransactionEvent,
        checkout_session_id: common_domain::ids::CheckoutSessionId,
    ) -> StoreResult<()> {
        use crate::domain::checkout_sessions::CheckoutType;
        use crate::services::InvoiceBillingMode;
        use crate::services::checkout_completion::DirectChargeResult;
        use diesel_models::customers::CustomerRow;

        self.store
            .transaction(|conn| {
                async move {
                    let session: crate::domain::checkout_sessions::CheckoutSession =
                        CheckoutSessionRow::get_by_id(conn, event.tenant_id, checkout_session_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .into();

                    if session.is_completed() {
                        log::warn!(
                            "Checkout session {} already completed, ignoring duplicate webhook",
                            checkout_session_id
                        );
                        return Ok(());
                    }

                    if session.is_expired() {
                        log::warn!(
                            "Payment settled for expired checkout session {}",
                            checkout_session_id
                        );
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Checkout session has expired".to_string(),
                        )));
                    }

                    let charge_result = DirectChargeResult {
                        payment_intent: crate::domain::payment_transactions::PaymentIntent {
                            external_id: event.provider_transaction_id.clone().unwrap_or_default(),
                            transaction_id: event.payment_transaction_id,
                            tenant_id: event.tenant_id,
                            amount_requested: event.amount,
                            amount_received: Some(event.amount),
                            currency: event.currency.clone(),
                            next_action: None,
                            status: crate::domain::PaymentStatusEnum::Settled,
                            last_payment_error: None,
                            processed_at: event.processed_at,
                        },
                        transaction_id: event.payment_transaction_id,
                        amount: event.amount,
                        currency: event.currency.clone(),
                        payment_method_id: event.payment_method_id.ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(
                                "Payment method ID required".to_string(),
                            ))
                        })?,
                    };

                    let subscription_id = match session.checkout_type {
                        CheckoutType::SelfServe => {
                            // Create subscription now that payment is confirmed
                            let coupon_ids = self
                                .resolve_coupon_ids_for_checkout_tx(
                                    conn,
                                    event.tenant_id,
                                    &session,
                                    None,
                                )
                                .await?;

                            let start_date = session
                                .billing_start_date
                                .unwrap_or_else(|| Utc::now().date_naive());

                            let create_subscription =
                                session.to_create_subscription(start_date, coupon_ids);

                            let context = self
                                .gather_subscription_context(
                                    conn,
                                    std::slice::from_ref(&create_subscription),
                                    event.tenant_id,
                                    &self.store.settings.crypt_key,
                                )
                                .await?;

                            let detailed_subscriptions =
                                self.build_subscription_details(&[create_subscription], &context)?;

                            let detailed_sub = detailed_subscriptions.into_iter().next().ok_or(
                                Report::new(StoreError::InsertError)
                                    .attach("No subscription details built"),
                            )?;

                            // Skip provider setup since payment already happened
                            let payment_result = crate::services::PaymentSetupResult {
                                card_connection_id: None,
                                direct_debit_connection_id: None,
                                checkout: false,
                                payment_method: Some(charge_result.payment_method_id),
                                bank: None,
                            };

                            let processed = self.process_subscription(
                                &detailed_sub,
                                &payment_result,
                                &context,
                                event.tenant_id,
                                None,
                            )?;

                            // Skip coupon validation since coupons were already validated before
                            // charging. The customer already paid the discounted price.
                            let created_subscriptions = self
                                .persist_subscriptions_skip_coupon_validation(
                                    conn,
                                    &[processed],
                                    event.tenant_id,
                                    &self.store.settings.jwt_secret,
                                    &self.store.settings.public_url,
                                )
                                .await?;

                            let created_subscription =
                                created_subscriptions.into_iter().next().ok_or(
                                    Report::new(StoreError::InsertError)
                                        .attach("No subscription created"),
                                )?;

                            self.bill_subscription_tx(
                                conn,
                                event.tenant_id,
                                created_subscription.id,
                                InvoiceBillingMode::AlreadyPaid {
                                    charge_result: charge_result.clone(),
                                    existing_transaction_id: Some(event.payment_transaction_id),
                                },
                            )
                            .await?
                            .ok_or(
                                Report::new(StoreError::InsertError)
                                    .attach("Failed to create invoice for subscription"),
                            )?;

                            // Activate the subscription now that payment is confirmed
                            let billing_start_date = session
                                .billing_start_date
                                .unwrap_or_else(|| Utc::now().date_naive());

                            // Get subscription to determine the period
                            let subscription = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                created_subscription.id,
                            )
                            .await?;

                            let billing_day_anchor = session
                                .billing_day_anchor
                                .map(|a| a as u32)
                                .unwrap_or_else(|| billing_start_date.day());

                            let payment_method = CustomerPaymentMethodRow::get_by_id(
                                conn,
                                &event.tenant_id,
                                &charge_result.payment_method_id,
                            )
                            .await
                            .map_err(|e| StoreError::DatabaseError(e.error))?;

                            self.activate_subscription_after_payment(
                                conn,
                                &created_subscription.id,
                                &event.tenant_id,
                                PaymentActivationParams {
                                    billing_start_date,
                                    trial_duration: session.trial_duration_days,
                                    billing_day_anchor,
                                    period: subscription.subscription.period.into(),
                                    payment_method: Some(PaymentMethodInfo {
                                        id: charge_result.payment_method_id,
                                        method_type: payment_method.payment_method_type,
                                    }),
                                },
                            )
                            .await?;

                            created_subscription.id
                        }
                        CheckoutType::SubscriptionActivation => {
                            let subscription_id = session.subscription_id.ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "SubscriptionActivation checkout missing subscription_id"
                                        .to_string(),
                                ))
                            })?;

                            let subscription = self
                                .store
                                .get_subscription_details_with_conn(
                                    conn,
                                    event.tenant_id,
                                    subscription_id,
                                )
                                .await?;

                            let customer: crate::domain::Customer = CustomerRow::find_by_id(
                                conn,
                                &subscription.subscription.customer_id,
                                &event.tenant_id,
                            )
                            .await
                            .map_err(Into::into)
                            .and_then(TryInto::try_into)?;

                            self.bill_subscription_with_data_tx(
                                conn,
                                event.tenant_id,
                                subscription.clone(),
                                customer,
                                InvoiceBillingMode::AlreadyPaid {
                                    charge_result: charge_result.clone(),
                                    existing_transaction_id: Some(event.payment_transaction_id),
                                },
                            )
                            .await?;

                            // Activate the subscription if it was pending checkout
                            let should_activate = subscription.subscription.activated_at.is_none()
                                && subscription.subscription.activation_condition
                                    == crate::domain::enums::SubscriptionActivationCondition::OnCheckout;

                            if should_activate {
                                let billing_start_date = subscription
                                    .subscription
                                    .billing_start_date
                                    .unwrap_or_else(|| Utc::now().date_naive());

                                let payment_method = CustomerPaymentMethodRow::get_by_id(
                                    conn,
                                    &event.tenant_id,
                                    &charge_result.payment_method_id,
                                )
                                .await
                                .map_err(|e| StoreError::DatabaseError(e.error))?;

                                self.activate_subscription_after_payment(
                                    conn,
                                    &subscription_id,
                                    &event.tenant_id,
                                    PaymentActivationParams {
                                        billing_start_date,
                                        trial_duration: subscription
                                            .subscription
                                            .trial_duration
                                            .map(|d| d as i32),
                                        billing_day_anchor: subscription.subscription.billing_day_anchor
                                            as u32,
                                        period: subscription.subscription.period,
                                        payment_method: Some(PaymentMethodInfo {
                                            id: charge_result.payment_method_id,
                                            method_type: payment_method.payment_method_type,
                                        }),
                                    },
                                )
                                .await?;
                            }

                            subscription_id
                        }
                    };

                    CheckoutSessionRow::mark_completed(
                        conn,
                        event.tenant_id,
                        checkout_session_id,
                        subscription_id,
                        chrono::Utc::now(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    log::info!(
                        "Completed checkout session {} with subscription {} after async payment",
                        checkout_session_id,
                        subscription_id
                    );

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }
}

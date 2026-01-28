use crate::StoreResult;
use crate::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use crate::domain::enums::{BillingPeriodEnum, PaymentStatusEnum};
use crate::domain::outbox_event::{
    InvoiceEvent, InvoicePdfGeneratedEvent, OutboxEvent, PaymentTransactionEvent,
    QuoteConvertedEvent,
};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    CheckoutSession, CreateSubscription, CreateSubscriptionFromQuote, CreatedSubscription,
    CustomerBuyCredits, DetailedInvoice, Invoice, QuoteActivityNew, SetupIntent, Subscription,
    SubscriptionDetails, UpdateInvoiceParams,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::InvoiceInterface;
use crate::repositories::outbox::OutboxInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::services::CycleTransitionResult;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::{InvoiceBillingMode, ServicesEdge};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::Datelike;
use chrono::{NaiveDate, Utc};
use common_domain::ids::{
    AppliedCouponId, BaseId, CheckoutSessionId, CustomerConnectionId, CustomerPaymentMethodId,
    InvoiceId, SubscriptionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::coupons::CouponRow;
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum as DbSubscriptionStatusEnum};
use diesel_models::plans::PlanRow;
use diesel_models::quotes::{QuoteActivityRowNew, QuoteRow};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt};
use uuid::Uuid;

impl ServicesEdge {
    async fn get_conn(&self) -> StoreResult<PgConn> {
        self.services.store.get_conn().await
    }

    pub async fn compute_invoice(
        &self,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.services
            .compute_invoice(
                &mut self.get_conn().await?,
                invoice_date,
                subscription_details,
                prepaid_amount,
                None,
            )
            .await
    }

    pub async fn create_setup_intent(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
    ) -> StoreResult<SetupIntent> {
        self.services
            .create_setup_intent(
                &mut self.get_conn().await?,
                tenant_id,
                customer_connection_id,
            )
            .await
    }

    pub async fn create_setup_intent_for_type(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        connection_type: crate::domain::ConnectionTypeEnum,
    ) -> StoreResult<SetupIntent> {
        self.services
            .create_setup_intent_for_type(
                &mut self.get_conn().await?,
                tenant_id,
                customer_connection_id,
                connection_type,
            )
            .await
    }

    pub async fn refresh_invoice_data(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .refresh_invoice_data(
                &mut self.get_conn().await?,
                invoice_id,
                tenant_id,
                &None,
                true,
            )
            .await?;

        self.store
            .get_detailed_invoice_by_id(tenant_id, invoice_id)
            .await
    }

    pub async fn get_and_process_cycle_transitions(&self) -> StoreResult<CycleTransitionResult> {
        self.services.get_and_process_cycle_transitions().await
    }

    pub async fn get_and_process_due_events(&self) -> StoreResult<usize> {
        self.services.get_and_process_due_events().await
    }

    pub async fn cleanup_timeout_scheduled_events(&self) -> StoreResult<()> {
        self.services.cleanup_timeout_scheduled_events().await
    }

    pub async fn buy_customer_credits(
        &self,
        params: CustomerBuyCredits,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .buy_customer_credits(&mut self.get_conn().await?, params)
            .await
    }

    /// Completes the checkout process for a subscription.
    ///
    /// For free trials (trial_is_free = true):
    /// - Saves the payment method on the subscription
    /// - Activates the subscription to TrialActive status
    /// - Returns (None, false)
    ///
    /// For paid subscriptions or paid trials:
    /// - Creates and finalizes an invoice (or draft if payment is pending)
    /// - Processes payment
    /// - Returns (Some(PaymentTransaction), is_pending)
    ///
    /// The `is_pending` flag indicates if the payment is still pending (async payment method).
    /// Caller should handle this by marking the checkout session as AwaitingPayment.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_subscription_checkout_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<(Option<PaymentTransaction>, bool), StoreErrorReport> {
        let subscription_row =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription = &subscription_row.subscription;

        let plan_with_version =
            PlanRow::get_with_version(conn, subscription.plan_version_id, tenant_id).await?;

        let plan_version = plan_with_version.version.ok_or(StoreError::ValueNotFound(
            "Plan version not found".to_string(),
        ))?;

        let is_free_trial = subscription.trial_duration.is_some() && plan_version.trial_is_free;
        let is_paid_trial = subscription.trial_duration.is_some() && !plan_version.trial_is_free;

        // Handle coupon if provided - lock and validate before any charging
        if let Some(code) = coupon_code {
            // Resolve coupon code to ID
            let coupon_rows =
                CouponRow::list_by_codes(conn, tenant_id, std::slice::from_ref(&code)).await?;

            let coupon_row = coupon_rows.into_iter().next().ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Coupon code '{}' not found",
                    code
                )))
            })?;

            let coupon_id = coupon_row.id;

            // Lock and validate coupon with FOR UPDATE to prevent race conditions
            let validated_coupons = self
                .services
                .lock_and_validate_coupons_for_checkout(
                    conn,
                    tenant_id,
                    subscription.customer_id,
                    &[coupon_id],
                    &subscription.currency,
                )
                .await?;

            // Apply the coupon (insert AppliedCoupon and update redemption count)
            if let Some(coupon) = validated_coupons.into_iter().next() {
                use crate::services::subscriptions::utils::apply_coupons_without_validation;

                let applied_coupon = AppliedCouponRowNew {
                    id: AppliedCouponId::new(),
                    subscription_id,
                    coupon_id: coupon.id,
                    customer_id: subscription.customer_id,
                    is_active: true,
                    applied_amount: None,
                    applied_count: None,
                    last_applied_at: None,
                };

                apply_coupons_without_validation(conn, &[&applied_coupon])
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
        }

        let is_trial_expired = subscription.status == DbSubscriptionStatusEnum::TrialExpired;

        if is_free_trial && !is_trial_expired {
            // Free trial checkout: just save the payment method, don't change the trial period.
            // The trial period remains unchanged (current_period_start/end stay the same).
            // Billing will happen when the trial ends via process_cycles.
            let payment_method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;

            // Keep existing period dates - only update payment method and clear pending_checkout
            SubscriptionRow::activate_subscription_with_payment_method(
                conn,
                &subscription_id,
                &tenant_id,
                subscription.current_period_start,
                subscription.current_period_end,
                subscription.next_cycle_action.clone(),
                subscription.cycle_index,
                DbSubscriptionStatusEnum::TrialActive,
                Some(payment_method_id),
                Some(payment_method.payment_method_type),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            Ok((None, false))
        } else {
            let detailed_invoice = self
                .services
                .bill_subscription_tx(
                    conn,
                    tenant_id,
                    subscription_id,
                    InvoiceBillingMode::FinalizeAfterPayment {
                        currency_confirmation,
                        total_amount_confirmation,
                        payment_method_id,
                    },
                )
                .await?
                .ok_or(StoreError::InsertError)
                .attach("Failed to bill the subscription")?;

            let payment_transaction = detailed_invoice
                .transactions
                .into_iter()
                .next()
                .ok_or(StoreError::InsertError)
                .attach("No payment transaction linked to invoice")?;

            // Handle payment status explicitly
            match payment_transaction.status {
                PaymentStatusEnum::Pending => {
                    // Payment is pending (e.g., async payment method like SEPA)
                    // Return early, subscription activation will happen via webhook
                    return Ok((Some(payment_transaction), true));
                }
                PaymentStatusEnum::Settled => {
                    // Payment succeeded, proceed with activation below
                }
                PaymentStatusEnum::Failed => {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Payment failed during checkout"));
                }
                PaymentStatusEnum::Cancelled => {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Payment was cancelled during checkout"));
                }
                PaymentStatusEnum::Ready => {
                    // Ready means not yet processed - this shouldn't happen after charging
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Payment was not processed correctly"));
                }
            }

            // Payment succeeded (settled) - activate the subscription
            // Calculate the billing period using the subscription's actual billing period
            let current_period_start = detailed_invoice.invoice.invoice_date;
            let billing_period: BillingPeriodEnum = subscription.period.clone().into();
            let period = calculate_advance_period_range(
                current_period_start,
                subscription.billing_day_anchor as u32,
                true, // First period
                &billing_period,
            );
            let current_period_end = Some(period.end);

            // Determine status and next action based on trial type
            let (new_status, next_action) = if is_paid_trial && !is_trial_expired {
                // Paid trial: go to TrialActive for feature resolution
                // Billing already happened, trial just affects plan features
                (
                    DbSubscriptionStatusEnum::TrialActive,
                    CycleActionEnum::EndTrial,
                )
            } else {
                // No trial or trial expired: go to Active
                (
                    DbSubscriptionStatusEnum::Active,
                    CycleActionEnum::RenewSubscription,
                )
            };

            SubscriptionRow::activate_subscription(
                conn,
                &subscription_id,
                &tenant_id,
                current_period_start,
                current_period_end,
                Some(next_action),
                Some(0),
                new_status,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            // If we get here, payment was settled
            Ok((Some(payment_transaction), false))
        }
    }

    pub async fn complete_invoice_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> StoreResult<PaymentTransaction> {
        self.store
            .transaction(|conn| {
                async move {
                    self.services
                        .process_invoice_payment_tx(conn, tenant_id, invoice_id, payment_method_id)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn get_or_create_customer_connections(
        &self,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        invoicing_entity_id: common_domain::ids::InvoicingEntityId,
    ) -> StoreResult<(Option<CustomerConnectionId>, Option<CustomerConnectionId>)> {
        self.services
            .get_or_create_customer_connections(
                &mut self.get_conn().await?,
                tenant_id,
                customer_id,
                invoicing_entity_id,
            )
            .await
    }

    pub async fn insert_subscription(
        &self,
        params: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch(vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(Report::new(StoreError::InsertError))
            .attach("No subscription inserted")
    }

    pub async fn insert_subscription_tx(
        &self,
        conn: &mut PgConn,
        params: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch_tx(conn, vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(Report::new(StoreError::InsertError))
            .attach("No subscription inserted")
    }

    /// Components and add-ons are already processed, so we skip plan-based processing.
    pub async fn insert_subscription_from_quote(
        &self,
        params: CreateSubscriptionFromQuote,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        let mut conn = self.get_conn().await?;
        let quote_id = params.quote_id;

        let context = self
            .services
            .gather_subscription_context_from_quote(
                &mut conn,
                &params,
                tenant_id,
                &self.store.settings.crypt_key,
            )
            .await?;

        let mut sub = self
            .services
            .build_subscription_details_from_quote(&params, &context)?;

        // For quote conversions, gracefully handle charge_automatically when
        // payment provider is not configured. This allows quotes to be converted even if
        // the invoicing entity doesn't have a payment provider set up yet.
        if sub.subscription.charge_automatically
            && let Some(invoicing_entity_providers) =
                context.get_invoicing_entity_providers_for_customer(&sub.customer)
        {
            let has_online_provider = invoicing_entity_providers.card_provider.is_some()
                || invoicing_entity_providers.direct_debit_provider.is_some();

            if !has_online_provider {
                log::warn!(
                    "Quote conversion: charge_automatically was set to true but no payment provider is configured. Falling back to charge_automatically=false for quote_id={}",
                    quote_id.as_base62()
                );
                sub.subscription.charge_automatically = false;
            }
        }

        let payment_result = self
            .services
            .setup_payment_provider(&mut conn, &sub.subscription, &sub.customer, &context)
            .await?;

        let processed = self.services.process_subscription(
            &sub,
            &payment_result,
            &context,
            tenant_id,
            Some(params.quote_id),
        )?;

        let inserted = self
            .store
            .transaction_with(&mut conn, |conn| {
                async move {
                    let mut inserted = self
                        .services
                        .persist_subscriptions(
                            conn,
                            &[processed],
                            tenant_id,
                            &self.store.settings.jwt_secret,
                            &self.store.settings.public_url,
                        )
                        .await?;

                    let inserted = inserted
                        .pop()
                        .ok_or(Report::new(StoreError::InsertError))
                        .attach("No subscription inserted from quote")?;

                    let rows_updated = QuoteRow::mark_as_converted_to_subscription(
                        conn,
                        quote_id,
                        tenant_id,
                        inserted.id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    if rows_updated == 0 {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Quote has already been converted to a subscription".to_string(),
                        )));
                    }

                    let activity = QuoteActivityNew {
                        quote_id,
                        activity_type: "converted".to_string(),
                        description: format!(
                            "Quote converted to subscription {}",
                            inserted.id.as_base62()
                        ),
                        actor_type: "user".to_string(),
                        actor_id: Some(inserted.created_by.to_string()),
                        actor_name: None,
                        ip_address: None,
                        user_agent: None,
                    };
                    let activity_row: QuoteActivityRowNew = activity.into();
                    activity_row
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let quote_converted_event = QuoteConvertedEvent::new(
                        quote_id,
                        tenant_id,
                        inserted.customer_id,
                        inserted.id,
                    );
                    self.store
                        .insert_outbox_event_tx(
                            conn,
                            OutboxEvent::quote_converted(quote_converted_event),
                        )
                        .await?;

                    Ok(inserted)
                }
                .scope_boxed()
            })
            .await?;

        self.services
            .handle_post_insertion(self.store.eventbus.clone(), std::slice::from_ref(&inserted))
            .await?;

        Ok(inserted)
    }

    pub async fn insert_subscription_batch(
        &self,
        batch: Vec<CreateSubscription>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let mut conn = self.get_conn().await?;

        self.insert_subscription_batch_tx(&mut conn, batch, tenant_id)
            .await
    }

    pub async fn insert_subscription_batch_tx(
        &self,
        conn: &mut PgConn,
        batch: Vec<CreateSubscription>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let context = self
            .services
            .gather_subscription_context(conn, &batch, tenant_id, &self.store.settings.crypt_key)
            .await?;

        let subscriptions = self.services.build_subscription_details(&batch, &context)?;

        let mut results = Vec::new();
        for sub in subscriptions {
            let result = self
                .services
                .setup_payment_provider(conn, &sub.subscription, &sub.customer, &context)
                .await?;

            let processed = self
                .services
                .process_subscription(&sub, &result, &context, tenant_id, None)?;

            results.push(processed);
        }

        let inserted = self
            .services
            .persist_subscriptions(
                conn,
                &results,
                tenant_id,
                &self.store.settings.jwt_secret,
                &self.store.settings.public_url,
            )
            .await?;

        self.services
            .handle_post_insertion(self.store.eventbus.clone(), &inserted)
            .await?;

        Ok(inserted)
    }

    pub async fn cancel_subscription(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        actor: Uuid,
    ) -> StoreResult<Subscription> {
        self.services
            .cancel_subscription(subscription_id, tenant_id, reason, effective_at, actor)
            .await
    }

    pub async fn on_invoice_accounting_pdf_generated(
        &self,
        event: InvoicePdfGeneratedEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .on_invoice_accounting_pdf_generated(event, tenant_id)
            .await
    }

    pub async fn on_invoice_paid(
        &self,
        event: InvoiceEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services.on_invoice_paid(event, tenant_id).await
    }

    pub async fn on_payment_transaction_settled(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        self.services.on_payment_transaction_settled(event).await
    }

    pub async fn finalize_invoice(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.services.finalize_invoice(invoice_id, tenant_id).await
    }

    pub async fn update_draft_invoice(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .update_draft_invoice(invoice_id, tenant_id, params)
            .await
    }

    pub async fn preview_draft_invoice_update(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        params: UpdateInvoiceParams,
    ) -> StoreResult<Invoice> {
        self.services
            .preview_draft_invoice_update(invoice_id, tenant_id, params)
            .await
    }

    /// For SelfServe checkout type :
    /// - Validates payment / charges customer FIRST
    /// - Only creates subscription if payment succeeds
    /// - Marks session completed
    ///
    /// For SubscriptionActivation checkout type:
    /// - Uses the linked subscription (already created via OnCheckout)
    /// - Processes payment (activates the subscription)
    /// - Marks session completed
    pub async fn complete_checkout(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let result = self
            .store
            .transaction(|conn| {
                async move {
                    let session_row = CheckoutSessionRow::get_by_id_for_update(
                        conn,
                        tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let session: CheckoutSession = session_row.into();

                    if !session.can_complete() {
                        if session.is_completed() {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "Checkout session has already been completed".to_string(),
                            )));
                        }
                        if session.is_expired() {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "Checkout session has expired".to_string(),
                            )));
                        }
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Checkout session is not in a valid state for completion".to_string(),
                        )));
                    }

                    match session.checkout_type {
                        CheckoutType::SubscriptionActivation => {
                            self.complete_checkout_activation_tx(
                                conn,
                                tenant_id,
                                checkout_session_id,
                                session,
                                payment_method_id,
                                total_amount_confirmation,
                                currency_confirmation,
                                coupon_code,
                            )
                            .await
                        }
                        CheckoutType::SelfServe => {
                            self.complete_checkout_self_serve_tx(
                                conn,
                                tenant_id,
                                checkout_session_id,
                                session,
                                payment_method_id,
                                total_amount_confirmation,
                                currency_confirmation,
                                coupon_code,
                            )
                            .await
                        }
                    }
                }
                .scope_boxed()
            })
            .await?;

        Ok(result)
    }

    /// Completes checkout for SubscriptionActivation type.
    /// The subscription already exists; we just need to activate it via payment.
    #[allow(clippy::too_many_arguments)]
    async fn complete_checkout_activation_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let subscription_id = session.subscription_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Session has no linked subscription for activation flow".to_string(),
            ))
        })?;

        let coupon_for_checkout = coupon_code.or(session.coupon_code.clone());

        let (payment_transaction, is_pending) = self
            .complete_subscription_checkout_tx(
                conn,
                tenant_id,
                subscription_id,
                payment_method_id,
                total_amount_confirmation,
                currency_confirmation,
                coupon_for_checkout,
            )
            .await?;

        if is_pending {
            CheckoutSessionRow::mark_awaiting_payment(
                conn,
                tenant_id,
                checkout_session_id,
                Some(subscription_id),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            Ok(CheckoutCompletionResult::AwaitingPayment {
                transaction: payment_transaction.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Pending payment must have transaction".to_string(),
                    ))
                })?,
            })
        } else {
            CheckoutSessionRow::mark_completed(
                conn,
                tenant_id,
                checkout_session_id,
                subscription_id,
                Utc::now(),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            Ok(CheckoutCompletionResult::Completed {
                subscription_id,
                transaction: payment_transaction,
            })
        }
    }

    /// Completes checkout for SelfServe type :
    /// 1. Build preview to compute amount
    /// 2. Charge customer (or validate payment method for free trials)
    /// 3. Only create subscription if payment succeeds (or is pending for async)
    /// 4. For pending payments: create transaction only, defer subscription/invoice
    /// 5. Mark session completed (sync) or awaiting payment (async)
    #[allow(clippy::too_many_arguments)]
    async fn complete_checkout_self_serve_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let effective_coupon_code = coupon_code.clone().or(session.coupon_code.clone());
        let preview_details = self
            .services
            .build_preview_subscription_details(
                conn,
                &session,
                tenant_id,
                effective_coupon_code.as_deref(),
            )
            .await?;

        let invoice_content = self
            .services
            .compute_invoice(
                conn,
                &preview_details.subscription.current_period_start,
                &preview_details,
                None,
                None,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let amount_due = invoice_content.total;
        let currency = preview_details.subscription.currency.clone();

        if currency != currency_confirmation {
            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                "Currency mismatch: expected {}, got {}",
                currency, currency_confirmation
            )));
        }

        let coupon_ids = self
            .services
            .resolve_coupon_ids_for_checkout_tx(conn, tenant_id, &session, effective_coupon_code)
            .await?;

        let _locked_coupons = self
            .services
            .lock_and_validate_coupons_for_checkout(
                conn,
                tenant_id,
                session.customer_id,
                &coupon_ids,
                &currency,
            )
            .await?;

        let is_free_trial = preview_details
            .trial_config
            .as_ref()
            .is_some_and(|tc| tc.is_free);

        let charge_result = if is_free_trial || amount_due <= 0 {
            let _method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|_| {
                    Report::new(StoreError::InvalidArgument(
                        "Payment method not found".to_string(),
                    ))
                })?;

            None
        } else {
            let amount_diff = (amount_due - total_amount_confirmation as i64).abs();
            if amount_diff > 1 {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "Amount mismatch: expected {}, got {}",
                    amount_due, total_amount_confirmation
                )));
            }

            // TODO: If subscription creation fails after this charge succeeds, we should allow manual reconciliation or auto refund.
            let result = self
                .services
                .charge_payment_method_directly(
                    conn,
                    tenant_id,
                    payment_method_id,
                    amount_due,
                    currency.clone(),
                )
                .await?;

            if result.payment_intent.status == PaymentStatusEnum::Pending {
                let transaction = self
                    .services
                    .create_transaction_for_checkout(conn, tenant_id, checkout_session_id, &result)
                    .await?;

                CheckoutSessionRow::mark_awaiting_payment(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    None,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                return Ok(CheckoutCompletionResult::AwaitingPayment { transaction });
            }

            Some(result)
        };

        let start_date = session
            .billing_start_date
            .unwrap_or_else(|| Utc::now().date_naive());

        let create_subscription = session.to_create_subscription(start_date, coupon_ids);

        let trial_config = preview_details.trial_config.clone();

        let created_subscription = self
            .insert_subscription_tx(conn, create_subscription, tenant_id)
            .await?;

        let payment_transaction = if let Some(charge_result) = charge_result {
            let payment_method_id = charge_result.payment_method_id;

            let detailed_invoice = self
                .services
                .bill_subscription_tx(
                    conn,
                    tenant_id,
                    created_subscription.id,
                    InvoiceBillingMode::AlreadyPaid {
                        charge_result,
                        existing_transaction_id: None,
                    },
                )
                .await?
                .ok_or(StoreError::InsertError)
                .attach("Failed to create invoice for subscription")?;

            // Activate the subscription now that payment is confirmed
            let payment_method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;

            let has_paid_trial = trial_config.as_ref().is_some_and(|tc| !tc.is_free);
            let trial_duration = if has_paid_trial {
                trial_config.as_ref().map(|tc| tc.duration_days as i32)
            } else {
                None
            };

            let subscription =
                SubscriptionRow::get_subscription_by_id(conn, &tenant_id, created_subscription.id)
                    .await?;

            let billing_day_anchor = session
                .billing_day_anchor
                .map(|a| a as u32)
                .unwrap_or_else(|| start_date.day());

            self.services
                .activate_subscription_after_payment(
                    conn,
                    &created_subscription.id,
                    &tenant_id,
                    crate::services::subscriptions::PaymentActivationParams {
                        billing_start_date: start_date,
                        trial_duration,
                        billing_day_anchor,
                        period: subscription.subscription.period.into(),
                        payment_method: Some(crate::services::subscriptions::PaymentMethodInfo {
                            id: payment_method_id,
                            method_type: payment_method.payment_method_type,
                        }),
                    },
                )
                .await?;

            detailed_invoice.transactions.into_iter().next()
        } else if is_free_trial {
            let payment_method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;

            let trial_duration = trial_config.as_ref().map(|tc| tc.duration_days as i32);

            let subscription =
                SubscriptionRow::get_subscription_by_id(conn, &tenant_id, created_subscription.id)
                    .await?;

            let billing_day_anchor = session
                .billing_day_anchor
                .map(|a| a as u32)
                .unwrap_or_else(|| start_date.day());

            self.services
                .activate_subscription_after_payment(
                    conn,
                    &created_subscription.id,
                    &tenant_id,
                    crate::services::subscriptions::PaymentActivationParams {
                        billing_start_date: start_date,
                        trial_duration,
                        billing_day_anchor,
                        period: subscription.subscription.period.into(),
                        payment_method: Some(crate::services::subscriptions::PaymentMethodInfo {
                            id: payment_method_id,
                            method_type: payment_method.payment_method_type,
                        }),
                    },
                )
                .await?;

            None
        } else {
            // No payment, no trial - activate directly
            SubscriptionRow::activate_subscription(
                conn,
                &created_subscription.id,
                &tenant_id,
                start_date,
                None,
                None,
                Some(0),
                DbSubscriptionStatusEnum::Active,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            None
        };

        CheckoutSessionRow::mark_completed(
            conn,
            tenant_id,
            checkout_session_id,
            created_subscription.id,
            Utc::now(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(CheckoutCompletionResult::Completed {
            subscription_id: created_subscription.id,
            transaction: payment_transaction,
        })
    }
}

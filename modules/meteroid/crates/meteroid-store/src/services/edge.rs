#![allow(clippy::too_many_arguments)]
use crate::StoreResult;
use crate::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use crate::domain::enums::{BillingPeriodEnum, PaymentStatusEnum};
use crate::domain::outbox_event::{
    InvoiceEvent, InvoicePdfGeneratedEvent, OutboxEvent, PaymentTransactionEvent,
    QuoteConvertedEvent,
};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::scheduled_events::ScheduledEventNew;
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{
    CheckoutSession, CreateSubscription, CreateSubscriptionFromQuote, CreatedSubscription,
    Customer, CustomerBuyCredits, DetailedInvoice, Invoice, InvoicingEntityProviderSensitive,
    QuoteActivityNew, SetupIntent, Subscription, SubscriptionDetails, UpdateInvoiceParams,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::outbox::OutboxInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::repositories::{InvoiceInterface, SubscriptionInterface};
use crate::services::CycleTransitionResult;
use crate::services::clients::usage::WindowedUsageData;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::subscriptions::payment_resolution::ResolvedPaymentMethods;
use crate::services::{InvoiceBillingMode, ServicesEdge};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::Datelike;
use chrono::{NaiveDate, Utc};
use common_domain::ids::{
    AppliedCouponId, BaseId, CheckoutSessionId, CustomerConnectionId, CustomerPaymentMethodId,
    InvoiceId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::coupons::CouponRow;
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum as DbSubscriptionStatusEnum};
use diesel_models::plans::PlanRow;
use diesel_models::quotes::{QuoteActivityRowNew, QuoteRow};
use diesel_models::scheduled_events::ScheduledEventRowNew;
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

    pub async fn compute_upcoming_invoice(
        &self,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.services
            .compute_upcoming_invoice(&mut self.get_conn().await?, subscription_details)
            .await
    }

    pub async fn get_subscription_component_usage(
        &self,
        subscription_details: &SubscriptionDetails,
        metric_id: common_domain::ids::BillableMetricId,
    ) -> StoreResult<WindowedUsageData> {
        self.services
            .get_subscription_component_usage(subscription_details, metric_id)
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
            // Free trial checkout: activate the subscription, don't change the trial period.
            // The trial period remains unchanged (current_period_start/end stay the same).
            // Billing will happen when the trial ends via process_cycles.
            // Validate the payment method exists (it's already saved on the customer)
            let _payment_method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;

            // Keep existing period dates - payment method is resolved dynamically from the customer
            SubscriptionRow::activate_subscription(
                conn,
                &subscription_id,
                &tenant_id,
                subscription.current_period_start,
                subscription.current_period_end,
                subscription.next_cycle_action.clone(),
                subscription.cycle_index,
                DbSubscriptionStatusEnum::TrialActive,
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

            let payment_transaction = detailed_invoice.transactions.into_iter().next();

            // Handle payment status explicitly
            // None means zero amount (e.g., 100% coupon) - invoice already finalized and marked paid
            if let Some(ref txn) = payment_transaction {
                match txn.status {
                    PaymentStatusEnum::Pending => {
                        // Payment is pending (e.g., async payment method like SEPA)
                        // Return early, subscription activation will happen via webhook
                        return Ok((payment_transaction, true));
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
            }

            // Payment succeeded (settled) or zero amount - activate the subscription
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
                // Use RenewSubscription for normal billing cycles - trial end handled via scheduled event
                (
                    DbSubscriptionStatusEnum::TrialActive,
                    CycleActionEnum::RenewSubscription,
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

            // For paid trials, schedule the EndTrial event to transition status when trial ends
            if is_paid_trial
                && !is_trial_expired
                && let Some(trial_days) = subscription.trial_duration
            {
                let scheduled_event = ScheduledEventNew::end_trial(
                    subscription_id,
                    tenant_id,
                    current_period_start,
                    trial_days,
                    "checkout_completion",
                )
                .ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Failed to compute trial end date".to_string(),
                    ))
                })?;
                let insertable: ScheduledEventRowNew = scheduled_event.try_into()?;
                ScheduledEventRowNew::insert_batch(conn, &[insertable])
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }

            // Return transaction if any (None for zero amount case)
            Ok((payment_transaction, false))
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

    /// Resolves payment methods for a subscription based on its config and current invoicing entity (inherited or overridden)
    pub async fn resolve_subscription_payment_methods(
        &self,
        tenant_id: TenantId,
        payment_methods_config: Option<&PaymentMethodsConfig>,
        customer: &Customer,
    ) -> StoreResult<ResolvedPaymentMethods> {
        use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;

        let mut conn = self.get_conn().await?;

        let providers_row = InvoicingEntityProvidersRow::resolve_providers_by_id(
            &mut conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let invoicing_entity_providers = InvoicingEntityProviderSensitive::from_row(
            providers_row,
            &self.store.settings.crypt_key,
        )?;

        self.services
            .resolve_payment_methods(
                &mut conn,
                tenant_id,
                payment_methods_config,
                customer,
                &invoicing_entity_providers,
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

        // For quote conversions, gracefully handle charge_automatically when the configuration
        // is invalid. This allows quotes to be converted even if misconfigured.
        // We validate and adjust values to be consistent rather than failing.
        if sub.subscription.charge_automatically {
            // Check 1: payment_methods_config must be Online (or None which defaults to Online)
            let is_online_config = match &sub.subscription.payment_methods_config {
                None => true,
                Some(PaymentMethodsConfig::Online { .. }) => true,
                Some(PaymentMethodsConfig::BankTransfer { .. }) => false,
                Some(PaymentMethodsConfig::External) => false,
            };

            if !is_online_config {
                log::warn!(
                    "Quote conversion: charge_automatically was set to true but payment_methods_config is not Online. Falling back to charge_automatically=false for quote_id={}",
                    quote_id.as_base62()
                );
                sub.subscription.charge_automatically = false;
            } else if let Some(invoicing_entity_providers) =
                context.get_invoicing_entity_providers_for_customer(&sub.customer)
            {
                // Check 2: Invoicing entity must have an online provider
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
        }

        // For quote conversions, gracefully handle payment_methods_config when
        // online payment is configured but no payment provider exists.
        if let Some(ref config) = sub.subscription.payment_methods_config
            && config.is_online()
            && let Some(invoicing_entity_providers) =
                context.get_invoicing_entity_providers_for_customer(&sub.customer)
        {
            let has_online_provider = invoicing_entity_providers.card_provider.is_some()
                || invoicing_entity_providers.direct_debit_provider.is_some();

            if !has_online_provider {
                log::warn!(
                    "Quote conversion: payment_methods_config was set to Online but no payment provider is configured. Falling back to External for quote_id={}",
                    quote_id.as_base62()
                );
                sub.subscription.payment_methods_config = Some(PaymentMethodsConfig::External);
            }
        }

        let payment_result =
            self.services
                .setup_payment_provider(&sub.subscription, &sub.customer, &context)?;

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
            let result =
                self.services
                    .setup_payment_provider(&sub.subscription, &sub.customer, &context)?;

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

    pub async fn schedule_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
    ) -> StoreResult<crate::domain::scheduled_events::ScheduledEvent> {
        self.services
            .schedule_plan_change(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
            )
            .await
    }

    pub async fn preview_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
        mode: Option<crate::domain::subscription_changes::PlanChangeMode>,
    ) -> StoreResult<crate::domain::subscription_changes::PlanChangePreviewExtended> {
        self.services
            .preview_plan_change(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
                mode,
            )
            .await
    }

    /// Compute the invoice content for a plan change checkout preview.
    /// Mirrors the exact amount computation from `complete_checkout_plan_change_tx`.
    pub async fn compute_plan_change_checkout_invoice(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut conn = self.get_conn().await?;

        let prepared = self
            .services
            .prepare_plan_change_readonly(
                &mut conn,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                &[],
                change_date,
            )
            .await?;

        if prepared.is_free_trial() {
            let preview_details =
                prepared.build_trial_change_preview(change_date, new_plan_version_id)?;
            self.services
                .compute_invoice(
                    &mut conn,
                    &preview_details.subscription.current_period_start,
                    &preview_details,
                    None,
                    None,
                )
                .await
                .change_context(StoreError::InvoiceComputationError)
        } else {
            Ok(self
                .services
                .compute_adjustment_invoice_content(
                    &mut conn,
                    tenant_id,
                    &prepared.subscription_details.subscription,
                    &prepared.subscription_details.customer,
                    &prepared.proration,
                )
                .await?
                .computed)
        }
    }

    pub async fn apply_plan_change_immediate(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
    ) -> StoreResult<crate::domain::subscription_changes::ImmediatePlanChangeResult> {
        self.services
            .apply_plan_change_immediate(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
            )
            .await
    }

    pub async fn apply_plan_change_immediate_at(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
        change_date: NaiveDate,
    ) -> StoreResult<crate::domain::subscription_changes::ImmediatePlanChangeResult> {
        self.services
            .apply_plan_change_immediate_at(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
                change_date,
            )
            .await
    }

    pub async fn cancel_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .cancel_plan_change(subscription_id, tenant_id)
            .await
    }

    pub async fn cancel_scheduled_event(
        &self,
        event_id: common_domain::ids::ScheduledEventId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .cancel_scheduled_event(event_id, subscription_id, tenant_id)
            .await
    }

    pub async fn update_matrix_prices(
        &self,
        tenant_id: TenantId,
        product_id: common_domain::ids::ProductId,
        update: crate::domain::prices::MatrixPriceUpdate,
        actor: uuid::Uuid,
    ) -> StoreResult<Vec<crate::domain::prices::Price>> {
        self.services
            .update_matrix_prices(tenant_id, product_id, update, actor)
            .await
    }

    pub async fn preview_matrix_update(
        &self,
        tenant_id: TenantId,
        product_id: common_domain::ids::ProductId,
        update: &crate::domain::prices::MatrixPriceUpdate,
    ) -> StoreResult<crate::domain::prices::MatrixUpdatePreview> {
        self.services
            .preview_matrix_update(tenant_id, product_id, update)
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

    pub async fn create_corrected_invoice_from(
        &self,
        tenant_id: TenantId,
        parent_invoice_id: InvoiceId,
    ) -> StoreResult<Invoice> {
        self.services
            .create_corrected_invoice_from(tenant_id, parent_invoice_id)
            .await
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

    /// Creates a checkout session for a plan change.
    /// Used when off-session payment fails or no saved payment method is available.
    pub async fn create_plan_change_checkout_session(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        new_plan_version_id: PlanVersionId,
        customer_id: common_domain::ids::CustomerId,
        created_by: uuid::Uuid,
        payment_methods_config: Option<PaymentMethodsConfig>,
        change_date: NaiveDate,
    ) -> StoreResult<CheckoutSession> {
        use crate::domain::checkout_sessions::{
            CheckoutType as DomainCheckoutType, CreateCheckoutSession,
        };
        use crate::repositories::checkout_sessions::CheckoutSessionsInterface;

        let session = CreateCheckoutSession {
            tenant_id,
            customer_id,
            plan_version_id: new_plan_version_id,
            created_by,
            billing_start_date: None,
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: DomainCheckoutType::PlanChange,
            subscription_id: Some(subscription_id),
            change_date: Some(change_date),
        };

        self.store.create_checkout_session(session).await
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
                        CheckoutType::PlanChange => {
                            self.complete_checkout_plan_change_tx(
                                conn,
                                tenant_id,
                                checkout_session_id,
                                session,
                                payment_method_id,
                                total_amount_confirmation,
                                currency_confirmation,
                            )
                            .await
                        }
                        CheckoutType::AddonPurchase => {
                            self.complete_checkout_addon_purchase_tx(
                                conn,
                                tenant_id,
                                checkout_session_id,
                                session,
                                payment_method_id,
                                total_amount_confirmation,
                                currency_confirmation,
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

        // Zero amount due to coupon discount (not free trial)
        let is_zero_amount = amount_due <= 0 && !is_free_trial;

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
                        is_paid_trial: has_paid_trial,
                        billing_day_anchor,
                        period: subscription.subscription.period.into(),
                    },
                )
                .await?;

            detailed_invoice.transactions.into_iter().next()
        } else if is_free_trial {
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
                        is_paid_trial: false, // Free trial
                        billing_day_anchor,
                        period: subscription.subscription.period.into(),
                    },
                )
                .await?;

            None
        } else if is_zero_amount {
            // Zero amount (e.g., 100% coupon) but not a free trial
            // Could still be a paid trial with 100% discount
            // Create invoice (to show line items + discount) and set up billing cycles
            // Zero amount is already marked as paid by bill_subscription_tx with Immediate mode
            self.services
                .bill_subscription_tx(
                    conn,
                    tenant_id,
                    created_subscription.id,
                    InvoiceBillingMode::Immediate,
                )
                .await?
                .ok_or(StoreError::InsertError)
                .attach("Failed to create invoice for zero-amount subscription")?;

            let subscription =
                SubscriptionRow::get_subscription_by_id(conn, &tenant_id, created_subscription.id)
                    .await?;

            let billing_day_anchor = session
                .billing_day_anchor
                .map(|a| a as u32)
                .unwrap_or_else(|| start_date.day());

            // Check if this is a paid trial (trial exists but is not free)
            let has_paid_trial = trial_config.as_ref().is_some_and(|tc| !tc.is_free);
            let trial_duration = if has_paid_trial {
                trial_config.as_ref().map(|tc| tc.duration_days as i32)
            } else {
                None
            };

            self.services
                .activate_subscription_after_payment(
                    conn,
                    &created_subscription.id,
                    &tenant_id,
                    crate::services::subscriptions::PaymentActivationParams {
                        billing_start_date: start_date,
                        trial_duration,
                        is_paid_trial: has_paid_trial,
                        billing_day_anchor,
                        period: subscription.subscription.period.into(),
                    },
                )
                .await?;

            None
        } else {
            // No payment, no trial, no zero-amount - this shouldn't happen
            // but keep as fallback for completely free plans
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
    //
    // pub async fn add_subscription_addon(
    //     &self,
    //     subscription_id: SubscriptionId,
    //     tenant_id: TenantId,
    //     add_on_id: common_domain::ids::AddOnId,
    //     require_self_serviceable: bool,
    // ) -> StoreResult<crate::domain::subscription_add_ons::SubscriptionAddOn> {
    //     use crate::domain::subscription_add_ons::{
    //         SubscriptionAddOnCustomization, SubscriptionAddOnNew, SubscriptionAddOnNewInternal,
    //     };
    //     use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
    //
    //     let inserted = self
    //         .store
    //         .transaction(|conn| {
    //             async move {
    //                 // Lock subscription row to serialize concurrent addon additions
    //                 SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
    //                     .await
    //                     .map_err(Into::<Report<StoreError>>::into)?;
    //
    //                 let details = self
    //                     .store
    //                     .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
    //                     .await?;
    //
    //                 let addon_row =
    //                     diesel_models::add_ons::AddOnRow::get_by_id(conn, tenant_id, add_on_id)
    //                         .await
    //                         .map_err(Into::<Report<StoreError>>::into)?;
    //                 let addon = {
    //                     let rows = vec![addon_row];
    //                     crate::repositories::add_ons::enrich_add_ons(conn, rows, tenant_id)
    //                         .await?
    //                         .into_iter()
    //                         .next()
    //                         .ok_or_else(|| {
    //                             Report::new(StoreError::InvalidArgument(format!(
    //                                 "Add-on {} not found after enrichment",
    //                                 add_on_id
    //                             )))
    //                         })?
    //                 };
    //
    //                 let pv_addon_rows =
    //                     diesel_models::plan_version_add_ons::PlanVersionAddOnRow::list_by_plan_version_id(
    //                         conn,
    //                         details.subscription.plan_version_id,
    //                         tenant_id,
    //                     )
    //                     .await
    //                     .map_err(Into::<Report<StoreError>>::into)?;
    //
    //                 let plan_version_add_ons: Vec<crate::domain::PlanVersionAddOn> =
    //                     pv_addon_rows.into_iter().map(Into::into).collect();
    //
    //                 let pv_addon = plan_version_add_ons
    //                     .iter()
    //                     .find(|pva| pva.add_on_id == add_on_id)
    //                     .ok_or_else(|| {
    //                         Report::new(StoreError::InvalidArgument(format!(
    //                             "Add-on {} is not attached to plan version {}",
    //                             add_on_id, details.subscription.plan_version_id
    //                         )))
    //                     })?;
    //
    //                 let is_self_serviceable = pv_addon
    //                     .self_serviceable
    //                     .unwrap_or(addon.self_serviceable);
    //                 if require_self_serviceable && !is_self_serviceable {
    //                     return Err(Report::new(StoreError::InvalidArgument(
    //                         "Add-on is not self-serviceable".to_string(),
    //                     )));
    //                 }
    //
    //                 let max_instances = pv_addon
    //                     .max_instances_per_subscription
    //                     .or(addon.max_instances_per_subscription);
    //                 let existing_count = details
    //                     .add_ons
    //                     .iter()
    //                     .filter(|a| a.add_on_id == add_on_id)
    //                     .map(|a| a.quantity as i64)
    //                     .sum::<i64>();
    //                 if let Some(max) = max_instances {
    //                     if existing_count + 1 > max as i64 {
    //                         return Err(Report::new(StoreError::InvalidArgument(format!(
    //                             "Add-on {} would exceed max instances ({}/{})",
    //                             add_on_id,
    //                             existing_count + 1,
    //                             max
    //                         ))));
    //                     }
    //                 }
    //
    //                 let mut products = std::collections::HashMap::new();
    //                 let mut prices = std::collections::HashMap::new();
    //
    //                 let product_rows = diesel_models::products::ProductRow::list_by_ids(
    //                     conn,
    //                     &[addon.product_id],
    //                     tenant_id,
    //                 )
    //                 .await
    //                 .map_err(Into::<Report<StoreError>>::into)?;
    //                 for row in product_rows {
    //                     let id = row.id;
    //                     products.insert(id, crate::domain::Product::try_from(row)?);
    //                 }
    //
    //                 let price_rows = diesel_models::prices::PriceRow::list_by_ids(
    //                     conn,
    //                     &[addon.price_id],
    //                     tenant_id,
    //                 )
    //                 .await
    //                 .map_err(Into::<Report<StoreError>>::into)?;
    //                 for row in price_rows {
    //                     let id = row.id;
    //                     prices.insert(id, crate::domain::Price::try_from(row)?);
    //                 }
    //
    //                 let resolved = addon
    //                     .resolve_customized(
    //                         &products,
    //                         &prices,
    //                         &SubscriptionAddOnCustomization::None,
    //                     )
    //                     .map_err(Report::new)?;
    //
    //                 let new_internal = SubscriptionAddOnNewInternal {
    //                     add_on_id: addon.id,
    //                     name: resolved.name,
    //                     period: resolved.period,
    //                     fee: resolved.fee,
    //                     product_id: resolved.product_id,
    //                     price_id: resolved.price_id,
    //                     quantity: 1,
    //                 };
    //
    //                 let new = SubscriptionAddOnNew {
    //                     subscription_id,
    //                     internal: new_internal,
    //                 };
    //
    //                 let row_new: SubscriptionAddOnRowNew = new.try_into()?;
    //                 let rows = SubscriptionAddOnRow::insert_batch(conn, vec![&row_new])
    //                     .await
    //                     .map_err(Into::<Report<StoreError>>::into)?;
    //                 rows.into_iter()
    //                     .next()
    //                     .ok_or_else(|| Report::new(StoreError::InsertError))
    //             }
    //             .scope_boxed()
    //         })
    //         .await?;
    //
    //     inserted.try_into().map_err(Into::into)
    // }

    pub async fn create_addon_purchase_checkout_session(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        add_on_id: common_domain::ids::AddOnId,
        plan_version_id: PlanVersionId,
        customer_id: common_domain::ids::CustomerId,
        created_by: uuid::Uuid,
        payment_methods_config: Option<PaymentMethodsConfig>,
    ) -> StoreResult<CheckoutSession> {
        use crate::domain::checkout_sessions::{
            CheckoutType as DomainCheckoutType, CreateCheckoutSession,
        };
        use crate::domain::subscription_add_ons::{
            CreateSubscriptionAddOn, CreateSubscriptionAddOns, SubscriptionAddOnCustomization,
        };
        use crate::repositories::checkout_sessions::CheckoutSessionsInterface;

        let today = Utc::now().date_naive();

        let session = CreateCheckoutSession {
            tenant_id,
            customer_id,
            plan_version_id,
            created_by,
            billing_start_date: None,
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config,
            components: None,
            add_ons: Some(CreateSubscriptionAddOns {
                add_ons: vec![CreateSubscriptionAddOn {
                    add_on_id,
                    customization: SubscriptionAddOnCustomization::None,
                    quantity: 1,
                }],
            }),
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: DomainCheckoutType::AddonPurchase,
            subscription_id: Some(subscription_id),
            change_date: Some(today),
        };

        self.store.create_checkout_session(session).await
    }

    /// Completes checkout for AddonPurchase type.
    /// Inserts addon rows, charges the customer, and creates an invoice.
    async fn complete_checkout_addon_purchase_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        use crate::repositories::subscription_add_ons::resolve_and_insert_checkout_addons;
        use crate::repositories::subscriptions::fetch_prices_and_products;

        let subscription_id = session.subscription_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Session has no linked subscription for addon purchase flow".to_string(),
            ))
        })?;

        let create_add_ons = session.add_ons.as_ref().ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "AddonPurchase checkout session has no add_ons".to_string(),
            ))
        })?;

        let addon_ids: Vec<_> = create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();
        let addons = {
            let rows = diesel_models::add_ons::AddOnRow::list_by_ids(conn, &addon_ids, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            crate::repositories::add_ons::enrich_add_ons(conn, rows, tenant_id).await?
        };

        let product_ids: Vec<_> = addons.iter().map(|a| a.product_id).collect();
        let price_ids: Vec<_> = addons.iter().map(|a| a.price_id).collect();
        let (prices_by_id, products_by_id) = fetch_prices_and_products(
            conn,
            tenant_id,
            price_ids.into_iter(),
            product_ids.into_iter(),
        )
        .await?;

        resolve_and_insert_checkout_addons(
            conn,
            subscription_id,
            &addons,
            &create_add_ons.add_ons,
            &products_by_id,
            &prices_by_id,
        )
        .await?;

        let result = self
            .services
            .compute_addon_purchase_invoice(
                conn,
                tenant_id,
                subscription_id,
                &create_add_ons.add_ons,
                &addons,
                &products_by_id,
                &prices_by_id,
            )
            .await?;

        let currency = result.subscription.currency.clone();
        if currency != currency_confirmation {
            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                "Currency mismatch: expected {}, got {}",
                currency, currency_confirmation
            )));
        }

        let expected_amount = result.invoice_content.amount_due;

        let amount_diff = (expected_amount - total_amount_confirmation as i64).abs();
        if amount_diff > 1 {
            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                "Amount mismatch: server computed {}, client sent {}",
                expected_amount, total_amount_confirmation
            )));
        }

        let charge_amount = expected_amount;

        if charge_amount <= 0 {
            CheckoutSessionRow::mark_completed(
                conn,
                tenant_id,
                checkout_session_id,
                subscription_id,
                Utc::now(),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            return Ok(CheckoutCompletionResult::Completed {
                subscription_id,
                transaction: None,
            });
        }

        let charge_result = self
            .services
            .charge_payment_method_directly(
                conn,
                tenant_id,
                payment_method_id,
                charge_amount,
                currency.clone(),
            )
            .await?;

        let payment_settled =
            charge_result.payment_intent.status == crate::domain::PaymentStatusEnum::Settled;

        if payment_settled {
            use crate::services::invoices::AdjustmentInvoiceContent;

            let content = AdjustmentInvoiceContent {
                computed: result.invoice_content,
                invoicing_entity: None,
            };

            let draft = self
                .services
                .create_adjustment_invoice_from_content(
                    conn,
                    &result.subscription,
                    &result.customer,
                    &result.proration,
                    content,
                )
                .await?;

            if let Some(invoice) = draft {
                self.services
                    .finalize_invoice_tx(conn, invoice.id, tenant_id, false, &None)
                    .await?;

                let _transaction = self
                    .services
                    .create_transaction_for_direct_charge(
                        conn,
                        tenant_id,
                        invoice.id,
                        &charge_result,
                        None,
                    )
                    .await?;

                diesel_models::invoices::InvoiceRow::apply_transaction(
                    conn,
                    invoice.id,
                    tenant_id,
                    charge_result.amount,
                )
                .await?;
                diesel_models::invoices::InvoiceRow::apply_payment_status(
                    conn,
                    invoice.id,
                    tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::Paid,
                    charge_result.payment_intent.processed_at,
                )
                .await?;
            }

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
                transaction: None,
            })
        } else {
            let transaction = self
                .services
                .create_transaction_for_checkout(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    &charge_result,
                )
                .await?;

            CheckoutSessionRow::mark_awaiting_payment(
                conn,
                tenant_id,
                checkout_session_id,
                Some(subscription_id),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            Ok(CheckoutCompletionResult::AwaitingPayment { transaction })
        }
    }

    /// Completes checkout for PlanChange type.
    /// The subscription already exists; we recompute proration, charge, and apply the plan change.
    async fn complete_checkout_plan_change_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let subscription_id = session.subscription_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Session has no linked subscription for plan change flow".to_string(),
            ))
        })?;

        let new_plan_version_id = session.plan_version_id;
        let change_date = session.change_date.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "PlanChange checkout session missing change_date".to_string(),
            ))
        })?;

        let prepared = self
            .services
            .prepare_plan_change_tx(
                conn,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                &[],
                change_date,
            )
            .await?;

        let is_free_trial = prepared.is_free_trial();
        let currency = prepared.subscription_details.subscription.currency.clone();

        if currency != currency_confirmation {
            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                "Currency mismatch: expected {}, got {}",
                currency, currency_confirmation
            )));
        }

        // Free trial: compute exact amount via virtual preview, charge,
        // then execute plan change + create invoice.
        if is_free_trial {
            let preview_details =
                prepared.build_trial_change_preview(change_date, new_plan_version_id)?;
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

            let charge_amount = invoice_content.total;

            let amount_diff = (charge_amount - total_amount_confirmation as i64).abs();
            if amount_diff > 1 {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "Amount mismatch: expected {}, got {}",
                    charge_amount, total_amount_confirmation
                )));
            }

            if charge_amount <= 0 {
                self.services
                    .execute_plan_change_tx(
                        conn,
                        &prepared,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;

                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    Utc::now(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                return Ok(CheckoutCompletionResult::Completed {
                    subscription_id,
                    transaction: None,
                });
            }

            let charge_result = self
                .services
                .charge_payment_method_directly(
                    conn,
                    tenant_id,
                    payment_method_id,
                    charge_amount,
                    currency,
                )
                .await?;

            let payment_settled =
                charge_result.payment_intent.status == crate::domain::PaymentStatusEnum::Settled;

            if payment_settled {
                // Execute plan change (trial→Active + components)
                self.services
                    .execute_plan_change_tx(
                        conn,
                        &prepared,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;

                // Create first invoice from real pipeline, linked to payment
                let subscription = self
                    .services
                    .store
                    .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                    .await?;
                let customer = diesel_models::customers::CustomerRow::find_by_id(
                    conn,
                    &subscription.subscription.customer_id,
                    &tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let customer: crate::domain::Customer = customer.try_into()?;

                let draft = self
                    .services
                    .create_subscription_draft_invoice(conn, tenant_id, &subscription, customer)
                    .await?;

                if let Some(invoice) = draft {
                    self.services
                        .finalize_invoice_tx(conn, invoice.id, tenant_id, false, &None)
                        .await?;

                    let _transaction = self
                        .services
                        .create_transaction_for_direct_charge(
                            conn,
                            tenant_id,
                            invoice.id,
                            &charge_result,
                            None,
                        )
                        .await?;

                    diesel_models::invoices::InvoiceRow::apply_transaction(
                        conn,
                        invoice.id,
                        tenant_id,
                        charge_result.amount,
                    )
                    .await?;
                    diesel_models::invoices::InvoiceRow::apply_payment_status(
                        conn,
                        invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Paid,
                        charge_result.payment_intent.processed_at,
                    )
                    .await?;
                }

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
                    transaction: None,
                })
            } else {
                // Pending (3DS, processing): record transaction against checkout session.
                // Settlement handler will apply plan change via on_checkout_payment_settled.
                let transaction = self
                    .services
                    .create_transaction_for_checkout(
                        conn,
                        tenant_id,
                        checkout_session_id,
                        &charge_result,
                    )
                    .await?;

                CheckoutSessionRow::mark_awaiting_payment(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    Some(subscription_id),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(CheckoutCompletionResult::AwaitingPayment { transaction })
            }
        } else {
            // Normal plan change: use proration
            let net_amount = prepared.proration.net_amount_cents;

            if net_amount > 0 {
                // Compute invoice content to determine amount_due after credits
                let content = self
                    .services
                    .compute_adjustment_invoice_content(
                        conn,
                        tenant_id,
                        &prepared.subscription_details.subscription,
                        &prepared.subscription_details.customer,
                        &prepared.proration,
                    )
                    .await?;

                let amount_due = content.computed.amount_due;

                let amount_diff = (amount_due - total_amount_confirmation as i64).abs();
                if amount_diff > 1 {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Amount mismatch: expected {}, got {}",
                        amount_due, total_amount_confirmation
                    )));
                }

                let invoice = self
                    .services
                    .create_adjustment_invoice_from_content(
                        conn,
                        &prepared.subscription_details.subscription,
                        &prepared.subscription_details.customer,
                        &prepared.proration,
                        content,
                    )
                    .await?
                    .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Expected adjustment invoice for positive proration".to_string(),
                        ))
                    })?;

                self.services
                    .finalize_invoice_tx(conn, invoice.id, tenant_id, false, &None)
                    .await?;

                if amount_due > 0 {
                    // Charge only the amount after credits
                    let charge_result = self
                        .services
                        .charge_payment_method_directly(
                            conn,
                            tenant_id,
                            payment_method_id,
                            amount_due,
                            currency,
                        )
                        .await?;

                    let payment_settled = charge_result.payment_intent.status
                        == crate::domain::PaymentStatusEnum::Settled;

                    let pending_pvid = if payment_settled {
                        None
                    } else {
                        Some(new_plan_version_id)
                    };

                    let transaction = self
                        .services
                        .create_transaction_for_direct_charge(
                            conn,
                            tenant_id,
                            invoice.id,
                            &charge_result,
                            pending_pvid,
                        )
                        .await?;

                    if payment_settled {
                        diesel_models::invoices::InvoiceRow::apply_transaction(
                            conn,
                            invoice.id,
                            tenant_id,
                            charge_result.amount,
                        )
                        .await?;
                        diesel_models::invoices::InvoiceRow::apply_payment_status(
                            conn,
                            invoice.id,
                            tenant_id,
                            diesel_models::enums::InvoicePaymentStatus::Paid,
                            charge_result.payment_intent.processed_at,
                        )
                        .await?;

                        self.services
                            .execute_plan_change_tx(
                                conn,
                                &prepared,
                                subscription_id,
                                tenant_id,
                                new_plan_version_id,
                                change_date,
                            )
                            .await?;

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
                            transaction: Some(transaction),
                        })
                    } else {
                        CheckoutSessionRow::mark_awaiting_payment(
                            conn,
                            tenant_id,
                            checkout_session_id,
                            Some(subscription_id),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        Ok(CheckoutCompletionResult::AwaitingPayment { transaction })
                    }
                } else {
                    // Credits cover the full upgrade cost — no payment needed
                    self.services
                        .execute_plan_change_tx(
                            conn,
                            &prepared,
                            subscription_id,
                            tenant_id,
                            new_plan_version_id,
                            change_date,
                        )
                        .await?;

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
                        transaction: None,
                    })
                }
            } else {
                // No charge needed (credit or zero) — apply immediately
                if net_amount != 0 {
                    let invoice = self
                        .services
                        .create_adjustment_invoice(
                            conn,
                            tenant_id,
                            &prepared.subscription_details.subscription,
                            &prepared.subscription_details.customer,
                            &prepared.proration,
                        )
                        .await?;
                    if let Some(inv) = &invoice {
                        self.services
                            .finalize_invoice_tx(conn, inv.id, tenant_id, false, &None)
                            .await?;
                    }
                }

                self.services
                    .execute_plan_change_tx(
                        conn,
                        &prepared,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;

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
                    transaction: None,
                })
            }
        }
    }
}

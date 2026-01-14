use crate::StoreResult;
use crate::domain::outbox_event::{
    InvoiceEvent, InvoicePdfGeneratedEvent, OutboxEvent, PaymentTransactionEvent,
    QuoteConvertedEvent,
};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    CreateSubscription, CreateSubscriptionFromQuote, CreatedSubscription, CustomerBuyCredits,
    DetailedInvoice, Invoice, QuoteActivityNew, SetupIntent, Subscription, SubscriptionDetails,
    UpdateInvoiceParams,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::InvoiceInterface;
use crate::repositories::outbox::OutboxInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::{InvoiceBillingMode, ServicesEdge};
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{
    AppliedCouponId, BaseId, CustomerConnectionId, CustomerPaymentMethodId, InvoiceId,
    SubscriptionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::coupons::CouponRow;
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

    pub async fn get_and_process_cycle_transitions(&self) -> StoreResult<usize> {
        self.services.get_and_process_cycle_transitions().await
    }

    pub async fn get_and_process_due_events(&self) -> StoreResult<usize> {
        self.services.get_and_process_due_events().await
    }

    pub async fn buy_customer_credits(
        &self,
        params: CustomerBuyCredits,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .buy_customer_credits(&mut self.get_conn().await?, params)
            .await
    }

    pub async fn complete_subscription_checkout(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<PaymentTransaction, StoreErrorReport> {
        let payment_transaction = self
            .store
            .transaction(|conn| {
                async move {
                    // Apply coupon if provided
                    if let Some(code) = coupon_code {
                        let subscription_row = SubscriptionRow::get_subscription_by_id(
                            conn,
                            &tenant_id,
                            subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // Look up coupon by code
                        let coupons =
                            CouponRow::list_by_codes(conn, tenant_id, std::slice::from_ref(&code))
                                .await?;

                        let coupon = coupons.into_iter().next().ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(format!(
                                "Coupon code '{}' not found",
                                code
                            )))
                        })?;

                        // Validate the coupon
                        let now = chrono::Utc::now().naive_utc();

                        if coupon.expires_at.is_some_and(|x| x <= now) {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Coupon '{}' has expired",
                                code
                            ))));
                        }

                        if coupon.archived_at.is_some() {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Coupon '{}' is no longer available",
                                code
                            ))));
                        }

                        if coupon.disabled {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Coupon '{}' is disabled",
                                code
                            ))));
                        }

                        if let Some(limit) = coupon.redemption_limit
                            && coupon.redemption_count >= limit
                        {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Coupon '{}' has reached its redemption limit",
                                code
                            ))));
                        }

                        // Check currency compatibility
                        let discount: crate::domain::coupons::CouponDiscount =
                            serde_json::from_value(coupon.discount.clone()).map_err(|_| {
                                Report::new(StoreError::InvalidArgument(
                                    "Invalid coupon discount format".to_string(),
                                ))
                            })?;

                        if discount
                            .currency()
                            .is_some_and(|c| c != subscription_row.subscription.currency)
                        {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Coupon '{}' currency does not match subscription currency",
                                code
                            ))));
                        }

                        // Check if non-reusable coupon was already used by this customer
                        if !coupon.reusable {
                            use diesel_models::applied_coupons::AppliedCouponRow;
                            let existing = AppliedCouponRow::find_existing_customer_coupon_pairs(
                                conn,
                                &[(coupon.id, subscription_row.subscription.customer_id)],
                            )
                            .await?;

                            if !existing.is_empty() {
                                return Err(Report::new(StoreError::InvalidArgument(format!(
                                    "Coupon '{}' has already been used",
                                    code
                                ))));
                            }
                        }

                        // Apply the coupon
                        let applied_coupon = AppliedCouponRowNew {
                            id: AppliedCouponId::new(),
                            subscription_id,
                            coupon_id: coupon.id,
                            customer_id: subscription_row.subscription.customer_id,
                            is_active: true,
                            applied_amount: None,
                            applied_count: None,
                            last_applied_at: None,
                        };

                        applied_coupon.insert(conn).await?;

                        // Update coupon redemption stats
                        CouponRow::update_last_redemption_at(
                            conn,
                            &[coupon.id],
                            chrono::Utc::now().naive_utc(),
                        )
                        .await?;
                        CouponRow::inc_redemption_count(conn, coupon.id, 1).await?;
                    }

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

                    Ok(payment_transaction)
                }
                .scope_boxed()
            })
            .await?;

        Ok(payment_transaction)
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

    /// Components and add-ons are already processed, so we skip plan-based processing.
    pub async fn insert_subscription_from_quote(
        &self,
        params: CreateSubscriptionFromQuote,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        let mut conn = self.get_conn().await?;
        let quote_id = params.quote_id;

        // Step 1: Gather minimal context (no price components/add-ons needed)
        let context = self
            .services
            .gather_subscription_context_from_quote(
                &mut conn,
                &params,
                tenant_id,
                &self.store.settings.crypt_key,
            )
            .await?;

        // Step 2: Build subscription details directly from pre-processed quote data
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

        // Step 3: Setup payment provider
        let payment_result = self
            .services
            .setup_payment_provider(&mut conn, &sub.subscription, &sub.customer, &context)
            .await?;

        // Step 4: Process subscription for insert (with quote_id linking)
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
                    // Step 5: Persist
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

                    // Atomically mark quote as converted (only if not already converted)
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

        // Step 6: Post-insertion tasks
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

        // Step 1: Gather all required data
        let context = self
            .services
            .gather_subscription_context(
                &mut conn,
                &batch,
                tenant_id,
                &self.store.settings.crypt_key,
            )
            .await?;

        // Step 2 : Prepare for internal usage, compute etc
        let subscriptions = self.services.build_subscription_details(&batch, &context)?;

        let mut results = Vec::new();
        for sub in subscriptions {
            // Step 3 : Connector stuff (create customer, create payment intent, bundle that for saving).
            // Not in the same transaction, it's fine if we have it already created in retry

            let result = self
                .services
                .setup_payment_provider(&mut conn, &sub.subscription, &sub.customer, &context)
                .await?;

            // Step 4 : Prepare for insert
            let processed = self
                .services
                .process_subscription(&sub, &result, &context, tenant_id, None)?;

            results.push(processed);
        }

        // Step 5 : Insert
        let inserted = self
            .services
            .persist_subscriptions(
                &mut conn,
                &results,
                tenant_id,
                &self.store.settings.jwt_secret,
                &self.store.settings.public_url,
            )
            .await?;

        // Step 4: Handle post-insertion tasks
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
}

use crate::StoreResult;
use crate::domain::outbox_event::{
    InvoiceEvent, InvoicePdfGeneratedEvent, PaymentTransactionEvent,
};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    CreateSubscription, CreatedSubscription, CustomerBuyCredits, DetailedInvoice, SetupIntent,
    Subscription, SubscriptionDetails,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::InvoiceInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::{InvoiceBillingMode, ServicesEdge};
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{
    CustomerConnectionId, CustomerPaymentMethodId, InvoiceId, SubscriptionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
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
    ) -> Result<PaymentTransaction, StoreErrorReport> {
        let payment_transaction = self
            .store
            .transaction(|conn| {
                async move {
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
                .process_subscription(&sub, &result, &context, tenant_id)?;

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
}

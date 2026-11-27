#![allow(clippy::too_many_arguments)]
use crate::StoreResult;
use crate::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutSessionStatus, CheckoutType,
};
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{BillingPeriodEnum, PaymentStatusEnum};
use crate::domain::outbox_event::{
    InvoiceEvent, InvoicePdfGeneratedEvent, OutboxEvent, PaymentTransactionEvent,
    QuoteConvertedEvent,
};
use crate::domain::payment_transactions::{PaymentNextAction, PaymentTransaction};
use crate::domain::subscription_changes::PlanTarget;
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{
    CheckoutSession, CreateSubscription, CreateSubscriptionFromQuote, CreatedSubscription,
    Customer, DetailedInvoice, Invoice, InvoicingEntityProviderSensitive, SetupIntent,
    Subscription, SubscriptionDetails, UpdateInvoiceParams,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::repositories::{InvoiceInterface, SubscriptionInterface};
use crate::services::checkout_completion::{
    CheckoutCharge, CheckoutChargeSent, InvoiceCheckoutCharge, PaymentCoupons, charge_needed,
};
use crate::services::clients::usage::WindowedUsageData;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::payment::process_payment::{InvoiceChargeResult, payment_in_flight};
use crate::services::subscriptions::payment_resolution::ResolvedPaymentMethods;
use crate::services::subscriptions::start::{StartTransition, mark_subscription_started_tx};
use crate::services::{InvoiceBillingMode, ServicesEdge};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::Datelike;
use chrono::NaiveDate;
use common_domain::ids::{
    AppliedCouponId, BaseId, CheckoutSessionId, CouponId, CreditNoteId, CustomerConnectionId,
    CustomerId, CustomerPaymentMethodId, InvoiceId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::SubscriptionStatusEnum as DbSubscriptionStatusEnum;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::plans::PlanRow;
use diesel_models::quotes::QuoteRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;

/// The customer's invoicing entity may require complete billing information before any
/// checkout completes; checked server-side whatever the checkout page showed.
async fn ensure_billing_information_if_required(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<()> {
    let required =
        InvoicingEntityRow::require_billing_information_of_customer(conn, tenant_id, customer_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if !required {
        return Ok(());
    }
    let customer: Customer = CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;
    if customer.has_complete_billing_information() {
        Ok(())
    } else {
        Err(Report::new(StoreError::InvalidArgument(
            "Billing information is required to complete this checkout: a billing email and a \
             complete billing address (address line 1, city, postal code and country)."
                .to_string(),
        )))
    }
}

/// One `initiate_hosted_checkout` pass: either a customer-facing result, or
/// an instruction to ADOPT the session's uncancelable prior intent (it must
/// be completed — never replaced or orphaned).
enum HostedCheckoutInitiation {
    Result(Box<CheckoutCompletionResult>),
    AdoptPriorIntent {
        connection_id: CustomerConnectionId,
        intent_id: String,
    },
}

impl ServicesEdge {
    async fn get_conn(&self) -> StoreResult<PgConn> {
        self.services.store.get_conn().await
    }

    /// The invoice of the stored subscription's period starting `invoice_date`, from its facts.
    pub async fn compute_period_invoice(
        &self,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.services
            .compute_invoice_from_facts(
                &mut self.get_conn().await?,
                invoice_date,
                subscription_details,
                None,
                None,
                crate::domain::enums::InvoiceType::Recurring,
                crate::services::invoice_lines::credit_units::CreditUnitCover::Estimate {
                    invoice_id: None,
                },
                None,
            )
            .await
    }

    /// The next invoice as `upcoming_invoice_forecast` estimates it.
    pub async fn compute_upcoming_invoice(
        &self,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        Ok(self
            .upcoming_invoice_forecast(subscription_details)
            .await?
            .content)
    }

    pub async fn preview_create_subscription(
        &self,
        create: &CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<(ComputedInvoiceContent, SubscriptionDetails)> {
        self.services
            .preview_create_subscription(&mut self.get_conn().await?, create, tenant_id)
            .await
    }

    /// Records a payment method that already exists at the provider of `connector_id` (imported
    /// with the customer from another billing system), its default when the customer has none.
    #[allow(clippy::too_many_arguments)]
    pub async fn adopt_existing_payment_method(
        &self,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        connector_id: common_domain::ids::ConnectorId,
        external_customer_id: String,
        external_payment_method_id: String,
        card_brand: Option<String>,
        card_last4: Option<String>,
    ) -> StoreResult<common_domain::ids::CustomerPaymentMethodId> {
        use crate::domain::customer_payment_methods::CustomerPaymentMethodNew;
        use crate::domain::customers::CustomerConnection;
        use crate::domain::enums::PaymentMethodTypeEnum;
        use crate::repositories::customer_connection::CustomerConnectionInterface;
        use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
        use common_domain::ids::{BaseId, CustomerConnectionId, CustomerPaymentMethodId};

        let connection = self
            .store
            .upsert_customer_connection(
                &tenant_id,
                CustomerConnection {
                    id: CustomerConnectionId::new(),
                    customer_id,
                    connector_id,
                    supported_payment_types: None,
                    external_customer_id,
                },
            )
            .await?;
        let method = self
            .store
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id,
                customer_id,
                connection_id: connection.id,
                external_payment_method_id,
                payment_method_type: PaymentMethodTypeEnum::Card,
                account_number_hint: None,
                card_brand,
                card_last4,
                card_exp_month: None,
                card_exp_year: None,
                fingerprint: None,
                saved_for_payment: false,
            })
            .await?;
        self.services
            .adopt_default_payment_method(
                Actor::System,
                tenant_id,
                customer_id,
                method.id,
                crate::services::payment::hosted_setup::DefaultMethodAdoption::IfNone,
            )
            .await?;
        Ok(method.id)
    }

    /// The recurring invoice of `period_start` that `create` would bill, nothing persisted,
    /// its usage taken from `usage` (one quantity per metric for the period).
    pub async fn preview_period_of_new_subscription(
        &self,
        create: &CreateSubscription,
        tenant_id: TenantId,
        period_start: chrono::NaiveDate,
        usage: &std::collections::HashMap<
            common_domain::ids::BillableMetricId,
            rust_decimal::Decimal,
        >,
    ) -> StoreResult<crate::services::evaluate::preview::VirtualPeriodInvoice> {
        let mut conn = self.get_conn().await?;
        let details = self
            .services
            .virtual_subscription_details(&mut conn, create, tenant_id)
            .await?;
        self.services
            .preview_period_invoice_of_details(&mut conn, &details, period_start, usage)
            .await
    }

    pub async fn get_subscription_component_usage(
        &self,
        subscription_details: &SubscriptionDetails,
        metric_id: common_domain::ids::BillableMetricId,
        period: crate::domain::Period,
    ) -> StoreResult<WindowedUsageData> {
        self.services
            .get_subscription_component_usage(subscription_details, metric_id, period)
            .await
    }

    pub async fn create_setup_intent(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        invoice_id: Option<InvoiceId>,
        preferred_type: Option<crate::domain::ConnectionTypeEnum>,
        return_url: Option<String>,
        descriptor_only: bool,
        for_payment: bool,
    ) -> StoreResult<SetupIntent> {
        self.services
            .create_setup_intent(
                tenant_id,
                customer_connection_id,
                invoice_id,
                preferred_type,
                return_url,
                descriptor_only,
                for_payment,
            )
            .await
    }

    pub async fn create_setup_intent_for_type(
        &self,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        connection_type: crate::domain::ConnectionTypeEnum,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        self.services
            .create_setup_intent_for_type(
                &mut self.get_conn().await?,
                tenant_id,
                customer_connection_id,
                connection_type,
                return_url,
            )
            .await
    }

    /// Explicit pay action for an in-flow hosted invoice payment (Stancer):
    /// pre-creates the Pending invoice tx, mints the capturing intent and
    /// returns the redirect in `client_secret`. Pay-click only, never on render.
    pub async fn initiate_hosted_invoice_payment(
        &self,
        tenant_id: TenantId,
        connection_id: CustomerConnectionId,
        invoice_id: InvoiceId,
        preferred_type: Option<crate::domain::ConnectionTypeEnum>,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        self.services
            .initiate_hosted_invoice_payment(
                tenant_id,
                connection_id,
                invoice_id,
                preferred_type,
                return_url,
            )
            .await
    }

    /// Re-reads the invoice's charge awaiting customer authentication (3DS / SCA) from the
    /// provider, secret included, so the portal can resume it.
    pub async fn resume_invoice_payment_authentication(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<crate::domain::payment_transactions::PaymentNextAction> {
        self.services
            .resume_invoice_payment_authentication(tenant_id, invoice_id)
            .await
    }

    /// Records a hosted-flow invoice capture on its pre-created transaction (idempotent).
    #[allow(clippy::too_many_arguments)]
    pub async fn settle_hosted_invoice_capture(
        &self,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        connector: &crate::domain::connectors::Connector,
        payment_method: crate::domain::CustomerPaymentMethod,
        invoice_id_str: &str,
        captured_payment_id: Option<String>,
        intent_transaction_id: Option<String>,
    ) -> StoreResult<crate::services::payment::hosted_setup::HostedSetupOutcome> {
        self.services
            .settle_hosted_invoice_capture(
                tenant_id,
                customer_id,
                connector,
                payment_method,
                invoice_id_str,
                captured_payment_id,
                intent_transaction_id,
            )
            .await
    }

    /// Complete a hosted-redirect return for any provider; see
    /// [`crate::services::payment::hosted_return`].
    pub async fn complete_hosted_return(
        &self,
        connection_id: CustomerConnectionId,
        intent_id: Option<String>,
    ) -> StoreResult<crate::services::HostedReturnOutcome> {
        self.services
            .complete_hosted_return(connection_id, intent_id)
            .await
    }

    /// Reconcile a single pending payment transaction against the provider.
    /// Used by the periodic reconciliation worker.
    #[tracing::instrument(skip_all, fields(tenant_id = %tenant_id, payment_transaction_id = %transaction_id))]
    pub async fn reconcile_pending_transaction(
        &self,
        transaction_id: common_domain::ids::PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .reconcile_pending_transaction(transaction_id, tenant_id)
            .await
    }

    /// Reconcile a single pending REFUND row against the provider. Second pass of the
    /// reconciliation worker's sweep.
    #[tracing::instrument(skip_all, fields(tenant_id = %tenant_id, payment_transaction_id = %refund_id))]
    pub async fn reconcile_pending_refund(
        &self,
        refund_id: common_domain::ids::PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .reconcile_pending_refund(refund_id, tenant_id)
            .await
    }

    /// Re-issue a refund whose provider call never came back. Third pass of the reconciliation
    /// worker's sweep.
    #[tracing::instrument(skip_all, fields(tenant_id = %tenant_id, payment_transaction_id = %refund_id))]
    pub async fn replay_unconfirmed_refund(
        &self,
        refund_id: common_domain::ids::PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .replay_unconfirmed_refund(refund_id, tenant_id)
            .await
    }

    /// List up to `limit` hosted payment attempts (checkout AND invoice)
    /// still carrying an in-flow pending intent, initiated before
    /// `older_than`, for the pending-intent sweeper.
    pub async fn list_pending_hosted_payments(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
        after: Option<(
            chrono::DateTime<chrono::Utc>,
            common_domain::ids::PaymentTransactionId,
        )>,
        limit: i64,
    ) -> StoreResult<Vec<crate::services::PendingHostedPaymentRef>> {
        self.services
            .list_pending_hosted_payments(older_than, after, limit)
            .await
    }

    /// Sweep one hosted payment attempt: re-run the return handler's
    /// completion (records a captured payment, never charges), closing out
    /// attempts abandoned before `abandoned_before`.
    #[tracing::instrument(skip_all, fields(
        tenant_id = %item.tenant_id,
        invoice_id = item.invoice_id.map(tracing::field::display),
        payment_transaction_id = %item.transaction_id,
    ))]
    pub async fn sweep_hosted_payment(
        &self,
        item: &crate::services::PendingHostedPaymentRef,
        abandoned_before: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<crate::services::HostedPaymentSweepOutcome> {
        self.services
            .sweep_hosted_payment(item, abandoned_before)
            .await
    }

    /// Claim up to `limit` Pending payments due for a provider poll; see
    /// `PaymentTransactionRow::claim_pending_for_reconcile` for the backoff.
    ///
    /// Returns lightweight projections to keep the worker decoupled from
    /// `diesel-models`.
    pub async fn claim_pending_payment_transactions(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<Vec<PendingTransactionRef>> {
        let mut conn = self.get_conn().await?;
        let rows = diesel_models::payments::PaymentTransactionRow::claim_pending_for_reconcile(
            &mut conn,
            diesel_models::enums::PaymentTypeEnum::Payment,
            older_than,
            limit,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(rows
            .into_iter()
            .map(|r| PendingTransactionRef {
                id: r.id,
                tenant_id: r.tenant_id,
                created_at: r.created_at,
            })
            .collect())
    }

    /// REFUND rows whose provider call never came back, so they never left `Ready`. Bounded on
    /// both sides: `no_older_than` is the point past which replaying is no longer safe.
    pub async fn list_unconfirmed_refund_transactions(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
        no_older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<Vec<PendingTransactionRef>> {
        let mut conn = self.get_conn().await?;
        let rows = diesel_models::payments::PaymentTransactionRow::list_stale_ready_refunds(
            &mut conn,
            older_than,
            no_older_than,
            limit,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(rows
            .into_iter()
            .map(|r| PendingTransactionRef {
                id: r.id,
                tenant_id: r.tenant_id,
                created_at: r.created_at,
            })
            .collect())
    }

    /// Same as [`Self::claim_pending_payment_transactions`], for REFUND rows the provider has
    /// not resolved yet.
    pub async fn claim_pending_refund_transactions(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<Vec<PendingTransactionRef>> {
        let mut conn = self.get_conn().await?;
        let rows = diesel_models::payments::PaymentTransactionRow::claim_pending_for_reconcile(
            &mut conn,
            diesel_models::enums::PaymentTypeEnum::Refund,
            older_than,
            limit,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(rows
            .into_iter()
            .map(|r| PendingTransactionRef {
                id: r.id,
                tenant_id: r.tenant_id,
                created_at: r.created_at,
            })
            .collect())
    }
}

/// How a checkout payment resolved, which decides what the customer gets now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutPaymentOutcome {
    /// Funds captured inline (card), or nothing to charge. Checkout is done.
    Settled,
    /// Delayed-notification rail: the mandate is signed and the debit submitted, so the
    /// sale is final and the subscription activates now, but the funds land days later.
    /// Checkout is done; the invoice carries `payment_status = Processing` until then.
    AcceptedInFlight,
    /// The customer still has to do something (card 3DS). Nothing is activated and the
    /// invoice stays draft until the webhook reports settlement.
    AwaitingCustomerAction,
}

impl ServicesEdge {
    /// Issues a checkout's invoice. A captured charge is linked first, so finalization draws only
    /// the credit it left, and is recorded whatever the invoice now asks (the excess is kept as
    /// overpayment credit); with no charge, credit must cover it whole. Paid when nothing is due.
    async fn issue_checkout_invoice_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        invoice_id: InvoiceId,
        charge: Option<&crate::services::checkout_completion::DirectChargeResult>,
        document: &str,
    ) -> StoreResult<()> {
        let charged = match charge {
            Some(charge) => Some(
                self.services
                    .create_transaction_for_direct_charge(
                        conn,
                        tenant_id,
                        invoice_id,
                        charge,
                        crate::services::DefaultMethodAdoption::Paid {
                            on_session: true,
                            started_at: common_clock::now(),
                        },
                    )
                    .await?,
            ),
            None => None,
        };

        self.services
            .finalize_invoice_tx(
                conn,
                &Actor::Customer { id: customer_id },
                invoice_id,
                tenant_id,
                false,
                &None,
            )
            .await?;

        self.record_checkout_invoice_tx(
            conn,
            tenant_id,
            customer_id,
            invoice_id,
            charged.as_ref(),
            document,
        )
        .await
    }

    /// Records a finalized checkout invoice against its captured charge, if any: what is left
    /// stays due, what is above it is kept as overpayment credit; with no charge, credit must
    /// have paid it whole. Paid when nothing is due, with its `invoice.paid` event.
    async fn record_checkout_invoice_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        invoice_id: InvoiceId,
        charged: Option<&PaymentTransaction>,
        document: &str,
    ) -> StoreResult<()> {
        let finalized_at =
            diesel_models::invoices::InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .finalized_at;
        let settled = self
            .services
            .settle_invoice_unfunded_tx(
                conn,
                tenant_id,
                invoice_id,
                charged
                    .and_then(|transaction| transaction.processed_at)
                    .or(finalized_at),
            )
            .await?;
        match charged {
            Some(transaction) => {
                crate::services::credit::collected_payment::reconcile_overpayment_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    invoice_id,
                    Some(crate::services::credit::collected_payment::Collection {
                        payment: transaction.id,
                        at: transaction
                            .processed_at
                            .map_or_else(common_clock::now, |at| at.and_utc()),
                    }),
                    crate::domain::wallet_events::CreditUnrecoveredCause::PaymentReversed,
                    None,
                )
                .await?;
            }
            None if settled.state.due != 0 => {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "{document} invoice {invoice_id} asks {} that credit no longer covers",
                    settled.state.due
                )));
            }
            None => {}
        }
        if settled.state.due > 0 {
            log::error!(
                "{document} invoice {invoice_id} left {} due after its checkout charge {}",
                settled.state.due,
                charged
                    .and_then(|transaction| transaction.provider_transaction_id.as_deref())
                    .unwrap_or("none")
            );
        } else if settled.paid_now {
            self.services
                .fund_paid_invoice_tx(conn, &settled.invoice, common_clock::now())
                .await?;
            self.store
                .internal
                .record_outbox_batch_tx(
                    conn,
                    tenant_id,
                    &Actor::Customer { id: customer_id },
                    vec![OutboxEvent::invoice_paid((&settled.invoice).into())],
                )
                .await?;
        }
        Ok(())
    }
}

impl ServicesEdge {
    /// A one-off invoice of `lines` as issuing it computes it: promotional credit estimated,
    /// taxed by the entity's engine.
    #[doc(hidden)]
    pub async fn compute_oneoff_invoice_for_test(
        &self,
        invoice_date: NaiveDate,
        lines: Vec<crate::domain::LineItem>,
        invoicing_entity: &crate::domain::InvoicingEntity,
        customer: &crate::domain::Customer,
        currency: String,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut conn = self.store.get_conn().await?;
        self.services
            .compute_oneoff_invoice(
                &mut conn,
                &invoice_date,
                lines,
                invoicing_entity,
                customer,
                currency,
                None,
                None,
                false,
            )
            .await
    }
}

/// Lightweight projection of a `PaymentTransactionRow` for the reconciliation
/// worker. Kept outside `diesel_models` so the worker (which lives in
/// `meteroid`, not `meteroid-store`) doesn't have to depend on the DB layer.
#[derive(Debug, Clone)]
pub struct PendingTransactionRef {
    pub id: common_domain::ids::PaymentTransactionId,
    pub tenant_id: TenantId,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ServicesEdge {
    pub async fn refresh_invoice_data(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        // A consolidated child must not be refreshed standalone (the parent is the canonical doc).
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;
        if invoice.is_consolidated_child() {
            return Err(crate::errors::StoreError::InvalidArgument(
                "Cannot refresh an invoice merged into a consolidated parent".to_string(),
            )
            .into());
        }

        // Same rule, and same reason, as editing a draft (see `update_draft_invoice_tx`): a
        // refresh moves `amount_due` just as an edit does, so running one under a live charge
        // desyncs the invoice from money the provider is already moving — on settlement it lands
        // PartiallyPaid or overpaid, and `FinalizeAfterPayment` asserts the amount it confirmed.
        // The background refresh excludes these in SQL; this covers the operator-driven path.
        let mut conn = self.get_conn().await?;
        let row = diesel_models::invoices::InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let account = crate::repositories::invoice_account::load_tx(&mut conn, &row).await?;
        if crate::repositories::invoice_account::owned_by_payment(&account.state()) {
            return Err(crate::errors::StoreError::InvalidArgument(
                "Cannot refresh an invoice with a payment in progress".to_string(),
            )
            .into());
        }

        self.services
            .refresh_draft_reading_usage_first(&mut conn, invoice_id, tenant_id)
            .await?;

        self.store
            .get_detailed_invoice_by_id(tenant_id, invoice_id)
            .await
    }

    /// Every customer's subscriptions stepped once, their invoices' deadlines left: a run's first
    /// step on its own. Returns how many customers took work.
    pub async fn run_subscription_steps(&self) -> StoreResult<usize> {
        self.services
            .run_steps_of_all(crate::services::lifecycle::controller::Steps::Subscriptions)
            .await
    }

    /// Every customer's invoice deadlines due taken once: drafts past their grace finalized,
    /// payment attempts made. Returns how many customers took work.
    pub async fn run_invoice_deadlines(&self) -> StoreResult<usize> {
        self.services
            .run_steps_of_all(crate::services::lifecycle::controller::Steps::Deadlines)
            .await
    }

    /// Runs the customers whose billing is due (see `lifecycle::controller`), up to `limit`.
    pub async fn run_billing_controller(
        &self,
        limit: i64,
    ) -> StoreResult<crate::services::lifecycle::controller::ControllerRun> {
        self.services.run_billing_controller(limit).await
    }

    /// As [`Self::run_billing_controller`], each subscription due stepped once: one boundary a
    /// run, for tests that walk a late history a boundary at a time.
    pub async fn run_billing_controller_one_step(
        &self,
        limit: i64,
    ) -> StoreResult<crate::services::lifecycle::controller::ControllerRun> {
        self.services.run_billing_controller_with(limit, 1).await
    }

    /// Decides the cadence origins the migration left pending, a run's worth; returns how many
    /// plan windows are left.
    pub async fn decide_pending_window_origins(&self) -> StoreResult<i64> {
        self.services.decide_pending_window_origins().await
    }

    /// Recomputes every customer's billing wake, counting the lost ones.
    pub async fn sweep_billing_controller(
        &self,
    ) -> StoreResult<crate::services::lifecycle::controller::ControllerSweep> {
        self.services.sweep_billing_controller().await
    }

    /// Completes the checkout process for a subscription.
    ///
    /// For free trials (trial_is_free = true):
    /// - Saves the payment method on the subscription
    /// - Activates the subscription to TrialActive status
    /// - Returns (None, false)
    ///
    /// For paid subscriptions or paid trials:
    /// - Creates and finalizes an invoice (or draft if the customer still has to act)
    /// - Processes payment
    /// - Returns the transaction (if any) and how the payment resolved
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
    ) -> Result<(Option<PaymentTransaction>, CheckoutPaymentOutcome), StoreErrorReport> {
        self.complete_subscription_checkout_charging_tx(
            conn,
            tenant_id,
            subscription_id,
            payment_method_id,
            total_amount_confirmation,
            currency_confirmation,
            coupon_code,
            None,
        )
        .await
    }

    /// [`Self::complete_subscription_checkout_tx`], charging in this transaction, or as a run of
    /// the checkout `checkout` names.
    #[allow(clippy::too_many_arguments)]
    async fn complete_subscription_checkout_charging_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
        checkout: Option<(CheckoutSessionId, &CheckoutCharge)>,
    ) -> Result<(Option<PaymentTransaction>, CheckoutPaymentOutcome), StoreErrorReport> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let mut confirmed_coupons = None;
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

            let trial_is_free =
                crate::services::subscriptions::recorded_trial_free_tx(conn, subscription.id)
                    .await?
                    .unwrap_or(plan_version.trial_is_free);
            let is_free_trial = subscription.trial_duration.is_some() && trial_is_free;
            let is_paid_trial = subscription.trial_duration.is_some() && !trial_is_free;

            // Handle coupon if provided - lock and validate before any charging
            if let Some(code) = coupon_code.filter(|code| !code.trim().is_empty()) {
                let coupon_id = self
                    .services
                    .checkout_coupon_by_code_tx(conn, tenant_id, &code)
                    .await?
                    .id;

                // Lock and validate coupon with FOR UPDATE to prevent race conditions
                let validated_coupons = self
                    .services
                    .lock_and_validate_coupons_for_checkout(
                        conn,
                        tenant_id,
                        subscription.customer_id,
                        &[coupon_id],
                        &subscription.currency,
                        Some(subscription.plan_version_id),
                        Some(subscription_id),
                        checkout.map(|(session_id, _)| session_id),
                    )
                    .await?;
                confirmed_coupons = Some(PaymentCoupons {
                    code: Some(code.clone()),
                    ids: vec![Some(coupon_id)],
                });

                // Apply the coupon (insert AppliedCoupon and update redemption count)
                if let Some(coupon) = validated_coupons.into_iter().next() {
                    use crate::services::subscriptions::utils::apply_coupons_without_validation;

                    let effective_from = crate::services::subscriptions::coupon_months_from_id_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                    )
                    .await?;
                    let applied_coupon = AppliedCouponRowNew {
                        id: AppliedCouponId::new(),
                        subscription_id,
                        coupon_id: coupon.id,
                        customer_id: subscription.customer_id,
                        is_active: true,
                        applied_amount: None,
                        applied_count: None,
                        last_applied_at: None,
                        effective_from,
                        discounts_from: None,
                    };

                    apply_coupons_without_validation(conn, &*self.store.hooks.credit, &[&applied_coupon], tenant_id).await?;
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
                // Nothing is charged now: the card the customer chose is what the trial bills.
                self.services
                    .adopt_default_payment_method_tx(
                        conn,
                        &Actor::Customer {
                            id: subscription.customer_id,
                        },
                        tenant_id,
                        subscription.customer_id,
                        payment_method_id,
                        crate::services::DefaultMethodAdoption::Replace,
                    )
                    .await?;

                // Keep existing period dates - payment method is resolved dynamically from the customer
                mark_subscription_started_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    subscription_id,
                    StartTransition::Activate {
                        current_period_start: subscription.current_period_start,
                        current_period_end: subscription.current_period_end,
                        status: DbSubscriptionStatusEnum::TrialActive,
                        by: crate::services::subscriptions::start::StartSettledBy::Checkout,
                    },
                )
                .await?;

                Ok((None, CheckoutPaymentOutcome::Settled))
            } else {
                let maybe_invoice = self
                    .services
                    .bill_subscription_tx(
                        conn,
                        &Actor::Customer {
                            id: subscription.customer_id,
                        },
                        tenant_id,
                        subscription_id,
                        InvoiceBillingMode::FinalizeAfterPayment {
                            currency_confirmation,
                            total_amount_confirmation,
                            payment_method_id,
                            charge: match checkout.map(|(_, charge)| charge) {
                                None => InvoiceCheckoutCharge::InTransaction,
                                Some(CheckoutCharge::Price) => {
                                    InvoiceCheckoutCharge::Price(confirmed_coupons)
                                }
                                Some(CheckoutCharge::Answered { result, .. }) => {
                                    InvoiceCheckoutCharge::Answered(result.clone())
                                }
                            },
                        },
                    )
                    .await?;

                // Resolve the payment transaction and the period anchor for activation. A `None`
                // invoice means there was nothing to bill at checkout — e.g. a usage-only plan (no
                // advance component) or a usage/arrears-scoped minimum, whose consumption (and floor
                // true-up) only settles at period end. Treat that like the self-serve zero-amount
                // path: no payment, just activate. The client must have confirmed a zero amount.
                let (payment_transaction, current_period_start) = match maybe_invoice {
                    Some(detailed_invoice) => {
                        // Billing finalizes the invoice when the charge settled inline (card) or
                        // when an async debit was accepted by the provider. Either way the sale is
                        // done and the customer gets access now; a still-draft invoice means the
                        // customer has an action outstanding (card 3DS).
                        let payment_accepted = !detailed_invoice.invoice.can_edit();
                        let period_start = detailed_invoice.invoice.billing_period_start;
                        let payment_transaction = detailed_invoice.transactions.into_iter().next();

                        // Handle payment status explicitly.
                        if let Some(ref txn) = payment_transaction {
                            match txn.status {
                                PaymentStatusEnum::Pending if !payment_accepted => {
                                    // Card 3DS: the customer still has to authenticate.
                                    // Activation happens via the webhook once it settles.
                                    return Ok((
                                        payment_transaction,
                                        CheckoutPaymentOutcome::AwaitingCustomerAction,
                                    ));
                                }
                                PaymentStatusEnum::Pending => {
                                    // Accepted async debit — activate below. The settle handler's
                                    // activation is a no-op then (guarded on `activated_at`).
                                }
                                PaymentStatusEnum::Settled => {
                                    // Payment succeeded, proceed with activation below.
                                }
                                PaymentStatusEnum::Failed => {
                                    // `error_type` holds the provider's decline message.
                                    let reason = txn
                                        .error_type
                                        .clone()
                                        .unwrap_or_else(|| "Payment failed".to_string());
                                    log::warn!(
                                        "Checkout payment declined for subscription {}: {} (transaction {}, provider transaction {:?}, payment method {:?})",
                                        subscription_id,
                                        reason,
                                        txn.id,
                                        txn.provider_transaction_id,
                                        txn.payment_method_id,
                                    );
                                    return Err(Report::new(StoreError::PaymentError(reason))
                                        .attach("Payment failed during checkout"));
                                }
                                PaymentStatusEnum::Cancelled => {
                                    return Err(Report::new(StoreError::CheckoutError)
                                        .attach("Payment was cancelled during checkout"));
                                }
                                PaymentStatusEnum::Ready => {
                                    // Ready means not yet processed - shouldn't happen after charging.
                                    return Err(Report::new(StoreError::CheckoutError)
                                        .attach("Payment was not processed correctly"));
                                }
                                PaymentStatusEnum::Refunded => {
                                    // A just-charged checkout payment can't already be reversed.
                                    return Err(Report::new(StoreError::CheckoutError)
                                        .attach("Payment was reversed during checkout"));
                                }
                            }
                        }

                        (payment_transaction, period_start)
                    }
                    None => {
                        if total_amount_confirmation != 0 {
                            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                            "Nothing to bill at checkout but a non-zero amount ({}) was confirmed",
                            total_amount_confirmation
                        )));
                        }
                        (None, subscription.current_period_start)
                    }
                };

                // Its renewals bill the card the customer completed it with. An accepted debit
                // still settling is adopted when it settles (its transaction names this checkout).
                if payment_transaction
                    .as_ref()
                    .is_none_or(|txn| txn.status == PaymentStatusEnum::Settled)
                {
                    self.services
                        .adopt_default_payment_method_tx(
                            conn,
                            &Actor::Customer {
                                id: subscription.customer_id,
                            },
                            tenant_id,
                            subscription.customer_id,
                            payment_method_id,
                            crate::services::DefaultMethodAdoption::Replace,
                        )
                        .await?;
                }

                // Payment succeeded (settled) or zero amount - activate the subscription
                // Calculate the billing period using the subscription's actual billing period
                let billing_period: BillingPeriodEnum = subscription.period.clone().into();
                let period = calculate_advance_period_range(
                    current_period_start,
                    subscription.billing_day_anchor as u32,
                    true, // First period
                    &billing_period,
                );
                let current_period_end = Some(period.end);

                // A paid trial is TrialActive for feature resolution; no trial or an expired one,
                // Active.
                let new_status = if is_paid_trial && !is_trial_expired {
                    DbSubscriptionStatusEnum::TrialActive
                } else {
                    DbSubscriptionStatusEnum::Active
                };

                mark_subscription_started_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    subscription_id,
                    StartTransition::Activate {
                        current_period_start,
                        current_period_end,
                        status: new_status,
                        by: crate::services::subscriptions::start::StartSettledBy::Checkout,
                    },
                )
                .await?;

                if !is_trial_expired {
                    self.services
                        .book_created_mrr_tx(conn, tenant_id, subscription_id)
                        .await?;
                }

                // Return transaction if any (None for zero amount case)
                let outcome = match payment_transaction {
                    Some(ref txn) if txn.status == PaymentStatusEnum::Pending => {
                        CheckoutPaymentOutcome::AcceptedInFlight
                    }
                    _ => CheckoutPaymentOutcome::Settled,
                };
                Ok((payment_transaction, outcome))
            }
        })
        .await
    }

    pub async fn complete_invoice_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        on_session: bool,
        // Stable provider-idempotency seed; see `charge_invoice`.
        // The webhook off-session charge passes one so a pgmq retry can't
        // double-charge; interactive callers pass None.
        idempotency_ref: Option<String>,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        match self
            .services
            .charge_invoice(
                tenant_id,
                invoice_id,
                payment_method_id,
                // Both edge callers (portal pay + mandate-setup
                // webhook charge) act on the customer's behalf.
                true,
                on_session,
                idempotency_ref,
                true,
            )
            .await?
        {
            InvoiceChargeResult::Charged(tx, next_action) => Ok((tx, next_action)),
            InvoiceChargeResult::InFlight(_) => Err(payment_in_flight()),
        }
    }

    /// The queued off-session charge (first attempt and dunning retries): nobody is present, so
    /// the attempt is not attributed to the customer and its failure can be emailed.
    /// A payment already in flight for the invoice is returned as is: there is nothing to retry.
    pub async fn complete_invoice_payment_automatic(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        idempotency_ref: Option<String>,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        match self
            .services
            .charge_invoice(
                tenant_id,
                invoice_id,
                payment_method_id,
                false,
                false,
                idempotency_ref,
                true,
            )
            .await?
        {
            InvoiceChargeResult::Charged(tx, next_action) => Ok((tx, next_action)),
            InvoiceChargeResult::InFlight(tx) => Ok((tx, None)),
        }
    }

    /// Re-send a charge (an invoice's or a checkout's) whose provider answer never came back.
    /// Fourth pass of the reconciliation worker's sweep.
    pub async fn resume_unsent_charge(
        &self,
        transaction_id: common_domain::ids::PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .resume_unsent_charge(transaction_id, tenant_id)
            .await
    }

    /// Claim up to `limit` committed charges with no provider answer, older than
    /// `older_than`; see `PaymentTransactionRow::claim_unsent_charges`.
    pub async fn claim_unsent_charges(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<Vec<PendingTransactionRef>> {
        let rows = self
            .store
            .transaction(|conn| {
                async move {
                    diesel_models::payments::PaymentTransactionRow::claim_unsent_charges(
                        conn, older_than, limit,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| PendingTransactionRef {
                id: r.id,
                tenant_id: r.tenant_id,
                created_at: r.created_at,
            })
            .collect())
    }

    pub async fn get_or_create_customer_connections(
        &self,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        invoicing_entity_id: common_domain::ids::InvoicingEntityId,
    ) -> StoreResult<(Option<CustomerConnectionId>, Option<CustomerConnectionId>)> {
        self.services
            .get_or_create_customer_connections(tenant_id, customer_id, invoicing_entity_id)
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

        let providers_row = {
            let mut conn = self.get_conn().await?;
            InvoicingEntityProvidersRow::resolve_providers_by_id(
                &mut conn,
                customer.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
        };

        let invoicing_entity_providers = InvoicingEntityProviderSensitive::from_row(
            providers_row,
            &self.store.settings.crypt_key,
        )?;

        self.services
            .resolve_payment_methods(
                tenant_id,
                payment_methods_config,
                customer,
                &invoicing_entity_providers,
            )
            .await
    }

    pub async fn insert_subscription(
        &self,
        actor: Actor,
        params: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch(actor, vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(Report::new(StoreError::InsertError))
            .attach("No subscription inserted")
    }

    pub async fn insert_subscription_tx(
        &self,
        actor: Actor,
        conn: &mut PgConn,
        params: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch_tx(actor, conn, vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(Report::new(StoreError::InsertError))
            .attach("No subscription inserted")
    }

    /// Components and add-ons are already processed, so we skip plan-based processing.
    pub async fn insert_subscription_from_quote(
        &self,
        actor: Actor,
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

        let payment_result = self.services.setup_payment_provider(&sub, &context)?;

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
                let actor = &actor;
                async move {
                    let mut inserted = self
                        .services
                        .persist_subscriptions(
                            conn,
                            actor,
                            &[processed],
                            tenant_id,
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

                    let quote_converted_event = QuoteConvertedEvent::new(
                        quote_id,
                        tenant_id,
                        inserted.customer_id,
                        inserted.id,
                    );
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::quote_converted(quote_converted_event)],
                        )
                        .await?;

                    Ok(inserted)
                }
                .scope_boxed()
            })
            .await?;

        self.services
            .handle_post_insertion(
                &actor,
                self.store.eventbus.clone(),
                std::slice::from_ref(&inserted),
            )
            .await?;

        Ok(inserted)
    }

    pub async fn insert_subscription_batch(
        &self,
        actor: Actor,
        batch: Vec<CreateSubscription>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let mut conn = self.get_conn().await?;

        self.insert_subscription_batch_tx(actor, &mut conn, batch, tenant_id)
            .await
    }

    pub async fn insert_subscription_batch_tx(
        &self,
        actor: Actor,
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
            let result = self.services.setup_payment_provider(&sub, &context)?;

            let processed = self
                .services
                .process_subscription(&sub, &result, &context, tenant_id, None)?;

            results.push(processed);
        }

        let inserted = self
            .services
            .persist_subscriptions(
                conn,
                &actor,
                &results,
                tenant_id,
                &self.store.settings.public_url,
            )
            .await?;

        self.services
            .handle_post_insertion(&actor, self.store.eventbus.clone(), &inserted)
            .await?;

        Ok(inserted)
    }

    pub async fn cancel_subscription(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<Subscription> {
        self.services
            .cancel_subscription(actor, subscription_id, tenant_id, reason, effective_at)
            .await
    }

    /// [`Self::cancel_subscription`] with the documents its ending issued when it applied in the
    /// request (API v2). Committed, it never fails: what could not follow is logged and retried
    /// by the controller (R-121, R-141).
    pub async fn cancel_subscription_with_documents(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<crate::domain::subscription_changes::CancellationOutcome> {
        self.services
            .cancel_subscription_outcome_at(
                actor,
                subscription_id,
                tenant_id,
                reason,
                effective_at,
                common_clock::now(),
            )
            .await
    }

    /// Moves a live subscription's end date, or clears it (`None`): the day it stops renewing and
    /// completes (R-140). Only what is not billed yet can move.
    pub async fn change_subscription_end_date(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        end_date: Option<NaiveDate>,
    ) -> StoreResult<Subscription> {
        self.services
            .change_subscription_end_date(actor, tenant_id, subscription_id, end_date)
            .await
    }

    /// API v2's PATCH of a subscription, in one transaction: its end date moved (R-140), v1's
    /// patch (`patch_subscription`) and its external id, checked free under the transaction, all
    /// written or none. A refusal carries the part it refuses (`SubscriptionUpdatePart`) when it is
    /// the end date's or the external id's; an external id another subscription holds is a
    /// `DuplicateValue`.
    pub async fn update_subscription(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        update: crate::domain::SubscriptionUpdate,
    ) -> StoreResult<()> {
        use crate::domain::SubscriptionUpdatePart;
        use diesel_models::query::catalog_codes;
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    // Customer first, as every transaction writing the subscription or its facts.
                    SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if let Some(Some(external_id)) = &update.external_id {
                        if external_id.trim().is_empty() || external_id.len() > 255 {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "an external id is 1 to 255 bytes".to_owned(),
                            ))
                            .attach_opaque(SubscriptionUpdatePart::ExternalId));
                        }
                        let holder = catalog_codes::subscription_id_by_external_id(
                            conn,
                            tenant_id,
                            external_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        if holder.is_some_and(|holder| holder != subscription_id) {
                            return Err(Report::new(StoreError::DuplicateValue {
                                entity: "subscription",
                                key: Some(external_id.clone()),
                            })
                            .attach_opaque(SubscriptionUpdatePart::ExternalId));
                        }
                    }
                    if let Some(end_date) = update.end_date {
                        self.services
                            .change_subscription_end_date_tx(
                                conn,
                                actor,
                                tenant_id,
                                subscription_id,
                                end_date,
                            )
                            .await
                            .map_err(|error| {
                                error.attach_opaque(SubscriptionUpdatePart::EndDate)
                            })?;
                    }
                    if let Some(patch) = update.patch {
                        self.store
                            .patch_subscription_tx(conn, tenant_id, patch, actor)
                            .await?;
                    }
                    if let Some(external_id) = update.external_id {
                        // Free when checked: only a write racing this one fails here, and with it
                        // the whole update.
                        let updated = catalog_codes::set_subscription_external_id(
                            conn,
                            tenant_id,
                            subscription_id,
                            external_id.as_deref(),
                        )
                        .await
                        .map_err(|error| {
                            Into::<Report<StoreError>>::into(error)
                                .attach_opaque(SubscriptionUpdatePart::ExternalId)
                        })?;
                        if updated == 0 {
                            return Err(Report::new(StoreError::ValueNotFound(format!(
                                "subscription {subscription_id}"
                            ))));
                        }
                    }
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// A cancellation requested at a past moment, as replays and models issue it: reached by
    /// then, it applies at once; later, at its date.
    pub async fn cancel_subscription_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        requested_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Subscription> {
        self.services
            .cancel_subscription_at(
                actor,
                subscription_id,
                tenant_id,
                reason,
                effective_at,
                requested_at,
            )
            .await
    }

    /// The invoice's payments, the retries its dunning policy still plans and what follows them.
    pub async fn invoice_collection(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<crate::domain::invoice_collection::InvoiceCollection> {
        self.services
            .invoice_collection(tenant_id, invoice_id)
            .await
    }

    pub async fn apply_coupon_to_subscription(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        coupon_id: CouponId,
    ) -> StoreResult<crate::domain::AppliedCouponDetailed> {
        self.services
            .apply_coupon_to_subscription(actor, tenant_id, subscription_id, coupon_id)
            .await
    }

    /// The day a coupon added now to the subscription would start discounting.
    pub async fn coupon_start(&self, details: &SubscriptionDetails) -> StoreResult<NaiveDate> {
        let mut conn = self.get_conn().await?;
        crate::services::subscriptions::coupon_apply::coupon_start(
            &mut conn,
            details,
            common_clock::now(),
        )
        .await
    }

    pub async fn schedule_plan_change(
        &self,
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: impl Into<PlanTarget>,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
    ) -> StoreResult<crate::domain::scheduled_events::ScheduledEvent> {
        self.services
            .schedule_plan_change(
                typed_actor,
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
        new_plan_version_id: impl Into<PlanTarget>,
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

    /// [`Self::preview_plan_change`] with what the change issues, the renewal under it and the
    /// subscription it leaves (API v2's previews).
    pub async fn preview_plan_change_in_full(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
        mode: Option<crate::domain::subscription_changes::PlanChangeMode>,
    ) -> StoreResult<crate::domain::subscription_changes::PlanChangePreviewFull> {
        self.services
            .preview_plan_change_in_full(subscription_id, tenant_id, target, component_params, mode)
            .await
    }

    /// The option of `version` a plan change naming none moves the subscription to, as the change
    /// picks it: the one of the alias it holds, else the one billing its fixed fees' cadence, else
    /// the first sold in its currency. None for a version without options.
    pub async fn plan_change_option(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        version: common_domain::ids::PlanVersionId,
    ) -> StoreResult<Option<crate::domain::plan_options::PlanOption>> {
        let mut conn = self.store.get_conn().await?;
        let details = self
            .store
            .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
            .await?;
        crate::services::subscriptions::plan_change::unnamed_target_option(
            &mut conn,
            &self.store,
            &details,
            version,
        )
        .await
    }

    /// What cancelling the subscription would issue, nothing written.
    pub async fn preview_cancellation(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<crate::domain::subscription_changes::CancellationPreview> {
        self.services
            .preview_cancellation(subscription_id, tenant_id, effective_at)
            .await
    }

    /// Compute the invoice content for a plan change checkout preview.
    /// Mirrors the exact amount computation from `complete_checkout_plan_change_tx`.
    pub async fn compute_plan_change_checkout_invoice(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        change_date: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.compute_plan_change_checkout_invoice_with(
            subscription_id,
            tenant_id,
            target,
            &[],
            change_date,
        )
        .await
    }

    /// `compute_plan_change_checkout_invoice` with the checkout's `component_params` (its levels).
    pub async fn compute_plan_change_checkout_invoice_with(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: &[crate::domain::subscription_components::ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        let target: PlanTarget = target.into();
        let new_plan_version_id = target.version;
        let mut conn = self.get_conn().await?;

        let prepared = self
            .services
            .prepare_plan_change_readonly(
                &mut conn,
                subscription_id,
                tenant_id,
                target,
                component_params,
                change_date,
            )
            .await?;
        prepared.ensure_immediate_allowed()?;
        // The old packs' settlement reads metering: each pass records what it was not served,
        // read between passes, outside its transaction.
        let mut passes = crate::services::served_usage::UsagePasses::default();
        loop {
            let usage = passes.next_pass()?;
            let quoted = self
                .services
                .reading_usage(usage.clone())
                .plan_change_quote(&mut conn, &prepared, new_plan_version_id, change_date)
                .await;
            if passes
                .served(self.services.usage_client.as_ref(), &usage)
                .await?
            {
                return quoted;
            }
        }
    }

    /// What buying the add-ons of an add-on purchase checkout bills, priced on `change_date` as
    /// the purchase prices it, and the add-ons it stores.
    pub async fn compute_addon_purchase_checkout_invoice(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        add_ons: &[crate::domain::subscription_add_ons::CreateSubscriptionAddOn],
        change_date: NaiveDate,
    ) -> StoreResult<(
        ComputedInvoiceContent,
        Vec<crate::domain::subscription_add_ons::SubscriptionAddOn>,
    )> {
        let mut conn = self.get_conn().await?;
        self.services
            .add_on_purchase_quote(&mut conn, tenant_id, subscription_id, add_ons, change_date)
            .await
    }

    /// The first invoice of the subscription `details` describe, not created yet: what its
    /// checkout charges.
    pub async fn compute_new_subscription_invoice(
        &self,
        details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut conn = self.get_conn().await?;
        self.services
            .preview_new_subscription_invoice(&mut conn, details)
            .await
    }

    pub async fn apply_plan_change_immediate(
        &self,
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: impl Into<PlanTarget>,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
    ) -> StoreResult<crate::domain::subscription_changes::ImmediatePlanChangeResult> {
        self.services
            .apply_plan_change_immediate(
                typed_actor,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
            )
            .await
    }

    pub async fn apply_plan_change_immediate_at(
        &self,
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: impl Into<PlanTarget>,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
        change_date: NaiveDate,
    ) -> StoreResult<crate::domain::subscription_changes::ImmediatePlanChangeResult> {
        self.services
            .apply_plan_change_immediate_at(
                typed_actor,
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
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .cancel_plan_change(typed_actor, subscription_id, tenant_id)
            .await
    }

    pub async fn cancel_scheduled_event(
        &self,
        typed_actor: Actor,
        event_id: common_domain::ids::ScheduledEventId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .cancel_scheduled_event(typed_actor, event_id, subscription_id, tenant_id)
            .await
    }

    pub async fn preview_amendment(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: crate::domain::subscription_amendment::SubscriptionAmendment,
    ) -> StoreResult<crate::domain::subscription_amendment::AmendmentPreviewExtended> {
        self.services
            .preview_amendment(subscription_id, tenant_id, amendment)
            .await
    }

    pub async fn apply_amendment_immediate(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: crate::domain::subscription_amendment::SubscriptionAmendment,
    ) -> StoreResult<crate::domain::subscription_amendment::ImmediateAmendmentResult> {
        self.services
            .apply_amendment_immediate(actor, subscription_id, tenant_id, amendment)
            .await
    }

    pub async fn apply_amendment_immediate_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: crate::domain::subscription_amendment::SubscriptionAmendment,
        change_date: NaiveDate,
    ) -> StoreResult<crate::domain::subscription_amendment::ImmediateAmendmentResult> {
        self.services
            .apply_amendment_immediate_at(actor, subscription_id, tenant_id, amendment, change_date)
            .await
    }

    pub async fn schedule_amendment(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        amendment: crate::domain::subscription_amendment::SubscriptionAmendment,
    ) -> StoreResult<crate::domain::scheduled_events::ScheduledEvent> {
        self.services
            .schedule_amendment(actor, subscription_id, tenant_id, amendment)
            .await
    }

    pub async fn cancel_amendment(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services
            .cancel_amendment(actor, subscription_id, tenant_id)
            .await
    }

    pub async fn list_capacity_levels(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<crate::domain::subscription_amendment::ComponentLevels>> {
        self.services
            .list_capacity_levels(tenant_id, subscription_id)
            .await
    }

    pub async fn preview_capacity_level_change(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        subscription_component_id: common_domain::ids::SubscriptionPriceComponentId,
        price_id: common_domain::ids::PriceId,
    ) -> StoreResult<crate::domain::subscription_amendment::LevelChangePreview> {
        self.services
            .preview_capacity_level_change(
                tenant_id,
                subscription_id,
                subscription_component_id,
                price_id,
            )
            .await
    }

    pub async fn change_capacity_level(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        subscription_component_id: common_domain::ids::SubscriptionPriceComponentId,
        price_id: common_domain::ids::PriceId,
    ) -> StoreResult<crate::domain::subscription_amendment::LevelChangeOutcome> {
        self.services
            .change_capacity_level(
                actor,
                tenant_id,
                subscription_id,
                subscription_component_id,
                price_id,
            )
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

    pub async fn fall_back_to_manual_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.services
            .fall_back_to_manual_payment(tenant_id, invoice_id)
            .await
    }

    /// A queued auto-charge dead-lettered: reminders take the invoice over.
    pub async fn hand_over_to_reminders(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.services
            .hand_over_to_reminders(tenant_id, invoice_id)
            .await
    }

    pub async fn on_invoice_paid(
        &self,
        event: InvoiceEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.services.on_invoice_paid(event, tenant_id).await
    }

    pub async fn credit_back_forgiven_late_packs(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<()> {
        self.services
            .credit_back_forgiven_late_packs(tenant_id, credit_note_id)
            .await
    }

    pub async fn credit_back_forgiven_late_packs_due(&self, limit: i64) -> StoreResult<usize> {
        self.services
            .credit_back_forgiven_late_packs_due(limit)
            .await
    }

    pub async fn on_payment_transaction_settled(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        self.services.on_payment_transaction_settled(event).await
    }

    pub async fn adopt_default_payment_method(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_id: CustomerId,
        payment_method_id: CustomerPaymentMethodId,
        adoption: crate::services::DefaultMethodAdoption,
    ) -> StoreResult<bool> {
        self.services
            .adopt_default_payment_method(
                actor,
                tenant_id,
                customer_id,
                payment_method_id,
                adoption,
            )
            .await
    }

    pub async fn preview_checkout_coupon(
        &self,
        tenant_id: TenantId,
        code: &str,
        subscription: &crate::domain::Subscription,
        add_on_ids: &[common_domain::ids::AddOnId],
    ) -> StoreResult<crate::domain::AppliedCouponDetailed> {
        self.services
            .preview_checkout_coupon(tenant_id, code, subscription, add_on_ids)
            .await
    }

    pub async fn on_payment_transaction_failed(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        self.services.on_payment_transaction_failed(event).await
    }

    pub async fn on_payment_transaction_reversed(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        self.services.on_payment_transaction_reversed(event).await
    }

    /// GoCardless `billing_requests.fulfilled` for a combined mandate+payment
    /// CHECKOUT Billing Request: bind the created payment to the Pending checkout
    /// transaction and materialize the subscription/invoice in-flight.
    pub async fn on_hosted_checkout_fulfilled(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        payment_method_id: CustomerPaymentMethodId,
        provider_payment_id: Option<String>,
        processed_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<()> {
        self.services
            .on_hosted_checkout_fulfilled(
                tenant_id,
                checkout_session_id,
                payment_method_id,
                provider_payment_id,
                processed_at,
            )
            .await
    }

    pub async fn finalize_invoice(
        &self,
        actor: Actor,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .finalize_invoice(actor, invoice_id, tenant_id)
            .await
    }

    /// A one-off invoice written by hand, priced as written (its discount spread, taxed by its
    /// lines' rates), ready to insert or to preview, with the invoicing entity issuing it.
    pub async fn manual_invoice_new(
        &self,
        tenant_id: TenantId,
        invoice: crate::services::ManualInvoice,
    ) -> StoreResult<(crate::domain::InvoiceNew, crate::domain::InvoicingEntity)> {
        self.services.manual_invoice_new(tenant_id, invoice).await
    }

    /// A one-off invoice written by hand, inserted as a draft: [`Self::manual_invoice_new`]
    /// inserted. It is issued by [`Self::finalize_invoice`], as every draft.
    pub async fn create_manual_invoice(
        &self,
        tenant_id: TenantId,
        invoice: crate::services::ManualInvoice,
    ) -> StoreResult<Invoice> {
        let (invoice, _) = self.services.manual_invoice_new(tenant_id, invoice).await?;
        self.store.insert_invoice(invoice).await
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

    pub async fn create_and_finalize_credit_note_with_reissue(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        params: crate::repositories::credit_notes::CreateCreditNoteParams,
        reissue_as_draft: bool,
        kind: crate::domain::enums::OperatorNoteKind,
    ) -> StoreResult<(crate::domain::CreditNote, Option<Invoice>)> {
        self.services
            .create_and_finalize_credit_note_with_reissue(
                actor,
                tenant_id,
                params,
                reissue_as_draft,
                kind,
            )
            .await
    }

    pub async fn regenerate_invoice_annexes(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .regenerate_invoice_annexes(tenant_id, invoice_id)
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
        target: impl Into<PlanTarget>,
        customer_id: common_domain::ids::CustomerId,
        payment_methods_config: Option<PaymentMethodsConfig>,
        change_date: NaiveDate,
    ) -> StoreResult<CheckoutSession> {
        self.create_plan_change_checkout_session_with(
            tenant_id,
            subscription_id,
            target,
            vec![],
            customer_id,
            payment_methods_config,
            change_date,
        )
        .await
    }

    /// `create_plan_change_checkout_session` moving with `component_params` (the levels picked):
    /// kept on the checkout, which previews and applies the change with them.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_plan_change_checkout_session_with(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        target: impl Into<PlanTarget>,
        component_params: Vec<crate::domain::subscription_components::ComponentParameterization>,
        customer_id: common_domain::ids::CustomerId,
        payment_methods_config: Option<PaymentMethodsConfig>,
        change_date: NaiveDate,
    ) -> StoreResult<CheckoutSession> {
        use crate::domain::checkout_sessions::{
            CheckoutType as DomainCheckoutType, CreateCheckoutSession,
        };
        use crate::repositories::checkout_sessions::CheckoutSessionsInterface;

        let target = target.into();
        let option = match target.option.as_ref() {
            Some(asked) => Some(
                crate::services::subscriptions::currency::option_alias(
                    &mut self.get_conn().await?,
                    tenant_id,
                    target.version,
                    asked,
                )
                .await?,
            ),
            None => None,
        };
        let session = CreateCheckoutSession {
            tenant_id,
            customer_id,
            plan_version_id: target.version,
            option,
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
            components: (!component_params.is_empty()).then(|| {
                crate::domain::subscription_components::CreateSubscriptionComponents {
                    parameterized_components: component_params,
                    overridden_components: vec![],
                    extra_components: vec![],
                    remove_components: vec![],
                }
            }),
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: DomainCheckoutType::PlanChange,
            subscription_id: Some(subscription_id),
            change_date: Some(change_date),
            success_url: None,
            cancel_url: None,
            resource_id: None,
        };

        self.store.create_checkout_session(session).await
    }

    /// Start a hosted-redirect checkout: a hosted mandate-setup flow + a
    /// Pending checkout tx, returning `AwaitingPayment` + a `RedirectToUrl`.
    /// WebhookBacked providers materialize via their webhook, PollingRequired
    /// ones via the return handler / pending-intent sweeper.
    pub async fn initiate_hosted_checkout(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        connection_id: common_domain::ids::CustomerConnectionId,
        amount_minor: i64,
        currency: String,
        coupon_code: Option<String>,
        rail: Option<crate::domain::ConnectionTypeEnum>,
        return_url: Option<String>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        use crate::domain::payment_transactions::PaymentNextAction;
        use crate::services::payment::hosted_setup::HostedSetupOutcome;
        use crate::services::payment::method::CancelPendingIntentOutcome;

        if amount_minor <= 0 {
            return Err(Report::new(StoreError::InvalidArgument(
                "Hosted checkout requires a positive first-payment amount".to_string(),
            )));
        }

        // Single-intent discipline: minting a replacement first cancels the
        // stored prior intent; an uncancelable one (payment underway/captured)
        // is ADOPTED through completion instead of replaced. At most two
        // passes: once more after an adoption resolves the prior attempt as dead.
        for adoption_attempt in 0..2u8 {
            let currency = currency.clone();
            let coupon_code = coupon_code.clone();
            let return_url = return_url.clone();

            let outcome = self
                .store
                .transaction(|conn| {
                async move {
                    let session: CheckoutSession = CheckoutSessionRow::get_by_id_for_update(
                        conn,
                        tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .1
                    .into();

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

                    ensure_billing_information_if_required(conn, tenant_id, session.customer_id)
                        .await?;

                    self.services
                        .ensure_connection_of_customer(
                            conn,
                            tenant_id,
                            session.customer_id,
                            connection_id,
                        )
                        .await?;
                    self.services
                        .ensure_coupon_code_accepted(conn, &session, coupon_code.as_deref())
                        .await?;

                    // One attempt: if a hosted attempt is already in flight (session
                    // AwaitingPayment with a live Pending checkout tx), return it and
                    // its stored redirect instead of minting a second Billing Request.
                    // The session row is locked, so concurrent clicks serialize.
                    if session.status == CheckoutSessionStatus::AwaitingPayment
                        && let Some((transaction, next_action)) = self
                            .services
                            .find_active_checkout_transaction(conn, tenant_id, checkout_session_id)
                            .await?
                    {
                        return Ok(HostedCheckoutInitiation::Result(Box::new(
                            CheckoutCompletionResult::AwaitingPayment {
                                transaction,
                                next_action,
                            },
                        )));
                    }

                    // At most ONE capturable intent per session: the prior
                    // stored intent is cancelled BEFORE a replacement is
                    // minted, so it can never capture money as an unwatched
                    // orphan; a not-cancelable one is adopted outside this
                    // transaction.
                    let prior_tx =
                        diesel_models::payments::PaymentTransactionRow::latest_with_pending_intent_by_checkout_session_id(
                            conn,
                            checkout_session_id,
                            tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if let Some(prior_tx) = prior_tx
                        && let Some(prior_intent) = prior_tx.pending_provider_intent_id.clone()
                    {
                        let prior_connection =
                            prior_tx.pending_connection_id.unwrap_or(connection_id);
                        match self
                            .services
                            .cancel_pending_hosted_intent(
                                conn,
                                &tenant_id,
                                &prior_connection,
                                &prior_intent,
                            )
                            .await?
                        {
                            CancelPendingIntentOutcome::Cancelled => {
                                // Dead at the provider: stop sweeping it.
                                diesel_models::payments::PaymentTransactionRow::clear_pending_intent_if_matches(
                                    conn,
                                    tenant_id,
                                    prior_tx.id,
                                    &prior_intent,
                                )
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                                log::info!(
                                    "cancelled superseded hosted-checkout intent {prior_intent} \
                                     for session {checkout_session_id}"
                                );
                            }
                            CancelPendingIntentOutcome::NotCancelable => {
                                return Ok(HostedCheckoutInitiation::AdoptPriorIntent {
                                    connection_id: prior_connection,
                                    intent_id: prior_intent,
                                });
                            }
                        }
                    }

                    // Persist the coupons this payment is priced with onto the
                    // session, so the deferred materialization on
                    // `billing_requests.fulfilled` rebuilds the subscription with
                    // the SAME coupons the amount was computed from. Without this a
                    // coupon applied only at the pay step (never persisted) would be
                    // dropped at fulfillment and the in-flight amount check would
                    // reject the webhook. Resolved the same way the settled path
                    // resolves them, so both phases agree.
                    let effective_coupon_code = coupon_code.or(session.coupon_code.clone());
                    let resolved_coupon_ids = self
                        .services
                        .resolve_coupon_ids_for_checkout_tx(
                            conn,
                            tenant_id,
                            &session,
                            effective_coupon_code.clone(),
                        )
                        .await?;
                    // Same gate as the synchronous confirm: the fulfilled webhook persists
                    // the subscription with coupon validation SKIPPED, so an expired /
                    // exhausted / wrong-currency coupon must be refused here, up front.
                    let _locked_coupons = self
                        .services
                        .lock_and_validate_coupons_for_checkout(
                            conn,
                            tenant_id,
                            session.customer_id,
                            &resolved_coupon_ids,
                            &currency,
                            session.plan_version_id,
                            None,
                            Some(checkout_session_id),
                        )
                        .await?;
                    CheckoutSessionRow::set_payment_coupons(
                        conn,
                        tenant_id,
                        checkout_session_id,
                        effective_coupon_code,
                        resolved_coupon_ids.into_iter().map(Some).collect(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let transaction_id = common_domain::ids::PaymentTransactionId::new();

                    // Combined mandate+payment Billing Request. The amount rides in
                    // the payment_request; the checkout session id in the BR/mandate
                    // metadata; our transaction id in the payment_request metadata so
                    // the payments.* webhooks resolve straight to this row.
                    let checkout_ctx = crate::adapters::payment::model::HostedCheckoutContext {
                        tenant_id: tenant_id.as_base62(),
                        checkout_session_id: checkout_session_id.as_base62(),
                        transaction_id: transaction_id.as_base62(),
                        amount_minor,
                        currency: currency.clone(),
                    };

                    let setup_intent = self
                        .services
                        .create_hosted_checkout_intent(
                            conn,
                            &tenant_id,
                            &connection_id,
                            checkout_ctx,
                            rail,
                            return_url,
                        )
                        .await?;

                    // Saved with the pending tx so the sweeper can recover a lost return (Stancer)
                    // or webhook (Mollie). GoCardless is never swept.
                    let polling_required =
                        crate::adapters::payment::payment_capabilities(&setup_intent.provider)
                            .is_some_and(|caps| caps.completes_pending_hosted_intents());

                    // For GoCardless the hosted authorisation URL rides in
                    // `client_secret`; drive the browser redirect through the same
                    // RedirectToUrl next_action the frontend already handles.
                    let next_action = PaymentNextAction::RedirectToUrl {
                        url: setup_intent.client_secret.clone(),
                    };
                    let next_action_json = serde_json::to_value(&next_action).ok();

                    // Anchor the hosted flow with a Pending checkout transaction.
                    // `provider_transaction_id` is left None until fulfillment sets
                    // it to the created payment id (so reconciliation doesn't probe a
                    // Billing Request id as if it were a payment). `next_action`
                    // persists the redirect so a re-click re-hydrates it.
                    let row = diesel_models::payments::PaymentTransactionRowNew {
                        id: transaction_id,
                        tenant_id,
                        invoice_id: None,
                        provider_transaction_id: None,
                        amount: amount_minor,
                        currency,
                        payment_method_id: None,
                        status: diesel_models::enums::PaymentStatusEnum::Pending,
                        payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: None,
                        checkout_session_id: Some(checkout_session_id),
                        pending_plan_version_id: None,
                        next_action: next_action_json,
                        initiated_by_customer_id: Some(session.customer_id),
                        pending_provider_intent_id: polling_required
                            .then(|| setup_intent.intent_id.clone()),
                        pending_connection_id: polling_required
                            .then_some(setup_intent.connection_id),
                        parent_transaction_id: None,
                        credit_note_id: None,
                        refund_mode: None,
                        reversal_reason: None,
                    };

                    let inserted = row
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    CheckoutSessionRow::mark_awaiting_payment(
                        conn,
                        tenant_id,
                        checkout_session_id,
                        session.subscription_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let mut transaction: crate::domain::PaymentTransaction = inserted.into();
                    transaction.next_action = Some(next_action.clone());

                    Ok(HostedCheckoutInitiation::Result(Box::new(
                        CheckoutCompletionResult::AwaitingPayment {
                            transaction,
                            next_action: Some(next_action),
                        },
                    )))
                }
                .scope_boxed()
            })
            .await?;

            let (prior_connection, prior_intent) = match outcome {
                HostedCheckoutInitiation::Result(result) => return Ok(*result),
                HostedCheckoutInitiation::AdoptPriorIntent {
                    connection_id,
                    intent_id,
                } => (connection_id, intent_id),
            };

            // Adoption: run the uncancelable prior intent through the SAME
            // completion routine (records, never charges) instead of minting
            // a second capturable intent.
            log::warn!(
                "hosted checkout session {checkout_session_id}: prior intent {prior_intent} is \
                 not cancelable; adopting it through completion instead of re-minting"
            );
            let setup_outcome = self
                .services
                .complete_hosted_setup_with_attempts(prior_connection, prior_intent.clone(), 1)
                .await?;

            match setup_outcome {
                HostedSetupOutcome::CheckoutActivated(_) => {
                    // Report the completed checkout rather than a new hosted page.
                    return self
                        .hosted_checkout_result_after_adoption(tenant_id, checkout_session_id)
                        .await;
                }
                HostedSetupOutcome::HeldForReview { .. } => {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A previous payment on this checkout requires manual review; please \
                         contact support before retrying."
                            .to_string(),
                    )));
                }
                HostedSetupOutcome::PaymentFailed { .. }
                | HostedSetupOutcome::SetupFailed { .. }
                    if adoption_attempt == 0 =>
                {
                    // Resolved as declined/dead: now cancelable — loop once to
                    // mint fresh.
                    continue;
                }
                HostedSetupOutcome::Processing => {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A payment for this checkout is still being processed; please retry \
                         shortly."
                            .to_string(),
                    )));
                }
                other => {
                    log::error!(
                        "hosted checkout session {checkout_session_id}: adopted intent \
                         {prior_intent} resolved as {other:?}; refusing to mint a replacement"
                    );
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Unable to start a new hosted payment for this checkout; please retry \
                         later or contact support."
                            .to_string(),
                    )));
                }
            }
        }

        // Unreachable: the second iteration always returns or errors above.
        Err(Report::new(StoreError::InvalidArgument(
            "Unable to start a hosted payment for this checkout; please retry later.".to_string(),
        )))
    }

    /// After an adopted prior intent activated the checkout, report the
    /// session's actual state instead of a fresh hosted page.
    async fn hosted_checkout_result_after_adoption(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let mut conn = self.get_conn().await?;
        let session: CheckoutSession =
            CheckoutSessionRow::get_by_id(&mut conn, tenant_id, checkout_session_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();

        if session.status == CheckoutSessionStatus::Completed
            && let Some(subscription_id) = session.subscription_id
        {
            let transaction =
                diesel_models::payments::PaymentTransactionRow::get_latest_by_checkout_session_id(
                    &mut conn,
                    checkout_session_id,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .map(Into::<PaymentTransaction>::into);
            return Ok(CheckoutCompletionResult::Completed {
                subscription_id,
                transaction,
            });
        }

        // Captured payment recorded, materialization still settling: money
        // already moved — never show a new page; have the customer refresh.
        Err(Report::new(StoreError::InvalidArgument(
            "A previous payment for this checkout was recovered and is being finalized; please \
             refresh the page."
                .to_string(),
        )))
    }

    /// Complete a checkout with a saved method: SelfServe charges first and only
    /// then creates the subscription; SubscriptionActivation activates the
    /// pre-created one. Both charge, then mark the session completed. A charge is
    /// made between two runs of the checkout's transaction (see [`CheckoutCharge`]).
    pub async fn complete_checkout(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let session: CheckoutSession = CheckoutSessionRow::get_by_id(
            &mut self.get_conn().await?,
            tenant_id,
            checkout_session_id,
        )
        .await?
        .into();
        if session.checkout_type == CheckoutType::TopUp {
            if session.can_complete() {
                ensure_billing_information_if_required(
                    &mut self.get_conn().await?,
                    tenant_id,
                    session.customer_id,
                )
                .await?;
            }
            return self
                .complete_top_up_checkout(
                    session,
                    payment_method_id,
                    total_amount_confirmation,
                    currency_confirmation,
                )
                .await;
        }
        // Read before the transaction locks the customer; the plan change is served these reads
        // under the lock and never reads metering there. One unreadable now fails it there.
        let plan_change_usage = match (
            session.checkout_type,
            session.subscription_id,
            session.plan_version_id,
            session.change_date,
        ) {
            (CheckoutType::PlanChange, Some(subscription_id), Some(plan_version_id), Some(at)) => {
                self.services
                    .plan_change_checkout_usage(
                        tenant_id,
                        subscription_id,
                        PlanTarget {
                            version: plan_version_id,
                            option: session.option_ref(),
                        },
                        &session.component_params(),
                        at,
                    )
                    .await
                    .unwrap_or_else(|error| {
                        log::warn!("plan change checkout usage not read ahead: {error:?}");
                        Some(Default::default())
                    })
            }
            _ => None,
        };
        let plan_change_usage = &plan_change_usage;
        let first = self
            .complete_checkout_run(
                tenant_id,
                checkout_session_id,
                payment_method_id,
                total_amount_confirmation,
                currency_confirmation.clone(),
                coupon_code.clone(),
                plan_change_usage,
                &CheckoutCharge::Price,
            )
            .await;
        let result = match first.as_ref().err().and_then(charge_needed) {
            None => first,
            Some(needed) => match self
                .services
                .send_checkout_charge(tenant_id, checkout_session_id, &needed)
                .await
            {
                Ok(CheckoutChargeSent::InFlight(transaction, next_action)) => {
                    Ok(CheckoutCompletionResult::AwaitingPayment {
                        transaction,
                        next_action,
                    })
                }
                Ok(CheckoutChargeSent::Answered(result)) => {
                    let charge = CheckoutCharge::answer(result);
                    let second = self
                        .complete_checkout_run(
                            tenant_id,
                            checkout_session_id,
                            payment_method_id,
                            total_amount_confirmation,
                            currency_confirmation.clone(),
                            coupon_code.clone(),
                            plan_change_usage,
                            &charge,
                        )
                        .await;
                    self.after_charged_run(tenant_id, checkout_session_id, &charge, second)
                        .await
                }
                Err(e) => Err(e),
            },
        };

        // Tx unwound → session lock released. Record a declined charge now (fresh
        // conn) so it survives the rollback and the failed-payment webhook resolves.
        // No-op unless the error carries the decline attachment.
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                self.services
                    .persist_declined_checkout_charge(tenant_id, checkout_session_id, &e)
                    .await;
                self.services
                    .cancel_checkout_after_repeated_declines(tenant_id, checkout_session_id, &e)
                    .await;
                return Err(e);
            }
        };

        Ok(self
            .activate_accepted_checkout_debit_in_flight(tenant_id, checkout_session_id, result)
            .await)
    }

    /// One run of a checkout's transaction (see [`CheckoutCharge`]).
    #[allow(clippy::too_many_arguments)]
    async fn complete_checkout_run(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
        plan_change_usage: &Option<crate::services::served_usage::UsageReads>,
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        // Metering is read before the session's locks: a pass that needs some reads it unlocked
        // and the next pass is served it (`served_usage`).
        let mut passes = crate::services::served_usage::UsagePasses::default();
        loop {
            let usage = passes.next_pass()?;
            let edge = ServicesEdge {
                store: self.store.clone(),
                services: crate::services::Services {
                    served: Some(usage.clone()),
                    ..self.services.reading_usage(usage.clone())
                },
            };
            charge.reset_used();
            let result = edge
                .complete_checkout_pass(
                    tenant_id,
                    checkout_session_id,
                    payment_method_id,
                    total_amount_confirmation,
                    currency_confirmation.clone(),
                    coupon_code.clone(),
                    plan_change_usage,
                    charge,
                )
                .await;
            if passes
                .served(self.services.usage_client.as_ref(), &usage)
                .await?
            {
                return result;
            }
        }
    }

    /// One pass of [`Self::complete_checkout_run`]'s transaction.
    #[allow(clippy::too_many_arguments)]
    async fn complete_checkout_pass(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
        plan_change_usage: &Option<crate::services::served_usage::UsageReads>,
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        self.store
            .transaction(|conn| {
                async move {
                    let (lock, session_row) = CheckoutSessionRow::get_by_id_for_update(
                        conn,
                        tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // Locked behind its customer: billing and credit take it later in this
                    // transaction, and the session's state is checked only under both.
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

                    ensure_billing_information_if_required(conn, tenant_id, session.customer_id)
                        .await?;

                    self.services
                        .ensure_payment_method_of_customer(
                            conn,
                            tenant_id,
                            session.customer_id,
                            payment_method_id,
                        )
                        .await?;
                    self.services
                        .ensure_coupon_code_accepted(conn, &session, coupon_code.as_deref())
                        .await?;

                    // Idempotency guard against a double charge. `can_complete()` allows
                    // re-completion from AwaitingPayment (retry after a failed attempt), but a
                    // charge still in flight (3DS `RequiresAction`→Pending is the common card
                    // case) means an existing non-terminal transaction is already at the
                    // provider. Re-charging would mint a fresh id → fresh `charge:{id}`
                    // idempotency key → a real second charge, later orphaned. Return the
                    // existing transaction (and its stored next_action) instead. The session
                    // row is locked above, so concurrent submits serialize here.
                    // The run answering a charge it committed goes on while that charge awaits
                    // its answer; recorded already (a webhook won), the settlement completes it.
                    if session.status == CheckoutSessionStatus::AwaitingPayment
                        && let Some((transaction, next_action)) = self
                            .services
                            .find_active_checkout_transaction(conn, tenant_id, checkout_session_id)
                            .await?
                    {
                        let ours = charge
                            .answered()
                            .is_some_and(|a| a.transaction_id == transaction.id);
                        let unrecorded = transaction.status == PaymentStatusEnum::Pending
                            && transaction.provider_transaction_id.is_none();
                        if !(ours && unrecorded) {
                            if ours {
                                charge.mark_used();
                            }
                            return Ok(CheckoutCompletionResult::AwaitingPayment {
                                transaction,
                                next_action,
                            });
                        }
                    }

                    let completed = match session.checkout_type {
                        CheckoutType::TopUp => {
                            unreachable!("handled before the subscription transaction")
                        }
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
                                charge,
                            )
                            .await
                        }
                        CheckoutType::SelfServe => {
                            // Self-serve checkouts are completed by the customer themselves.
                            let actor = Actor::Customer {
                                id: session.customer_id,
                            };
                            self.complete_checkout_self_serve_tx(
                                actor,
                                conn,
                                tenant_id,
                                checkout_session_id,
                                session,
                                payment_method_id,
                                total_amount_confirmation,
                                currency_confirmation,
                                coupon_code,
                                charge,
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
                                plan_change_usage.clone().unwrap_or_default(),
                                charge,
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
                                charge,
                            )
                            .await
                        }
                    }?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    // A pass that read metering it was not served never commits (see `served_usage`).
                    self.services.ensure_usage_served()?;
                    Ok(completed)
                }
                .scope_boxed()
            })
            .await
    }

    /// A second run that did not record the charge it answers (it failed, or no longer charges)
    /// leaves the answer to the settlement pipeline, which completes the checkout from its session.
    async fn after_charged_run(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        charge: &CheckoutCharge,
        run: Result<CheckoutCompletionResult, StoreErrorReport>,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        let Some(answer) = charge.answered() else {
            return run;
        };
        if charge.used() {
            return run;
        }
        match &run {
            Err(e) => log::error!(
                "Checkout session {checkout_session_id} could not record its charge {}: {e:?}",
                answer.transaction_id
            ),
            Ok(_) => log::error!(
                "Checkout session {checkout_session_id} completed without its charge {}",
                answer.transaction_id
            ),
        }
        let settled = self
            .services
            .settle_unrecorded_checkout_charge(tenant_id, answer)
            .await;
        match (run, settled) {
            (Ok(completed), _) => Ok(completed),
            (Err(_), Ok((transaction, next_action))) => {
                Ok(CheckoutCompletionResult::AwaitingPayment {
                    transaction,
                    next_action,
                })
            }
            (Err(e), Err(settle_error)) => {
                log::error!(
                    "Checkout charge {} left in flight for reconciliation: {settle_error:?}",
                    answer.transaction_id
                );
                Err(e)
            }
        }
    }

    /// Activate-at-acceptance for a checkout paid with an EXISTING direct-debit
    /// mandate (SelfServe / PlanChange / AddonPurchase charge-first paths). The
    /// charge came back Pending with no customer action, i.e. the provider accepted
    /// the debit — so, like the activation path and the hosted flow, materialize the
    /// subscription now (invoice `Processing`, tx linked) rather than only at
    /// settlement days later. Runs after the checkout tx committed: the
    /// materialization opens its own tx and needs the session lock released.
    ///
    /// Card is never affected (a pending card charge is either 3DS — `next_action`
    /// set — or not an accepted debit rail). On any failure the original
    /// `AwaitingPayment` is returned unchanged and the settlement webhook
    /// materializes it later, exactly as before.
    async fn activate_accepted_checkout_debit_in_flight(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        result: CheckoutCompletionResult,
    ) -> CheckoutCompletionResult {
        let CheckoutCompletionResult::AwaitingPayment {
            transaction,
            next_action: None,
        } = &result
        else {
            return result;
        };
        // Only the charge-first paths (tx not yet linked to an invoice); the
        // activation path decides acceptance inline, inside its own tx.
        // A charge with no answer back yet proves no acceptance.
        if transaction.invoice_id.is_some()
            || transaction.status != PaymentStatusEnum::Pending
            || transaction.provider_transaction_id.is_none()
        {
            return result;
        }
        let Some(payment_method_id) = transaction.payment_method_id else {
            return result;
        };

        let accepted = async {
            let mut conn = self.store.get_conn().await?;
            let method =
                diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                    &mut conn,
                    &tenant_id,
                    &payment_method_id,
                )
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;
            self.services
                .accepted_async_debit(&mut conn, tenant_id, transaction, &method)
                .await
        }
        .await;
        match accepted {
            Ok(true) => {}
            Ok(false) => return result,
            Err(e) => {
                log::warn!(
                    "Could not classify checkout charge {} for session {}; deferring to settlement: {e:?}",
                    transaction.id,
                    checkout_session_id
                );
                return result;
            }
        }

        let event: crate::domain::outbox_event::PaymentTransactionEvent =
            transaction.clone().into();
        if let Err(e) = self
            .services
            .on_checkout_payment_accepted_in_flight(event, checkout_session_id)
            .await
        {
            log::error!(
                "Accepted debit {} for checkout session {} could not be materialized in-flight; deferring to settlement: {e:?}",
                transaction.id,
                checkout_session_id
            );
            return result;
        }

        // Materialized: the session now carries the subscription it completed.
        let subscription_id = async {
            let mut conn = self.store.get_conn().await?;
            let session: CheckoutSession =
                CheckoutSessionRow::get_by_id(&mut conn, tenant_id, checkout_session_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into();
            Ok::<_, Report<StoreError>>(session.subscription_id)
        }
        .await;
        match subscription_id {
            Ok(Some(subscription_id)) => CheckoutCompletionResult::Completed {
                subscription_id,
                transaction: Some(transaction.clone()),
            },
            other => {
                log::warn!(
                    "Checkout session {} materialized in-flight but its subscription could not be read back ({other:?}); reporting AwaitingPayment",
                    checkout_session_id
                );
                result
            }
        }
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
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let subscription_id = session.subscription_id.ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Session has no linked subscription for activation flow".to_string(),
                ))
            })?;

            let coupon_for_checkout = coupon_code.or(session.coupon_code.clone());

            let (payment_transaction, outcome) = self
                .complete_subscription_checkout_charging_tx(
                    conn,
                    tenant_id,
                    subscription_id,
                    payment_method_id,
                    total_amount_confirmation,
                    currency_confirmation,
                    coupon_for_checkout,
                    Some((checkout_session_id, charge)),
                )
                .await?;
            if let (Some(answer), Some(transaction)) = (charge.answered(), &payment_transaction)
                && answer.transaction_id == transaction.id
            {
                charge.mark_used();
            }

            // Both non-settled outcomes leave a transaction whose fate is decided by a webhook.
            // This path's charge is linked to the invoice, so the transaction doesn't carry the
            // checkout_session_id. Tag it so a re-completion's idempotency guard
            // (complete_checkout) finds it and returns instead of re-charging. Keep the in-memory
            // `transaction` (with its transient next_action) for the response; only the persisted
            // row needs the tag.
            if outcome != CheckoutPaymentOutcome::Settled
                && let Some(ref transaction) = payment_transaction
                && transaction.checkout_session_id.is_none()
            {
                diesel_models::payments::PaymentTransactionRow::set_checkout_session_id(
                    conn,
                    transaction.id,
                    tenant_id,
                    checkout_session_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            }

            if outcome == CheckoutPaymentOutcome::AwaitingCustomerAction {
                CheckoutSessionRow::mark_awaiting_payment(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    Some(subscription_id),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let transaction = payment_transaction.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Pending payment must have transaction".to_string(),
                    ))
                })?;
                let next_action = transaction.next_action.clone();

                Ok(CheckoutCompletionResult::AwaitingPayment {
                    transaction,
                    next_action,
                })
            } else {
                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    common_clock::now(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(CheckoutCompletionResult::Completed {
                    subscription_id,
                    transaction: payment_transaction,
                })
            }
        })
        .await
    }

    /// Completes checkout for SelfServe type :
    /// 1. Build preview to compute amount
    /// 2. Charge customer (or validate payment method for free trials)
    /// 3. Only create subscription if payment succeeds (or is pending for async)
    /// 4. For pending payments: create transaction only, defer subscription/invoice
    /// 5. Mark session completed (sync) or awaiting payment (async)
    async fn complete_checkout_self_serve_tx(
        &self,
        actor: Actor,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        session: CheckoutSession,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
        coupon_code: Option<String>,
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
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
                .preview_new_subscription_invoice(conn, &preview_details)
                .await
                .change_context(StoreError::InvoiceComputationError)?;

            // Charged net of the customer's credit, as the checkout shows it: the invoice finalized
            // below draws that same credit (the customer stays locked in between).
            let amount_due = invoice_content.amount_due;
            let currency = preview_details.subscription.currency.clone();

            if currency != currency_confirmation {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "Currency mismatch: expected {}, got {}",
                    currency, currency_confirmation
                )));
            }

            let coupon_ids = self
                .services
                .resolve_coupon_ids_for_checkout_tx(
                    conn,
                    tenant_id,
                    &session,
                    effective_coupon_code.clone(),
                )
                .await?;

            let _locked_coupons = self
                .services
                .lock_and_validate_coupons_for_checkout(
                    conn,
                    tenant_id,
                    session.customer_id,
                    &coupon_ids,
                    &currency,
                    session.plan_version_id,
                    None,
                    Some(checkout_session_id),
                )
                .await?;
            // An asynchronous payment builds the subscription when it settles, from the session:
            // it must carry the coupons this payment was priced with, as the hosted path does.
            CheckoutSessionRow::set_payment_coupons(
                conn,
                tenant_id,
                checkout_session_id,
                effective_coupon_code.clone(),
                coupon_ids.iter().copied().map(Some).collect(),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

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
                // Nothing is charged now: the card the customer chose is what renewals bill.
                self.services
                    .adopt_default_payment_method_tx(
                        conn,
                        &actor,
                        tenant_id,
                        session.customer_id,
                        payment_method_id,
                        crate::services::DefaultMethodAdoption::Replace,
                    )
                    .await?;

                None
            } else {
                let amount_diff = (amount_due - total_amount_confirmation as i64).abs();
                if amount_diff > 1 {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Amount mismatch: expected {}, got {}",
                        amount_due, total_amount_confirmation
                    )));
                }

                let result = self.services.take_checkout_charge(
                    charge,
                    payment_method_id,
                    amount_due,
                    currency.clone(),
                    Some(PaymentCoupons {
                        code: effective_coupon_code.clone(),
                        ids: coupon_ids.iter().copied().map(Some).collect(),
                    }),
                )?;

                if result.payment_intent.status == PaymentStatusEnum::Pending {
                    let transaction = self
                        .services
                        .create_transaction_for_checkout(conn, tenant_id, &result)
                        .await?;

                    CheckoutSessionRow::mark_awaiting_payment(
                        conn,
                        tenant_id,
                        checkout_session_id,
                        None,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    return Ok(CheckoutCompletionResult::AwaitingPayment {
                        transaction,
                        next_action: result.payment_intent.next_action.clone(),
                    });
                }

                Some(result)
            };

            let start_date = session
                .billing_start_date
                .unwrap_or_else(|| common_clock::now().date_naive());

            let create_subscription = session.to_create_subscription(start_date, coupon_ids)?;

            let trial_config = preview_details.trial_config.clone();

            let created_subscription = self
                .insert_subscription_tx(actor.clone(), conn, create_subscription, tenant_id)
                .await?;

            let payment_transaction = if let Some(charge_result) = charge_result {
                let detailed_invoice = self
                    .services
                    .bill_subscription_tx(
                        conn,
                        &actor,
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

                let subscription = SubscriptionRow::get_subscription_by_id(
                    conn,
                    &tenant_id,
                    created_subscription.id,
                )
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

                let subscription = SubscriptionRow::get_subscription_by_id(
                    conn,
                    &tenant_id,
                    created_subscription.id,
                )
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
                // Zero amount: either a 100% coupon discount (invoice with line items + discount,
                // marked paid via Immediate mode) or a usage-only plan with nothing to bill at
                // activation (no invoice; first invoice will be produced at period end).
                self.services
                    .bill_subscription_tx(
                        conn,
                        &actor,
                        tenant_id,
                        created_subscription.id,
                        InvoiceBillingMode::Immediate,
                    )
                    .await?;

                let subscription = SubscriptionRow::get_subscription_by_id(
                    conn,
                    &tenant_id,
                    created_subscription.id,
                )
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
                mark_subscription_started_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    created_subscription.id,
                    StartTransition::Activate {
                        current_period_start: start_date,
                        current_period_end: None,
                        status: DbSubscriptionStatusEnum::Active,
                        by: crate::services::subscriptions::start::StartSettledBy::Checkout,
                    },
                )
                .await?;

                None
            };

            CheckoutSessionRow::mark_completed(
                conn,
                tenant_id,
                checkout_session_id,
                created_subscription.id,
                common_clock::now(),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            Ok(CheckoutCompletionResult::Completed {
                subscription_id: created_subscription.id,
                transaction: payment_transaction,
            })
        })
        .await
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
        payment_methods_config: Option<PaymentMethodsConfig>,
    ) -> StoreResult<CheckoutSession> {
        use crate::domain::checkout_sessions::{
            CheckoutType as DomainCheckoutType, CreateCheckoutSession,
        };
        use crate::domain::subscription_add_ons::{
            CreateSubscriptionAddOn, CreateSubscriptionAddOns, SubscriptionAddOnCustomization,
        };
        use crate::repositories::checkout_sessions::CheckoutSessionsInterface;

        let today = common_clock::now().date_naive();

        let session = CreateCheckoutSession {
            tenant_id,
            customer_id,
            plan_version_id,
            option: None,
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
            success_url: None,
            cancel_url: None,
            resource_id: None,
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
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
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

            // Bought the day it is paid, as the quote prices it. The rows are written once the
            // charge settles: an accepted one leaves them to its settlement.
            let addon_effective_from = common_clock::now().date_naive();
            let rows = crate::repositories::subscription_add_ons::checkout_add_on_rows(
                conn,
                tenant_id,
                subscription_id,
                &create_add_ons.add_ons,
                addon_effective_from,
            )
            .await?;
            let result = self
                .services
                .compute_addon_purchase_invoice(
                    conn,
                    tenant_id,
                    subscription_id,
                    &rows,
                    addon_effective_from,
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

            // Credit covers the add-on: its invoice is still issued, the credit drawn and the
            // invoice paid by it, as when part of it is charged.
            if charge_amount <= 0 {
                let draft = self
                    .services
                    .record_addon_purchase_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        &rows,
                        addon_effective_from,
                        result,
                    )
                    .await?;
                if let Some(invoice) = draft {
                    self.issue_checkout_invoice_tx(
                        conn,
                        tenant_id,
                        session.customer_id,
                        invoice.id,
                        None,
                        "Add-on",
                    )
                    .await?;
                }

                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    common_clock::now(),
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
                .take_checkout_charge(
                    charge,
                    payment_method_id,
                    charge_amount,
                    currency.clone(),
                    None,
                )?;

            let payment_settled =
                charge_result.payment_intent.status == crate::domain::PaymentStatusEnum::Settled;

            if payment_settled {
                let draft = self
                    .services
                    .record_addon_purchase_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        &rows,
                        addon_effective_from,
                        result,
                    )
                    .await?;

                let invoice = draft.ok_or_else(|| {
                    Report::new(StoreError::CheckoutError).attach(format!(
                        "Settled charge {} for subscription {subscription_id} has no add-on invoice to record against",
                        charge_result.payment_intent.external_id
                    ))
                })?;
                self.issue_checkout_invoice_tx(
                    conn,
                    tenant_id,
                    session.customer_id,
                    invoice.id,
                    Some(&charge_result),
                    "Add-on",
                )
                .await?;

                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    common_clock::now(),
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

                Ok(CheckoutCompletionResult::AwaitingPayment {
                    transaction,
                    next_action: charge_result.payment_intent.next_action.clone(),
                })
            }
        })
        .await
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
        usage: crate::services::served_usage::UsageReads,
        charge: &CheckoutCharge,
    ) -> Result<CheckoutCompletionResult, StoreErrorReport> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
        let subscription_id = session.subscription_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "Session has no linked subscription for plan change flow".to_string(),
            ))
        })?;

        let new_plan_version_id = session.subscription_plan_version_id()?;
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
                session.plan_target()?,
                &session.component_params(),
                change_date,
            )
            .await?;
        crate::services::subscriptions::plan_change::refuse_while_a_paid_change_is_pending(
            conn,
            tenant_id,
            subscription_id,
            Some(checkout_session_id),
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

        // Free trial: the new plan's first invoice, priced from the facts with the change's rows
        // laid over them; charged, then the plan change is executed and the invoice created.
        if is_free_trial {
            let invoice_content = self
                .services
                .plan_change_quote(conn, &prepared, new_plan_version_id, change_date)
                .await?;

            let charge_amount = invoice_content.amount_due;

            let amount_diff = (charge_amount - total_amount_confirmation as i64).abs();
            if amount_diff > 1 {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "Amount mismatch: expected {}, got {}",
                    charge_amount, total_amount_confirmation
                )));
            }

            if invoice_content.total <= 0 {
                self.services
                    .execute_plan_change_tx(
                        conn,
                        &Actor::Customer {
                            id: session.customer_id,
                        },
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
                    common_clock::now(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                return Ok(CheckoutCompletionResult::Completed {
                    subscription_id,
                    transaction: None,
                });
            }

            // The capture is irreversible from this transaction's point of view — the PSP keeps
            // the money even when we roll back — so anything that would make the post-charge
            // recording fail must be checked BEFORE charging. A prior run may have already
            // settled the period this change bills; without this check the customer's retry
            // captures again on every click, each time with nowhere to record the charge. The
            // post-charge guard below stays as the last line of defense.
            if let Some(existing) =
                diesel_models::invoices::InvoiceRow::find_existing_recurring_invoice(
                    conn,
                    tenant_id,
                    subscription_id,
                    change_date,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
            {
                let existing: crate::domain::Invoice = existing.try_into()?;
                if existing.status == crate::domain::InvoiceStatusEnum::Void {
                    return Err(Report::new(StoreError::InvalidArgument(
                        crate::services::invoices::bill::VOIDED_PERIOD_CHECKOUT.to_string(),
                    )));
                }
                if !existing.can_edit() {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Period starting {change_date} is already settled by invoice {} in terminal status {:?}; refusing to charge for this plan change",
                        existing.id, existing.status
                    )));
                }
            }

            // Credit covering the whole amount leaves nothing to charge: the invoice draws it.
            let charge_result = if charge_amount > 0 {
                Some(
                    self.services
                        .take_checkout_charge(charge, payment_method_id, charge_amount, currency, None)?,
                )
            } else {
                None
            };

            let pending_charge = charge_result.as_ref().filter(|charge| {
                charge.payment_intent.status != crate::domain::PaymentStatusEnum::Settled
            });

            if let Some(charge_result) = pending_charge {
                // Pending (3DS, processing): record transaction against checkout session.
                // Settlement handler will apply plan change via on_checkout_payment_settled.
                let transaction = self
                    .services
                    .create_transaction_for_checkout(
                        conn,
                        tenant_id,
                        charge_result,
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

                Ok(CheckoutCompletionResult::AwaitingPayment {
                    transaction,
                    next_action: charge_result.payment_intent.next_action.clone(),
                })
            } else {
                // Execute plan change (trial→Active + components)
                self.services
                    .execute_plan_change_tx(
                        conn,
                        &Actor::Customer {
                            id: session.customer_id,
                        },
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
                    .create_subscription_draft_invoice(
                        conn,
                        tenant_id,
                        &subscription,
                        customer,
                        crate::services::invoices::PeriodBilling::Bill,
                    )
                    .await?;

                // The charge above already settled at the PSP, so this block is what records it.
                // create_subscription_draft_invoice can return an invoice a prior run already
                // settled (Finalized or Closed); skipping the recording would complete the checkout
                // with the money captured and no transaction against any invoice. Fail loudly so it
                // is reconciled instead.
                // Carry the PSP reference: when either arm fires the money is captured and nothing
                // is in the DB, so the intent id is what an operator reconciles or refunds from.
                let charge_ref = charge_result
                    .as_ref()
                    .map_or("none", |charge| charge.payment_intent.external_id.as_str());
                let invoice = draft.ok_or_else(|| {
                    Report::new(StoreError::CheckoutError).attach(format!(
                        "Settled charge {charge_ref} for subscription {subscription_id} has no invoice to record against"
                    ))
                })?;
                if !invoice.can_edit() {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Settled charge {charge_ref} for subscription {subscription_id} cannot be recorded: invoice {} is already in terminal status {:?}",
                        invoice.id, invoice.status
                    )));
                }

                self.issue_checkout_invoice_tx(
                    conn,
                    tenant_id,
                    session.customer_id,
                    invoice.id,
                    charge_result.as_ref(),
                    "Plan change",
                )
                .await?;

                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    common_clock::now(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(CheckoutCompletionResult::Completed {
                    subscription_id,
                    transaction: None,
                })
            }
        } else {
            // Normal plan change. Priced as the change issues it, from the facts with its rows
            // laid over them: nothing is applied, granted or credited before the payment settles.
            let actor = Actor::Customer {
                id: session.customer_id,
            };
            let served = self.services.served_only(usage);
            let quote = served
                .plan_change_quote(conn, &prepared, new_plan_version_id, change_date)
                .await?;
            let amount_due = quote.amount_due;
            if (amount_due - total_amount_confirmation as i64).abs() > 1 {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "Amount mismatch: expected {}, got {}",
                    amount_due, total_amount_confirmation
                )));
            }

            if amount_due <= 0 {
                let (invoice, _) = served
                    .apply_and_invoice_plan_change_tx(
                        conn,
                        &actor,
                        &prepared,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;
                if let Some(invoice) = invoice {
                    self.record_checkout_invoice_tx(
                        conn,
                        tenant_id,
                        session.customer_id,
                        invoice.id,
                        None,
                        "Plan change",
                    )
                    .await?;
                }
                CheckoutSessionRow::mark_completed(
                    conn,
                    tenant_id,
                    checkout_session_id,
                    subscription_id,
                    common_clock::now(),
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
                .take_checkout_charge(charge, payment_method_id, amount_due, currency, None)?;
            if charge_result.payment_intent.status != crate::domain::PaymentStatusEnum::Settled {
                // Applied when the payment settles (`on_checkout_payment_settled`); a retried or
                // reloaded checkout finds this pending payment through its session.
                let transaction = self
                    .services
                    .create_transaction_for_checkout(
                        conn,
                        tenant_id,
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
                return Ok(CheckoutCompletionResult::AwaitingPayment {
                    transaction,
                    next_action: charge_result.payment_intent.next_action.clone(),
                });
            }

            let invoice = served
                .apply_and_invoice_plan_change_tx(
                    conn,
                    &actor,
                    &prepared,
                    new_plan_version_id,
                    change_date,
                )
                .await?
                .0
                .ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Expected an adjustment invoice for a charged plan change".to_string(),
                    ))
                })?;
            let transaction = self
                .services
                .create_transaction_for_direct_charge(
                    conn,
                    tenant_id,
                    invoice.id,
                    &charge_result,
                    crate::services::DefaultMethodAdoption::Paid {
                            on_session: true,
                            started_at: common_clock::now(),
                        },
                )
                .await?;
            self.record_checkout_invoice_tx(
                conn,
                tenant_id,
                session.customer_id,
                invoice.id,
                Some(&transaction),
                "Plan change",
            )
            .await?;
            CheckoutSessionRow::mark_completed(
                conn,
                tenant_id,
                checkout_session_id,
                subscription_id,
                common_clock::now(),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            Ok(CheckoutCompletionResult::Completed {
                subscription_id,
                transaction: Some(transaction),
            })
        }
    }).await
    }
}

use crate::domain::add_ons::AddOn;
use crate::domain::subscription_add_ons::CreateSubscriptionAddOn;
use crate::domain::subscription_changes::{ProrationLineItem, ProrationResult};
use crate::domain::{
    Customer, InlineCustomer, InlineInvoicingEntity, Invoice, InvoiceNew, InvoicePaymentStatus,
    InvoiceStatusEnum, InvoiceType, LineItem, Price, Product, Subscription, SubscriptionDetails,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_balance::convert_currency;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use crate::services::Services;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveTime, Utc};
use common_domain::ids::{
    BaseId, PriceId, ProductId, SubscriptionAddOnId, SubscriptionId, TenantId,
};
use diesel_models::invoices::InvoiceRow;
use error_stack::{Report, ResultExt};
use rust_decimal::Decimal;
use rust_decimal::prelude::Zero;
use std::collections::HashMap;

impl Services {
    pub(in crate::services) async fn create_subscription_draft_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: &SubscriptionDetails,
        customer: Customer,
    ) -> Result<Option<Invoice>, StoreErrorReport> {
        let subscription = &subscription_details.subscription;

        let invoice_date = subscription.current_period_start;

        // Check if a recurring invoice already exists for this subscription and date.
        // This prevents duplicate invoice creation (e.g., when cancelling a subscription
        // that was already billed for the current period).
        if let Some(existing_invoice) = InvoiceRow::find_existing_recurring_invoice(
            conn,
            tenant_id,
            subscription.id,
            invoice_date,
        )
        .await?
        {
            return Ok(Some(existing_invoice.try_into()?));
        }

        // Compute invoice lines for the period
        let invoice_content = self
            .compute_invoice(conn, &invoice_date, subscription_details, None, None)
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        if invoice_content.invoice_lines.is_empty() {
            log::info!(
                "No invoice lines computed for subscription {}. Skipping draft invoice creation.",
                subscription.id
            );
            return Ok(None);
        }

        let due_date = (invoice_date + chrono::Duration::days(i64::from(subscription.net_terms)))
            .and_time(NaiveTime::MIN);

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        // Draft invoice uses "draft" as invoice number
        let invoice_number = "draft";

        let invoice_new = InvoiceNew {
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: Some(subscription.id),
            plan_version_id: Some(subscription.plan_version_id),
            invoice_type: InvoiceType::Recurring,
            currency: subscription.currency.clone(),
            line_items: invoice_content.invoice_lines,
            coupons: invoice_content.applied_coupons,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date,
            finalized_at: None,
            total: invoice_content.total,
            amount_due: invoice_content.amount_due,
            applied_credits: invoice_content.applied_credits,
            net_terms: subscription.net_terms as i32,
            subtotal: invoice_content.subtotal,
            subtotal_recurring: invoice_content.subtotal_recurring,
            reference: None,
            purchase_order: subscription.purchase_order.clone(),
            memo: subscription.invoice_memo.clone(),
            due_at: Some(due_date),
            plan_name: Some(subscription.plan_name.clone()),
            invoice_number: invoice_number.to_string(),
            customer_details: customer.into(),
            seller_details: invoicing_entity.into(),
            auto_advance: subscription.auto_advance_invoices,
            payment_status: InvoicePaymentStatus::Unpaid,
            discount: invoice_content.discount,
            tax_breakdown: invoice_content.tax_breakdown,
            tax_amount: invoice_content.tax_amount,
            manual: false,
            invoicing_entity_id: subscription.invoicing_entity_id,
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn create_oneoff_draft_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_date: NaiveDate,
        invoice_lines: Vec<LineItem>,
        customer: &Customer,
        currency: String,
        discount: Option<u64>,
        prepaid_amount: Option<u64>,
    ) -> Result<Option<Invoice>, StoreErrorReport> {
        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let invoice_content = self
            .compute_oneoff_invoice(
                conn,
                &invoice_date,
                invoice_lines,
                &invoicing_entity,
                customer,
                currency.clone(),
                discount,
                prepaid_amount,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let due_date = (invoice_date
            + chrono::Duration::days(i64::from(invoicing_entity.net_terms)))
        .and_time(NaiveTime::MIN);

        // Draft invoice uses "draft" as invoice number
        let invoice_number = "draft";

        let invoice_new = InvoiceNew {
            tenant_id: customer.tenant_id,
            customer_id: customer.id,
            subscription_id: None,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            currency,
            line_items: invoice_content.invoice_lines,
            coupons: invoice_content.applied_coupons,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date,
            finalized_at: None,
            total: invoice_content.total,
            amount_due: invoice_content.amount_due,
            applied_credits: invoice_content.applied_credits,
            net_terms: invoicing_entity.net_terms,
            subtotal: invoice_content.subtotal,
            subtotal_recurring: invoice_content.subtotal_recurring,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: Some(due_date),
            plan_name: None,
            invoice_number: invoice_number.to_string(),
            customer_details: InlineCustomer {
                id: customer.id,
                name: customer.name.clone(),
                billing_address: customer.billing_address.clone(),
                vat_number: customer.vat_number.clone(),
                email: customer.billing_email.clone(),
                alias: customer.alias.clone(),
                snapshot_at: chrono::Utc::now().naive_utc(),
            },
            seller_details: InlineInvoicingEntity {
                id: invoicing_entity.id,
                legal_name: invoicing_entity.legal_name.clone(),
                vat_number: invoicing_entity.vat_number.clone(),
                address: invoicing_entity.address(),
                snapshot_at: chrono::Utc::now().naive_utc(),
            },
            auto_advance: true,
            payment_status: InvoicePaymentStatus::Unpaid,
            discount: invoice_content.discount,
            tax_breakdown: invoice_content.tax_breakdown,
            tax_amount: invoice_content.tax_amount,
            manual: false,
            invoicing_entity_id: invoicing_entity.id,
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }

    /// Creates a draft adjustment invoice for an immediate plan change based on proration results.
    /// Returns None if the net proration amount is zero (no adjustment needed).
    /// Creates an adjustment invoice for a plan change proration.
    /// If `plan_version_id_override` is Some, it's used instead of the subscription's
    /// current plan_version_id. This is used for pending-payment plan changes where the
    /// invoice references the TARGET plan version so the settlement handler can detect
    /// and apply the deferred plan change.
    /// Compute adjustment invoice content from proration without inserting.
    pub(in crate::services) async fn compute_adjustment_invoice_content(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: &Subscription,
        customer: &Customer,
        proration: &ProrationResult,
    ) -> crate::StoreResult<AdjustmentInvoiceContent> {
        use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;

        if proration.net_amount_cents == 0 {
            return Ok(AdjustmentInvoiceContent {
                computed: ComputedInvoiceContent {
                    invoice_lines: vec![],
                    subtotal: 0,
                    applied_coupons: vec![],
                    discount: 0,
                    tax_breakdown: vec![],
                    applied_credits: 0,
                    total: 0,
                    tax_amount: 0,
                    amount_due: 0,
                    subtotal_recurring: 0,
                },
                invoicing_entity: None,
            });
        }

        let invoice_date = proration.change_date;
        let period_end = proration.period_end;

        let invoice_lines: Vec<LineItem> = proration
            .lines
            .iter()
            .map(|line| {
                let amount_subtotal = line.amount_cents;
                let taxable_amount = if amount_subtotal > 0 {
                    amount_subtotal
                } else {
                    0
                };

                LineItem {
                    local_id: uuid::Uuid::now_v7().to_string(),
                    name: line.name.clone(),
                    amount_subtotal,
                    tax_rate: Decimal::zero(),
                    taxable_amount,
                    tax_amount: 0,
                    amount_total: amount_subtotal,
                    tax_details: vec![],
                    quantity: None,
                    unit_price: None,
                    start_date: invoice_date,
                    end_date: period_end,
                    sub_lines: vec![],
                    is_prorated: true,
                    price_component_id: line.price_component_id,
                    sub_component_id: None,
                    sub_add_on_id: None,
                    product_id: line.product_id,
                    metric_id: None,
                    description: None,
                    group_by_dimensions: None,
                }
            })
            .collect();

        let invoicing_entity = self
            .store
            .get_invoicing_entity_with_conn(conn, tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let (invoice_lines, tax_breakdown) = self
            .process_invoice_lines_taxes(
                invoice_lines,
                &invoicing_entity,
                customer,
                subscription.currency.clone(),
                &invoice_date,
            )
            .await?;

        let subtotal: i64 = invoice_lines.iter().map(|l| l.amount_subtotal).sum();
        let tax_amount: i64 = invoice_lines.iter().map(|l| l.tax_amount).sum();
        let total = subtotal + tax_amount;

        let applied_credits = if total > 0 {
            let balance_in_invoice_currency = convert_currency(
                conn,
                customer.balance_value_cents.max(0),
                &customer.currency,
                &subscription.currency,
            )
            .await?;
            std::cmp::min(total as u64, balance_in_invoice_currency.max(0) as u64) as i64
        } else {
            0
        };
        let amount_due = std::cmp::max(0, total - applied_credits);

        Ok(AdjustmentInvoiceContent {
            computed: ComputedInvoiceContent {
                invoice_lines,
                subtotal,
                applied_coupons: vec![],
                discount: 0,
                tax_breakdown,
                applied_credits,
                total,
                tax_amount,
                amount_due,
                subtotal_recurring: 0,
            },
            invoicing_entity: Some(invoicing_entity),
        })
    }

    pub(in crate::services) async fn create_adjustment_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: &Subscription,
        customer: &Customer,
        proration: &ProrationResult,
    ) -> Result<Option<Invoice>, StoreErrorReport> {
        let content = self
            .compute_adjustment_invoice_content(conn, tenant_id, subscription, customer, proration)
            .await?;

        self.create_adjustment_invoice_from_content(
            conn,
            subscription,
            customer,
            proration,
            content,
        )
        .await
    }

    pub(in crate::services) async fn create_adjustment_invoice_from_content(
        &self,
        conn: &mut PgConn,
        subscription: &Subscription,
        customer: &Customer,
        proration: &ProrationResult,
        content: AdjustmentInvoiceContent,
    ) -> Result<Option<Invoice>, StoreErrorReport> {
        if content.computed.total == 0 && content.computed.invoice_lines.is_empty() {
            return Ok(None);
        }

        let invoicing_entity = match content.invoicing_entity {
            Some(ie) => ie,
            None => {
                self.store
                    .get_invoicing_entity_with_conn(
                        conn,
                        subscription.tenant_id,
                        Some(customer.invoicing_entity_id),
                    )
                    .await?
            }
        };

        let invoice_date = proration.change_date;
        let due_date = (invoice_date + chrono::Duration::days(i64::from(subscription.net_terms)))
            .and_time(NaiveTime::MIN);

        let invoice_new = InvoiceNew {
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: Some(subscription.id),
            plan_version_id: Some(subscription.plan_version_id),
            invoice_type: InvoiceType::Adjustment,
            currency: subscription.currency.clone(),
            line_items: content.computed.invoice_lines,
            coupons: vec![],
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date,
            finalized_at: None,
            total: content.computed.total,
            amount_due: content.computed.amount_due,
            applied_credits: content.computed.applied_credits,
            net_terms: subscription.net_terms as i32,
            subtotal: content.computed.subtotal,
            subtotal_recurring: 0,
            reference: None,
            purchase_order: subscription.purchase_order.clone(),
            memo: subscription.invoice_memo.clone(),
            due_at: Some(due_date),
            plan_name: Some(subscription.plan_name.clone()),
            invoice_number: "draft".to_string(),
            customer_details: customer.clone().into(),
            seller_details: invoicing_entity.into(),
            auto_advance: false,
            payment_status: if content.computed.amount_due > 0 {
                InvoicePaymentStatus::Unpaid
            } else {
                InvoicePaymentStatus::Paid
            },
            discount: 0,
            tax_breakdown: content.computed.tax_breakdown,
            tax_amount: content.computed.tax_amount,
            manual: false,
            invoicing_entity_id: subscription.invoicing_entity_id,
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }
}

pub(crate) struct AddonPurchaseInvoiceResult {
    pub invoice_content: ComputedInvoiceContent,
    pub proration: ProrationResult,
    pub customer: Customer,
    /// The original (unmodified) subscription details.
    pub subscription: Subscription,
}

impl Services {
    /// Computes prorated invoice content for an addon purchase.
    /// Sets billing_start_date=now and cycle_index=0 on a minimal subscription snapshot
    /// so that compute_invoice produces line items for the new addons only.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn compute_addon_purchase_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        add_ons_request: &[CreateSubscriptionAddOn],
        addons: &[AddOn],
        products_by_id: &HashMap<ProductId, Product>,
        prices_by_id: &HashMap<PriceId, Price>,
    ) -> Result<AddonPurchaseInvoiceResult, StoreErrorReport> {
        let now = Utc::now().date_naive();

        let mut addon_details = self
            .store
            .get_subscription_details_minimal_with_conn(conn, tenant_id, subscription_id)
            .await?;
        let original_subscription = addon_details.subscription.clone();

        let customer: Customer = diesel_models::customers::CustomerRow::find_by_id(
            conn,
            &original_subscription.customer_id,
            &tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;

        addon_details.subscription.billing_start_date = Some(now);
        addon_details.subscription.cycle_index = Some(0);

        for cs_ao in add_ons_request {
            let addon = addons
                .iter()
                .find(|a| a.id == cs_ao.add_on_id)
                .ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Add-on {} not found",
                        cs_ao.add_on_id
                    )))
                })?;
            let resolved = addon
                .resolve_customized(products_by_id, prices_by_id, &cs_ao.customization)
                .map_err(Report::new)?;
            addon_details
                .add_ons
                .push(crate::domain::subscription_add_ons::SubscriptionAddOn {
                    id: SubscriptionAddOnId::new(),
                    subscription_id,
                    add_on_id: addon.id,
                    name: resolved.name,
                    period: resolved.period,
                    fee: resolved.fee,
                    created_at: Utc::now().naive_utc(),
                    product_id: resolved.product_id,
                    price_id: resolved.price_id,
                    quantity: cs_ao.quantity,
                });
        }

        let invoice_content = self
            .compute_invoice(conn, &now, &addon_details, None, None)
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let period_end = original_subscription.current_period_end.unwrap_or(now);
        let full_period = crate::domain::Period {
            start: original_subscription.current_period_start,
            end: period_end,
        };
        let partial_period = crate::domain::Period {
            start: now,
            end: period_end,
        };
        let proration_factor =
            crate::utils::periods::calculate_proration_factor(&partial_period, &full_period)
                .unwrap_or(1.0);

        let proration = ProrationResult {
            lines: invoice_content
                .invoice_lines
                .iter()
                .map(|l| ProrationLineItem {
                    name: l.name.clone(),
                    amount_cents: l.amount_subtotal,
                    full_period_amount_cents: l.amount_subtotal,
                    is_credit: false,
                    product_id: l.product_id,
                    price_component_id: l.price_component_id,
                })
                .collect(),
            net_amount_cents: invoice_content.subtotal,
            change_date: now,
            period_start: original_subscription.current_period_start,
            period_end,
            proration_factor,
        };

        Ok(AddonPurchaseInvoiceResult {
            invoice_content,
            proration,
            customer,
            subscription: original_subscription,
        })
    }
}

pub(crate) struct AdjustmentInvoiceContent {
    pub computed: ComputedInvoiceContent,
    pub invoicing_entity: Option<crate::domain::InvoicingEntity>,
}

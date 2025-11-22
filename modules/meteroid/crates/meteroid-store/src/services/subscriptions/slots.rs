#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::slot_transactions::{
    SlotUpdatePreview, SlotUpgradeBillingMode, UpdateSlotsResult,
};
use crate::domain::{
    LineItem, PaymentTransaction, Period, SlotForTransaction, SubscriptionFee,
    SubscriptionFeeInterface,
};
use crate::errors::StoreError;
use crate::repositories::customers::CustomersInterface;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::subscriptions::slots::validate_slot_limits;
use crate::repositories::subscriptions::{
    SubscriptionInterfaceAuto, SubscriptionSlotsInterface as RepoInterface,
};
use crate::services::Services;
use crate::store::PgConn;
use crate::utils::local_id::LocalId;
use chrono::{NaiveDateTime, NaiveTime};
use common_domain::ids::{InvoiceId, PriceComponentId, SubscriptionId, TenantId};
use common_utils::date::NaiveDateExt;
use common_utils::decimals::ToSubunit;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::{ResultExt, bail};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

impl Services {
    /// Upgrades (delta > 0): Behavior depends on billing_mode (OnCheckout/OnInvoicePaid/Optimistic)
    /// Downgrades (delta < 0): Always deferred to next billing period
    pub async fn update_subscription_slots(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        delta: i32,
        billing_mode: SlotUpgradeBillingMode,
        at_ts: Option<NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        if delta == 0 {
            return Err(StoreError::InvalidArgument(
                "Delta cannot be zero - no slot change requested".to_string(),
            )
            .into());
        }

        self.store
            .transaction(|conn| {
                async move {
                    let subscription_details = self
                        .store
                        .get_subscription_details(tenant_id, subscription_id)
                        .await?;

                    let subscription = &subscription_details.subscription;

                    let period_end = match subscription.current_period_end {
                        Some(period) => period,
                        None => bail!(StoreError::InvalidArgument(
                            "Cannot modify slots for subscription without active billing period"
                                .to_string()
                        )),
                    };

                    let slot = self.extract_slot(&subscription_details, price_component_id)?;

                    let now = at_ts.unwrap_or(chrono::Utc::now().naive_utc());
                    let now_date = now.date();

                    let unit_rate = slot.unit_rate;

                    if delta < 0 {
                        let new_slot_count = self
                            .store
                            .add_slot_transaction_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                period_end,
                                delta,
                                &slot,
                                at_ts,
                            )
                            .await?
                            .prev_active_slots
                            + delta;

                        return Ok(UpdateSlotsResult {
                            new_slot_count,
                            delta_applied: delta,
                            invoice_id: None,
                            prorated_amount: None,
                            slots_active: true,
                        });
                    }

                    // Get currency precision for proper rounding
                    let currency = Currencies::resolve_currency(&subscription.currency)
                        .ok_or_else(|| {
                            StoreError::ValueNotFound(format!(
                                "Currency {} not found",
                                subscription.currency
                            ))
                        })?;

                    let prorated = self
                        .calculate_slot_upgrade_amount(now_date, period_end, delta, &unit_rate)?
                        .round_dp(currency.precision as u32);

                    match billing_mode {
                        SlotUpgradeBillingMode::OnCheckout => {
                            self.handle_on_checkout_upgrade_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                delta,
                                &slot,
                                prorated,
                                at_ts,
                            )
                            .await
                        }
                        SlotUpgradeBillingMode::OnInvoicePaid => {
                            self.handle_on_invoice_paid_upgrade_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                price_component_id,
                                subscription_details,
                                delta,
                                &slot,
                                prorated,
                                now_date,
                                period_end,
                                at_ts,
                            )
                            .await
                        }
                        SlotUpgradeBillingMode::Optimistic => {
                            self.handle_optimistic_upgrade_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                price_component_id,
                                subscription_details,
                                delta,
                                &slot,
                                prorated,
                                now_date,
                                period_end,
                                at_ts,
                            )
                            .await
                        }
                    }
                }
                .scope_boxed()
            })
            .await
    }

    fn calculate_slot_upgrade_amount(
        &self,
        now: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        delta: i32,
        unit_rate: &Decimal,
    ) -> StoreResult<Decimal> {
        let period = Period {
            start: now,
            end: period_end,
        };

        let proration_factor = self.calculate_proration_factor(&period);
        let base_amount = Decimal::from(delta) * unit_rate;

        let prorated = if let Some(factor) = proration_factor {
            base_amount * Decimal::from_f64(factor).unwrap_or(Decimal::ONE)
        } else {
            base_amount
        };

        Ok(prorated.max(Decimal::ZERO))
    }

    async fn handle_on_checkout_upgrade_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        delta: i32,
        slot: &SlotForTransaction,
        prorated_amount: Decimal,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        // OnCheckout: return amount, actual transaction created when payment completes
        let current_slots = self
            .store
            .get_active_slots_value_with_conn(
                conn,
                tenant_id,
                subscription_id,
                slot.unit.clone(),
                at_ts,
            )
            .await?;

        Ok(UpdateSlotsResult {
            new_slot_count: (current_slots as i32) + delta,
            delta_applied: delta,
            invoice_id: None,
            prorated_amount: Some(prorated_amount),
            slots_active: false, // Not active until payment completes
        })
    }

    async fn handle_on_invoice_paid_upgrade_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        subscription_details: crate::domain::SubscriptionDetails,
        delta: i32,
        slot: &SlotForTransaction,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        let unit = slot.unit.clone();

        let invoice_id = self
            .create_slot_upgrade_invoice(
                conn,
                tenant_id,
                subscription_details,
                price_component_id,
                delta,
                unit.as_str(),
                prorated_amount,
                start_date,
                end_date,
            )
            .await?;

        self.store
            .add_pending_slot_transaction_with_conn(
                conn,
                tenant_id,
                subscription_id,
                end_date,
                delta,
                slot,
                invoice_id,
                at_ts,
            )
            .await?;

        let current_slots = self
            .store
            .get_active_slots_value_with_conn(conn, tenant_id, subscription_id, unit, at_ts)
            .await?;

        Ok(UpdateSlotsResult {
            new_slot_count: current_slots as i32, // No change yet
            delta_applied: delta,
            invoice_id: Some(invoice_id),
            prorated_amount: Some(prorated_amount),
            slots_active: false, // Will activate on payment webhook
        })
    }

    async fn handle_optimistic_upgrade_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        subscription_details: crate::domain::SubscriptionDetails,
        delta: i32,
        slot: &SlotForTransaction,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        let new_slot_count = self
            .store
            .add_slot_transaction_tx(
                conn,
                tenant_id,
                subscription_id,
                end_date,
                delta,
                slot,
                at_ts,
            )
            .await?
            .prev_active_slots
            + delta;

        let invoice_id = self
            .create_slot_upgrade_invoice(
                conn,
                tenant_id,
                subscription_details,
                price_component_id,
                delta,
                slot.unit.as_str(),
                prorated_amount,
                start_date,
                end_date,
            )
            .await?;

        Ok(UpdateSlotsResult {
            new_slot_count,
            delta_applied: delta,
            invoice_id: Some(invoice_id),
            prorated_amount: Some(prorated_amount),
            slots_active: true,
        })
    }

    fn calculate_proration_factor(&self, period: &Period) -> Option<f64> {
        let days_in_period = period.end.signed_duration_since(period.start).num_days() as u64;
        let days_in_month_from = u64::from(period.start.days_in_month());

        if days_in_period >= days_in_month_from {
            return None;
        }

        let proration_factor = days_in_period as f64 / days_in_month_from as f64;
        Some(proration_factor)
    }

    fn extract_slot(
        &self,
        subscription_details: &crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
    ) -> StoreResult<SlotForTransaction> {
        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| {
                StoreError::ValueNotFound(format!(
                    "Price component {} not found in subscription",
                    price_component_id
                ))
            })?;

        match slot_component.fee_ref() {
            SubscriptionFee::Slot {
                unit_rate,
                unit,
                min_slots,
                max_slots,
                ..
            } => Ok(SlotForTransaction {
                unit_rate: *unit_rate,
                unit: unit.clone(),
                min_slots: *min_slots,
                max_slots: *max_slots,
            }),
            _ => Err(StoreError::InvalidArgument(format!(
                "Price component {} is not a slot component",
                price_component_id
            ))
            .into()),
        }
    }

    async fn create_slot_upgrade_invoice_draft(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        delta: i32,
        unit_name: &str,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
    ) -> StoreResult<InvoiceId> {
        self.create_slot_upgrade_invoice_internal(
            conn,
            tenant_id,
            subscription_details,
            price_component_id,
            delta,
            unit_name,
            prorated_amount,
            start_date,
            end_date,
            false,
        )
        .await
    }

    async fn create_slot_upgrade_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        delta: i32,
        unit_name: &str,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
    ) -> StoreResult<InvoiceId> {
        self.create_slot_upgrade_invoice_internal(
            conn,
            tenant_id,
            subscription_details,
            price_component_id,
            delta,
            unit_name,
            prorated_amount,
            start_date,
            end_date,
            true,
        )
        .await
    }

    async fn create_slot_upgrade_invoice_internal(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        delta: i32,
        unit_name: &str,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
        respect_auto_advance: bool,
    ) -> StoreResult<InvoiceId> {
        use crate::domain::{InvoiceNew, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
        use crate::repositories::invoices::insert_invoice_tx;

        let subscription = &subscription_details.subscription;
        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| {
                StoreError::ValueNotFound(format!(
                    "Price component {} not found in subscription",
                    price_component_id
                ))
            })?;

        let currency = Currencies::resolve_currency(&subscription.currency).ok_or_else(|| {
            StoreError::ValueNotFound(format!("Currency {} not found", subscription.currency))
        })?;

        let delta_decimal = Decimal::from_i32(delta).ok_or_else(|| {
            StoreError::InvalidArgument(format!("Invalid delta value: {}", delta))
        })?;
        let amount_subtotal = (delta_decimal * prorated_amount)
            .to_subunit_opt(currency.precision)
            .ok_or_else(|| {
                StoreError::InvalidArgument(format!(
                    "Failed to convert amount to subunits for currency {} with precision {}",
                    subscription.currency, currency.precision
                ))
            })?;

        let line_item = LineItem {
            local_id: LocalId::no_prefix(),
            name: format!("{} {} upgrade", delta, unit_name),
            start_date,
            end_date,
            quantity: Decimal::from_i32(delta),
            unit_price: Some(prorated_amount),
            tax_rate: Decimal::ZERO,
            taxable_amount: amount_subtotal,
            tax_amount: 0,
            amount_total: 0,
            is_prorated: true,
            price_component_id: Some(price_component_id),
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: slot_component.product_id(),
            metric_id: None,
            description: Some(format!(
                "Prorated charge for {} additional {}",
                delta, unit_name
            )),
            amount_subtotal,
            tax_details: vec![],
            sub_lines: vec![],
            group_by_dimensions: None,
        };

        let customer = self
            .store
            .find_customer_by_id(subscription.customer_id, tenant_id)
            .await?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(subscription.invoicing_entity_id))
            .await?;

        let invoice_content = self
            .compute_oneoff_invoice(
                conn,
                &start_date,
                vec![line_item],
                &invoicing_entity,
                &customer,
                subscription.currency.clone(),
                None,
                None,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let due_at = Some(
            (start_date + chrono::Duration::days(i64::from(subscription.net_terms)))
                .and_time(NaiveTime::MIN),
        );

        let invoice_new = InvoiceNew {
            tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: Some(subscription.id),
            plan_version_id: Some(subscription.plan_version_id),
            invoice_type: InvoiceType::OneOff,
            currency: subscription.currency.clone(),
            line_items: invoice_content.invoice_lines,
            coupons: invoice_content.applied_coupons,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date: start_date,
            finalized_at: None,
            total: invoice_content.total,
            amount_due: invoice_content.amount_due,
            net_terms: subscription.net_terms as i32,
            subtotal: invoice_content.subtotal,
            subtotal_recurring: invoice_content.subtotal_recurring,
            reference: None,
            purchase_order: subscription.purchase_order.clone(),
            memo: Some(format!("Slot upgrade: +{} {}", delta, unit_name)),
            due_at,
            plan_name: Some(subscription.plan_name.clone()),
            invoice_number: format!("SLOT-{}", uuid::Uuid::new_v4()),
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

        let draft_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        if respect_auto_advance && subscription.auto_advance_invoices {
            self.finalize_invoice_tx(conn, draft_invoice.id, tenant_id, false, &None)
                .await?;
        }

        Ok(draft_invoice.id)
    }

    /// Activate pending slot transactions when invoice is paid
    pub async fn activate_pending_slot_transactions(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        effective_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<Vec<(SubscriptionId, i32)>> {
        let mut conn = self.store.get_conn().await?;

        let activated_transactions = self
            .store
            .activate_pending_slot_transactions_for_invoice(
                &mut conn,
                tenant_id,
                invoice_id,
                effective_at,
            )
            .await?;

        let mut results = Vec::new();
        for transaction in activated_transactions {
            let new_count = transaction.prev_active_slots + transaction.delta;
            results.push((transaction.subscription_id, new_count));
        }

        Ok(results)
    }

    pub async fn preview_slot_update(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        delta: i32,
    ) -> StoreResult<SlotUpdatePreview> {
        if delta == 0 {
            return Err(StoreError::InvalidArgument(
                "Delta cannot be zero - no slot change requested".to_string(),
            )
            .into());
        }

        let subscription_details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await?;

        let subscription = &subscription_details.subscription;

        let period_end = match subscription.current_period_end {
            Some(period) => period,
            None => bail!(StoreError::InvalidArgument(
                "Cannot modify slots for subscription without active billing period".to_string()
            )),
        };

        let slot = self.extract_slot(&subscription_details, price_component_id)?;

        let now = chrono::Utc::now().naive_utc();
        let now_date = now.date();

        let mut conn = self.store.get_conn().await?;

        let current_slots = self
            .store
            .get_active_slots_value_with_conn(
                &mut conn,
                tenant_id,
                subscription_id,
                slot.unit.clone(),
                None,
            )
            .await? as i32;

        let new_slots = current_slots + delta;

        // Validate limits
        validate_slot_limits(&slot, delta, current_slots)?;

        let effective_at = if delta > 0 { now_date } else { period_end };

        // Get currency precision for proper rounding
        let currency = Currencies::resolve_currency(&subscription.currency).ok_or_else(|| {
            StoreError::ValueNotFound(format!("Currency {} not found", subscription.currency))
        })?;

        let (prorated_amount, full_period_amount) = if delta > 0 {
            let prorated = self
                .calculate_slot_upgrade_amount(now_date, period_end, delta, &slot.unit_rate)?
                .round_dp(currency.precision as u32);
            let full_period =
                (Decimal::from(delta) * slot.unit_rate).round_dp(currency.precision as u32);
            (prorated, full_period)
        } else {
            // For downgrades, show what they'll save
            let full_period_reduction =
                (Decimal::from(-delta) * slot.unit_rate).round_dp(currency.precision as u32);
            (Decimal::ZERO, -full_period_reduction)
        };

        let period = Period {
            start: now_date,
            end: period_end,
        };
        let days_remaining = period.end.signed_duration_since(period.start).num_days() as i32;
        let days_total = now_date.days_in_month() as i32;

        Ok(SlotUpdatePreview {
            current_slots,
            new_slots,
            delta,
            unit: slot.unit.clone(),
            unit_rate: slot.unit_rate,
            prorated_amount,
            full_period_amount,
            days_remaining,
            days_total,
            effective_at,
            current_period_end: period_end,
            next_invoice_delta: prorated_amount,
        })
    }

    pub async fn complete_slot_upgrade_checkout(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        delta: i32,
        payment_method_id: common_domain::ids::CustomerPaymentMethodId,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<(PaymentTransaction, i32)> {
        use diesel_async::scoped_futures::ScopedFutureExt;

        if delta <= 0 {
            return Err(StoreError::InvalidArgument(
                "Slot upgrades via checkout must have positive delta".to_string(),
            )
            .into());
        }

        self.store
            .transaction(|conn| {
                async move {
                    let subscription_details = self
                        .store
                        .get_subscription_details(tenant_id, subscription_id)
                        .await?;

                    let now = at_ts.unwrap_or(chrono::Utc::now().naive_utc());
                    let now_date = now.date();
                    let period_end = subscription_details.subscription.current_period_end.ok_or(
                        StoreError::InvalidArgument(
                            "Subscription must have current_period_end".to_string(),
                        ),
                    )?;

                    let slot = self.extract_slot(&subscription_details, price_component_id)?;

                    let unit_rate = slot.unit_rate;
                    let unit_name = slot.unit.clone();

                    // Get currency precision for proper rounding
                    let currency =
                        Currencies::resolve_currency(&subscription_details.subscription.currency)
                            .ok_or_else(|| {
                            StoreError::ValueNotFound(format!(
                                "Currency {} not found",
                                subscription_details.subscription.currency
                            ))
                        })?;

                    let prorated = self
                        .calculate_slot_upgrade_amount(now_date, period_end, delta, &unit_rate)?
                        .round_dp(currency.precision as u32);

                    let active_slots = self
                        .store
                        .get_active_slots_value_with_conn(
                            conn,
                            tenant_id,
                            subscription_id,
                            unit_name.clone(),
                            None,
                        )
                        .await?;

                    validate_slot_limits(&slot, delta, active_slots as i32)?;

                    let invoice_id = self
                        .create_slot_upgrade_invoice_draft(
                            conn,
                            tenant_id,
                            subscription_details.clone(),
                            price_component_id,
                            delta,
                            &unit_name,
                            prorated,
                            now_date,
                            period_end,
                        )
                        .await?;

                    let payment_result = self
                        .process_invoice_payment_tx(conn, tenant_id, invoice_id, payment_method_id)
                        .await?;

                    if payment_result.status == crate::domain::PaymentStatusEnum::Settled {
                        // Update subscription's payment method with the one that successfully paid
                        let payment_method = diesel_models::customer_payment_methods::CustomerPaymentMethodRow::get_by_id(
                            conn,
                            &tenant_id,
                            &payment_method_id,
                        )
                        .await
                        .map_err(|e| StoreError::DatabaseError(e.error))?;

                        diesel_models::subscriptions::SubscriptionRow::update_subscription_payment_method(
                            conn,
                            subscription_id,
                            tenant_id,
                            Some(payment_method_id),
                            Some(payment_method.payment_method_type),
                        )
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;

                        let slot_transaction = self
                            .store
                            .add_slot_transaction_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                period_end,
                                delta,
                                &slot,
                                at_ts,
                            )
                            .await?;

                        let new_slot_count =
                            slot_transaction.prev_active_slots + slot_transaction.delta;

                        self.finalize_invoice_tx(conn, invoice_id, tenant_id, false, &None)
                            .await?;

                        Ok((payment_result, new_slot_count))
                    } else {
                        Err(StoreError::PaymentError(format!(
                            "Payment failed or pending. Status: {:?}",
                            payment_result.status
                        ))
                        .into())
                    }
                }
                .scope_boxed()
            })
            .await
    }
}

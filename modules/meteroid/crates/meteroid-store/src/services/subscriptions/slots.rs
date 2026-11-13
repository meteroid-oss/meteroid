#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::payment_transactions::PaymentNextAction;
use crate::domain::slot_transactions::{
    SlotUpdatePreview, SlotUpgradeBillingMode, UpdateSlotsResult,
};
use crate::domain::{
    ChargeType, LineItem, PaymentTransaction, Period, SlotForTransaction, SubscriptionFee,
    SubscriptionFeeInterface,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::customers::CustomersInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::subscriptions::slots::validate_slot_limits;
use crate::repositories::subscriptions::{
    SubscriptionInterfaceAuto, SubscriptionSlotsInterface as RepoInterface,
};
use crate::services::Services;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
use crate::store::PgConn;
use crate::utils::local_id::LocalId;
use chrono::{NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, InvoiceId, PriceComponentId, SubscriptionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_models::enums::SubscriptionEventType;
use diesel_models::lock::CustomerLock;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt, bail};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

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
        actor: Actor,
        at_ts: Option<NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        if delta == 0 {
            return Err(StoreError::InvalidArgument(
                "Delta cannot be zero - no slot change requested".to_string(),
            )
            .into());
        }

        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            let actor = &actor;
            async move {
                // Customer, then subscription, before the seats: the order finalization takes.
                let lock =
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
                let lock = &lock;
                diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
                crate::services::subscriptions::plan_change::refuse_while_a_paid_change_is_pending(
                    conn,
                    tenant_id,
                    subscription_id,
                    None,
                )
                .await?;
                let subscription_details = self
                    .store
                    .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                    .await?;

                let subscription = &subscription_details.subscription;

                let period_end = match subscription.current_period_end {
                    Some(period) => period,
                    None => bail!(StoreError::InvalidArgument(
                        "Cannot modify slots for subscription without active billing period"
                            .to_string()
                    )),
                };

                let slot = services.extract_slot(&subscription_details, price_component_id)?;

                let now = at_ts.unwrap_or(common_domain::clock::now().naive_utc());
                let now_date = now.date();

                let unit_rate = slot.unit_rate;
                let customer_id = subscription.customer_id;

                let result = if delta < 0 {
                    let new_slot_count = self
                        .store
                        .add_slot_transaction_tx(
                            conn,
                            lock,
                            subscription_id,
                            period_end,
                            delta,
                            &slot,
                            at_ts,
                        )
                        .await?
                        .prev_active_slots
                        + delta;

                    UpdateSlotsResult {
                        new_slot_count,
                        delta_applied: delta,
                        invoice_id: None,
                        prorated_amount: None,
                        slots_active: true,
                    }
                } else {
                    // Get currency precision for proper rounding
                    let currency = Currencies::resolve_currency(&subscription.currency)
                        .ok_or_else(|| {
                            StoreError::ValueNotFound(format!(
                                "Currency {} not found",
                                subscription.currency
                            ))
                        })?;

                    let period_start = subscription.current_period_start;
                    let prorated = services.calculate_slot_upgrade_amount(
                        period_start,
                        now_date,
                        period_end,
                        delta,
                        &unit_rate,
                        currency.precision as u32,
                    )?;

                    match billing_mode {
                        SlotUpgradeBillingMode::OnCheckout => {
                            services
                                .handle_on_checkout_upgrade_tx(
                                    conn,
                                    tenant_id,
                                    subscription_id,
                                    delta,
                                    &slot,
                                    prorated,
                                    at_ts,
                                )
                                .await?
                        }
                        SlotUpgradeBillingMode::OnInvoicePaid => {
                            services
                                .handle_on_invoice_paid_upgrade_tx(
                                    conn,
                                    lock,
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
                                .await?
                        }
                        SlotUpgradeBillingMode::Optimistic => {
                            services
                                .handle_optimistic_upgrade_tx(
                                    conn,
                                    lock,
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
                                .await?
                        }
                    }
                };

                // Deferred billing modes (OnInvoicePaid / optimistic-pending) apply the slot
                // change only once the upgrade invoice is paid; those emit at activation time
                // (`activate_pending_slot_transactions`) instead, so the payload reflects the
                // real slot count. Emit here only when the change is already active.
                if result.slots_active {
                    services
                        .emit_subscription_updated_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            crate::domain::outbox_event::SubscriptionUpdateType::UnitsChanged,
                        )
                        .await?;
                }

                // Audit the requested seat change against the acting user. Pending
                // (checkout/on-paid) and deferred (downgrade) modes activate later;
                // `applied` distinguishes an immediate change from a booked one.
                let activity = Activity::new(
                    ActivityType::SubscriptionSlotsChanged,
                    EntityType::Subscription,
                    subscription_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "from": result.new_slot_count - result.delta_applied,
                    "to": result.new_slot_count,
                    "delta": result.delta_applied,
                    "mode": format!("{billing_mode:?}"),
                    "applied": result.slots_active,
                }))
                .agg_customer(customer_id);
                services
                    .store
                    .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Ok(result)
            }
            .scope_boxed()
        })
        .await
    }

    fn calculate_slot_upgrade_amount(
        &self,
        period_start: chrono::NaiveDate,
        now: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        delta: i32,
        unit_rate: &Decimal,
        precision: u32,
    ) -> StoreResult<Decimal> {
        Ok(slot_upgrade_amount(
            period_start,
            now,
            period_end,
            delta,
            unit_rate,
            precision,
        ))
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
        lock: &CustomerLock,
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
        let tenant_id = lock.tenant_id();
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
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
                    lock,
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
        })
        .await
    }

    async fn handle_optimistic_upgrade_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id: SubscriptionId,
        price_component_id: PriceComponentId,
        mut subscription_details: crate::domain::SubscriptionDetails,
        delta: i32,
        slot: &SlotForTransaction,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        let tenant_id = lock.tenant_id();
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let new_slot_count = self
                .store
                .add_slot_transaction_tx(conn, lock, subscription_id, end_date, delta, slot, at_ts)
                .await?
                .prev_active_slots
                + delta;

            // The added slots are active immediately, so the subscription MRR grows now.
            // Recompute it (live slot counts) before `subscription_details` is consumed.
            let old_mrr = subscription_details.subscription.mrr_cents as i64;
            let precision =
                Currencies::resolve_currency_precision(&subscription_details.subscription.currency)
                    .unwrap_or(2);
            let component_lines = calculate_components_mrr_with_slots(
                conn,
                tenant_id,
                subscription_id,
                &subscription_details.price_components,
                precision,
            )
            .await?;
            // Stated as stored, so the delta against `old_mrr` is like for like.
            let new_mrr = self
                .restate_subscription_mrr(
                    conn,
                    &mut subscription_details,
                    component_lines,
                    precision,
                    start_date,
                )
                .await?;
            let mrr_delta = new_mrr - old_mrr;

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

            // Book the slot expansion: a Switch event (mrr_delta None, so process_mrr
            // never books it) plus the sole movement writer.
            if mrr_delta != 0 {
                SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                SubscriptionEventRow {
                    id: Uuid::now_v7(),
                    subscription_id,
                    event_type: SubscriptionEventType::Switch,
                    details: Some(serde_json::json!({ "kind": "slots", "mode": "immediate" })),
                    created_at: common_domain::clock::now().naive_utc(),
                    mrr_delta: None,
                    bi_mrr_movement_log_id: None,
                    applies_to: start_date,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                self.create_movement_mrr_log(
                    conn,
                    tenant_id,
                    subscription_id,
                    start_date,
                    mrr_delta,
                )
                .await?;
            }

            Ok(UpdateSlotsResult {
                new_slot_count,
                delta_applied: delta,
                invoice_id: Some(invoice_id),
                prorated_amount: Some(prorated_amount),
                slots_active: true,
            })
        })
        .await
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
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
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
        })
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
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
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
        })
        .await
    }

    /// Build the taxed prorated invoice content for a slot upgrade (delta > 0).
    ///
    /// `prorated_amount` is the prorated total for ALL `delta` slots (it already
    /// includes `delta` — see `calculate_slot_upgrade_amount`), so it is converted
    /// straight to subunits and must NOT be multiplied by `delta` again. The
    /// per-slot `unit_price` is derived so `quantity × unit_price == amount_subtotal`.
    ///
    /// Shared by the apply route (`create_slot_upgrade_invoice_internal`) and the
    /// preview (`preview_slot_update`) so the previewed charge equals what is billed.
    async fn compute_slot_upgrade_invoice_content(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: &crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        delta: i32,
        unit_name: &str,
        prorated_amount: Decimal,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
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
        let amount_subtotal = prorated_amount
            .to_subunit_opt(currency.precision)
            .ok_or_else(|| {
                StoreError::InvalidArgument(format!(
                    "Failed to convert amount to subunits for currency {} with precision {}",
                    subscription.currency, currency.precision
                ))
            })?;

        // Per-slot prorated price (precision 8) for display; `amount_subtotal` above
        // is the authoritative figure used to compute the invoice total.
        let unit_price = round_money(prorated_amount / delta_decimal, 8);

        let line_item = LineItem {
            local_id: LocalId::no_prefix(),
            name: format!("{} {} upgrade", delta, unit_name),
            // Slot upgrade: prorated per-seat recurring charge.
            charge_type: Some(ChargeType::Recurring),
            start_date,
            end_date,
            quantity: Some(delta_decimal),
            unit_price: Some(unit_price),
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
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: Some(subscription.id),
            usage_ends_at: None,
        };

        let customer = self
            .store
            .find_customer_by_id_with_conn(conn, subscription.customer_id, tenant_id)
            .await?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity_with_conn(conn, tenant_id, Some(subscription.invoicing_entity_id))
            .await?;

        self.compute_oneoff_invoice(
            conn,
            &start_date,
            vec![line_item],
            &invoicing_entity,
            &customer,
            subscription.currency.clone(),
            None,
            None,
            subscription.tax_inclusive,
        )
        .await
        .change_context(StoreError::InvoiceComputationError)
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
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            use crate::domain::{InvoiceNew, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
            use crate::repositories::invoices::insert_invoice_tx;

            let invoice_content = self
                .compute_slot_upgrade_invoice_content(
                    conn,
                    tenant_id,
                    &subscription_details,
                    price_component_id,
                    delta,
                    unit_name,
                    prorated_amount,
                    start_date,
                    end_date,
                )
                .await?;

            let subscription = &subscription_details.subscription;

            let customer = self
                .store
                .find_customer_by_id_with_conn(conn, subscription.customer_id, tenant_id)
                .await?;

            let invoicing_entity = self
                .store
                .get_invoicing_entity_with_conn(
                    conn,
                    tenant_id,
                    Some(subscription.invoicing_entity_id),
                )
                .await?;

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
                // Not a subscription period: never period-refreshed, so no cycle to pin.
                cycle_index: None,
                currency: subscription.currency.clone(),
                line_items: invoice_content.invoice_lines,
                coupons: invoice_content.applied_coupons,
                data_updated_at: None,
                status: InvoiceStatusEnum::Draft,
                invoice_date: start_date,
                billing_period_start: start_date,
                tax_inclusive: subscription.tax_inclusive,
                finalized_at: None,
                total: invoice_content.total,
                amount_due: invoice_content.amount_due,
                applied_credits: invoice_content.applied_credits,
                promotional_credits: invoice_content.promotional_credits,
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
                parent_invoice_id: None,
                consolidated_into_invoice_id: None,
                annexes: vec![],
                custom_properties: serde_json::Value::Null,
            };

            let draft_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

            if respect_auto_advance && subscription.auto_advance_invoices {
                self.finalize_invoice_tx(
                    conn,
                    &Actor::Customer {
                        id: subscription.customer_id,
                    },
                    draft_invoice.id,
                    tenant_id,
                    false,
                    &None,
                )
                .await?;
            }

            Ok(draft_invoice.id)
        })
        .await
    }

    /// Activate pending slot transactions when invoice is paid
    pub async fn activate_pending_slot_transactions(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        effective_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<Vec<(SubscriptionId, i32)>> {
        use scoped_futures::ScopedFutureExt;
        self.store
            .transaction(|conn| {
                async move {
                    // Seat writes come after the customer, as every slot change.
                    let lock = diesel_models::invoices::InvoiceRow::lock_customer_of(
                        conn, tenant_id, invoice_id,
                    )
                    .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let activated_transactions = self
                        .store
                        .activate_pending_slot_transactions_for_invoice(
                            conn,
                            &lock,
                            invoice_id,
                            effective_at,
                        )
                        .await?;

                    let mut results = Vec::new();
                    for transaction in activated_transactions {
                        let new_count = transaction.prev_active_slots + transaction.delta;
                        // The deferred slot change is now active: emit the `units_changed` that
                        // the request-time path skipped for OnInvoicePaid/optimistic-pending modes.
                        self.emit_subscription_updated_tx(
                            conn,
                            tenant_id,
                            transaction.subscription_id,
                            crate::domain::outbox_event::SubscriptionUpdateType::UnitsChanged,
                        )
                        .await?;
                        results.push((transaction.subscription_id, new_count));
                    }

                    Ok(results)
                }
                .scope_boxed()
            })
            .await
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

        let now = common_domain::clock::now().naive_utc();
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

        let period_start = subscription.current_period_start;

        let (prorated_amount, full_period_amount, adjustment_invoice) = if delta > 0 {
            let prorated = self.calculate_slot_upgrade_amount(
                period_start,
                now_date,
                period_end,
                delta,
                &slot.unit_rate,
                currency.precision as u32,
            )?;
            let full_period = round_money(
                Decimal::from(delta) * slot.unit_rate,
                currency.precision as u32,
            );
            // Compute the taxed invoice through the exact path the apply route bills.
            let invoice = self
                .compute_slot_upgrade_invoice_content(
                    &mut conn,
                    tenant_id,
                    &subscription_details,
                    price_component_id,
                    delta,
                    slot.unit.as_str(),
                    prorated,
                    now_date,
                    period_end,
                )
                .await?;
            (prorated, full_period, Some(invoice))
        } else {
            // For downgrades, show what they'll save
            let full_period_reduction = round_money(
                Decimal::from(-delta) * slot.unit_rate,
                currency.precision as u32,
            );
            (Decimal::ZERO, -full_period_reduction, None)
        };

        let days_remaining = period_end.signed_duration_since(now_date).num_days() as i32;
        let days_total = period_end.signed_duration_since(period_start).num_days() as i32;

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
            adjustment_invoice,
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
    ) -> StoreResult<(PaymentTransaction, i32, Option<PaymentNextAction>)> {
        use scoped_futures::ScopedFutureExt;

        if delta <= 0 {
            return Err(StoreError::InvalidArgument(
                "Slot upgrades via checkout must have positive delta".to_string(),
            )
            .into());
        }

        self.store
            .transaction(|conn| {
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let subscription_details = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    self.ensure_payment_method_of_customer(
                        conn,
                        tenant_id,
                        subscription_details.subscription.customer_id,
                        payment_method_id,
                    )
                    .await?;

                    let now = at_ts.unwrap_or(common_domain::clock::now().naive_utc());
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

                    let period_start = subscription_details.subscription.current_period_start;
                    let prorated = self.calculate_slot_upgrade_amount(
                        period_start,
                        now_date,
                        period_end,
                        delta,
                        &unit_rate,
                        currency.precision as u32,
                    )?;

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

                    let (payment_result, next_action) = self
                        .process_invoice_payment_tx(
                            conn,
                            tenant_id,
                            invoice_id,
                            payment_method_id,
                            true,
                            true,
                            None,
                        )
                        .await?;

                    use crate::domain::PaymentStatusEnum;
                    match payment_result.status {
                        PaymentStatusEnum::Settled => {
                            // Payment succeeded - payment method is already saved on the customer
                            let slot_transaction = self
                                .store
                                .add_slot_transaction_tx(
                                    conn,
                                    &lock,
                                    subscription_id,
                                    period_end,
                                    delta,
                                    &slot,
                                    at_ts,
                                )
                                .await?;

                            let new_slot_count =
                                slot_transaction.prev_active_slots + slot_transaction.delta;

                            self.finalize_invoice_tx(
                                conn,
                                &Actor::System,
                                invoice_id,
                                tenant_id,
                                false,
                                &None,
                            )
                            .await?;

                            Ok((payment_result, new_slot_count, None))
                        }
                        // 3DS/SCA (or async settlement): record the slot as pending
                        // and finalize the invoice. The slot activates when the
                        // payment settles (payment webhook → invoice paid →
                        // activate_pending_slot_transactions); the portal completes
                        // the returned next_action meanwhile.
                        PaymentStatusEnum::Pending | PaymentStatusEnum::Ready => {
                            self.store
                                .add_pending_slot_transaction_with_conn(
                                    conn,
                                    &lock,
                                    subscription_id,
                                    period_end,
                                    delta,
                                    &slot,
                                    invoice_id,
                                    at_ts,
                                )
                                .await?;

                            self.finalize_invoice_tx(
                                conn,
                                &Actor::System,
                                invoice_id,
                                tenant_id,
                                false,
                                &None,
                            )
                            .await?;

                            let current_slots = self
                                .store
                                .get_active_slots_value_with_conn(
                                    conn,
                                    tenant_id,
                                    subscription_id,
                                    unit_name.clone(),
                                    None,
                                )
                                .await?;

                            Ok((payment_result, current_slots as i32, next_action))
                        }
                        // A fresh charge never settles-then-reverses in one call;
                        // Refunded here would be anomalous, so treat it as a failure.
                        PaymentStatusEnum::Failed
                        | PaymentStatusEnum::Cancelled
                        | PaymentStatusEnum::Refunded => Err(StoreError::PaymentError(format!(
                            "Payment failed. Status: {:?}",
                            payment_result.status
                        ))
                        .into()),
                    }
                }
                .scope_boxed()
            })
            .await
    }
}

/// Money to the currency's minor unit, half-up like every invoice amount (OQ14).
fn round_money(amount: Decimal, precision: u32) -> Decimal {
    amount.round_dp_with_strategy(
        precision,
        rust_decimal::RoundingStrategy::MidpointAwayFromZero,
    )
}

/// The upgrade of `delta` slots for the whole days left of the period, half-up.
fn slot_upgrade_amount(
    period_start: chrono::NaiveDate,
    now: chrono::NaiveDate,
    period_end: chrono::NaiveDate,
    delta: i32,
    unit_rate: &Decimal,
    precision: u32,
) -> Decimal {
    let full_period = Period {
        start: period_start,
        end: period_end,
    };
    let partial_period = Period {
        start: now,
        end: period_end,
    };

    let proration_factor =
        crate::utils::periods::calculate_proration_factor(&partial_period, &full_period);
    let base_amount = Decimal::from(delta) * unit_rate;

    let prorated = if let Some(factor) = proration_factor {
        factor.prorate_decimal(base_amount)
    } else {
        base_amount
    };

    round_money(prorated.max(Decimal::ZERO), precision)
}

#[cfg(test)]
mod tests {
    use super::slot_upgrade_amount;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    /// One day left of a 28-day February on a 0.70 seat is 0.025: half-up 0.03 (OQ14), where
    /// half-even with a float factor billed 0.02.
    #[test]
    fn a_half_cent_slot_upgrade_rounds_half_up() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 2, d).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(
            slot_upgrade_amount(day(1), day(28), end, 1, &dec!(0.70), 2),
            dec!(0.03)
        );
        assert_eq!(
            slot_upgrade_amount(day(1), day(1), end, 2, &dec!(0.70), 2),
            dec!(1.40)
        );
        // 3.78 for one day of 28 is 0.135 exactly: 0.14, where dividing first read 0.1349… (0.13).
        assert_eq!(
            slot_upgrade_amount(day(1), day(28), end, 1, &dec!(3.78), 2),
            dec!(0.14)
        );
    }
}

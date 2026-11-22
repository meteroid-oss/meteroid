#![allow(clippy::too_many_arguments)]

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::payment_transactions::PaymentNextAction;
use crate::domain::slot_transactions::{SlotUpgradeBillingMode, UpdateSlotsResult};
use crate::domain::{
    ChargeType, LineItem, PaymentTransaction, SlotForTransaction, SubscriptionFee,
    SubscriptionFeeInterface,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::customers::CustomersInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::subscriptions::SubscriptionSlotsInterface as RepoInterface;
use crate::repositories::subscriptions::slots::validate_slot_limits;
use crate::services::Services;
use crate::services::evaluate::change::{Bought, seat_purchase};
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::subscriptions::proration::DaySpan;
use crate::store::PgConn;
use crate::utils::local_id::LocalId;
use chrono::{NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, InvoiceId, PriceComponentId, SubscriptionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_models::enums::SubscriptionEventType;
use diesel_models::lock::CustomerLock;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt};
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

                let slot = services.extract_slot(&subscription_details, price_component_id)?;

                let now = at_ts.unwrap_or(common_clock::now().naive_utc());
                let now_date = now.date();
                let left = services
                    .slot_fee_period(conn, &subscription_details, price_component_id, now_date)
                    .await?;
                let period_end = left.until;

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

                    // Held from the day their step is written, at most the fee period's end.
                    let on = now_date.min(period_end);
                    let prorated = services
                        .seat_purchase_amount(
                            conn,
                            &subscription_details,
                            price_component_id,
                            delta,
                            on,
                            currency.precision as u32,
                        )
                        .await?;

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
                                    on,
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
                                    on,
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

    /// What `delta` seats bought on `on` bill, from the facts as if held from that day: the slot
    /// fee's charge for its period less what its lines billed (R-079, R-080), in major units.
    pub(in crate::services) async fn seat_purchase_amount(
        &self,
        conn: &mut PgConn,
        details: &crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        delta: i32,
        on: chrono::NaiveDate,
        precision: u32,
    ) -> StoreResult<Decimal> {
        let component = details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| {
                StoreError::ValueNotFound(format!(
                    "Price component {price_component_id} not found in subscription"
                ))
            })?;
        let bought = Bought {
            lineage: billing_core::charge::Fee::Component(component.lineage()),
            seats: u64::try_from(delta).unwrap_or(0),
        };
        let purchase = seat_purchase(
            conn,
            details.subscription.tenant_id,
            details.subscription.id,
            on,
            &bought,
        )
        .await?;
        Ok(Decimal::new(
            purchase.map_or(0, |purchase| purchase.amount.max(0)),
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

            // The added slots are active immediately, so the subscription MRR grows now: the
            // writer's, from the seat step just written.
            let old_mrr = subscription_details.subscription.mrr_cents as i64;
            self.derive_state_tx(
                conn,
                &crate::domain::entity_activity::Actor::System,
                tenant_id,
                subscription_id,
                crate::services::evaluate::bill::midnight(start_date),
            )
            .await?;
            let new_mrr =
                SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .subscription
                    .mrr_cents;
            subscription_details.subscription.mrr_cents = new_mrr.max(0) as u64;

            let invoice_id = Some(
                self.create_slot_upgrade_invoice(
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
                .await?,
            );

            // The expansion's record: a Switch event (mrr_delta None, so process_mrr never
            // books it); the writer booked the movement.
            if new_mrr != old_mrr {
                SubscriptionEventRow {
                    id: Uuid::now_v7(),
                    subscription_id,
                    event_type: SubscriptionEventType::Switch,
                    details: Some(serde_json::json!({ "kind": "slots", "mode": "immediate" })),
                    created_at: common_clock::now().naive_utc(),
                    mrr_delta: None,
                    bi_mrr_movement_log_id: None,
                    applies_to: start_date,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            }

            Ok(UpdateSlotsResult {
                new_slot_count,
                delta_applied: delta,
                invoice_id,
                prorated_amount: Some(prorated_amount),
                slots_active: true,
            })
        })
        .await
    }

    /// What is left from `on` of the slot fee's period on its own grid (R-084), up to the term's
    /// end: a fee billed off the subscription's cadence prorates an upgrade, and steps a downgrade
    /// down, over its own period.
    async fn slot_fee_period(
        &self,
        conn: &mut PgConn,
        details: &crate::domain::SubscriptionDetails,
        price_component_id: PriceComponentId,
        on: chrono::NaiveDate,
    ) -> StoreResult<DaySpan> {
        let subscription = &details.subscription;
        let current_end = subscription.current_period_end.ok_or_else(|| {
            StoreError::InvalidArgument(
                "Cannot modify slots for subscription without active billing period".to_string(),
            )
        })?;
        let period = details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .map(|c| *c.period_ref())
            .ok_or_else(|| {
                StoreError::ValueNotFound(format!(
                    "Price component {price_component_id} not found in subscription"
                ))
            })?;
        let grid = crate::services::subscriptions::proration::OwnGrid::read(
            conn,
            subscription,
            (subscription.current_period_start, current_end),
            on,
        )
        .await?;
        // A fee of the subscription's cadence bills the period holding its current one, on its
        // run's own grid, which a change of cadence may have left apart (R-084).
        let same_cadence = period
            .as_billing_period_opt()
            .is_some_and(|own| own.as_months() == subscription.period.as_months());
        let lineage = details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .map(|c| billing_core::charge::Fee::Component(c.lineage()))
            .filter(|_| same_cadence);
        let held = on
            .min(current_end.pred_opt().unwrap_or(current_end))
            .max(subscription.current_period_start);
        let billed = match &lineage {
            Some(lineage) => {
                crate::services::evaluate::change::fee_period_on(
                    conn,
                    subscription.tenant_id,
                    subscription.id,
                    held,
                    lineage,
                )
                .await?
            }
            None => None,
        };
        let own = billed.unwrap_or_else(|| {
            crate::services::subscriptions::proration::own_period(grid, on)(&period)
        });
        Ok(grid.left(own, on))
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
    pub(in crate::services) async fn compute_slot_upgrade_invoice_content(
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
            // The seats bill the slot fee's charge for its period: reconcile reads them on it.
            sub_component_id: Some(slot_component.id),
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
            usage_breakdown: None,
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

        let (invoice_id, unit_name, claim) = self
            .store
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

                    let now = at_ts.unwrap_or(common_clock::now().naive_utc());
                    let now_date = now.date();

                    let slot = self.extract_slot(&subscription_details, price_component_id)?;
                    let left = self
                        .slot_fee_period(conn, &subscription_details, price_component_id, now_date)
                        .await?;
                    let period_end = left.until;

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

                    let on = now_date.min(period_end);
                    let prorated = self
                        .seat_purchase_amount(
                            conn,
                            &subscription_details,
                            price_component_id,
                            delta,
                            on,
                            currency.precision as u32,
                        )
                        .await?;

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
                            on,
                            period_end,
                        )
                        .await?;

                    // Pending until its payment settles; a refused payment drops it.
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
                    let claim = self
                        .claim_invoice_charge_tx(
                            conn,
                            tenant_id,
                            invoice_id,
                            payment_method_id,
                            true,
                            true,
                            None,
                        )
                        .await?;
                    Ok((invoice_id, unit_name, claim))
                }
                .scope_boxed()
            })
            .await?;

        use crate::services::payment::process_payment::{ChargeAnswer, InvoiceChargeClaim};
        let charge = match claim {
            InvoiceChargeClaim::Claimed(charge) => charge,
            // A fresh invoice has no earlier attempt.
            InvoiceChargeClaim::InFlight(_) | InvoiceChargeClaim::Recorded(_) => {
                return Err(crate::services::payment::process_payment::payment_in_flight());
            }
        };
        let row = charge.transaction().clone();

        match self.send_invoice_charge(&charge).await {
            ChargeAnswer::Outcome { intent, amount }
                if matches!(
                    intent.status,
                    crate::domain::PaymentStatusEnum::Failed
                        | crate::domain::PaymentStatusEnum::Cancelled
                ) =>
            {
                let status = intent.status.clone();
                self.unwind_slot_upgrade(tenant_id, invoice_id, &row, Some((*intent, amount)))
                    .await?;
                Err(StoreError::PaymentError(format!("Payment failed. Status: {status:?}")).into())
            }
            ChargeAnswer::Refused(error) => {
                self.unwind_slot_upgrade(tenant_id, invoice_id, &row, None)
                    .await?;
                Err(error)
            }
            answer => {
                let answered = match answer {
                    ChargeAnswer::Outcome { intent, amount } => Some((*intent, amount)),
                    // No answer: the reconciliation worker re-sends it under the same key. An
                    // unconfirmed re-send leaves the issued invoice to reminders, its seats
                    // activating once paid, as a payment failing after 3DS does.
                    _ => None,
                };
                self.record_slot_upgrade_charge(
                    tenant_id,
                    subscription_id,
                    invoice_id,
                    &row,
                    answered,
                    unit_name,
                    at_ts,
                )
                .await
            }
        }
    }

    /// Records a slot upgrade's charge on its committed row and issues the invoice; a capture
    /// activates the slots at once, a payment settling later activates them when it does.
    #[allow(clippy::too_many_arguments)]
    async fn record_slot_upgrade_charge(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        invoice_id: InvoiceId,
        row: &diesel_models::payments::PaymentTransactionRow,
        answered: Option<(crate::domain::payment_transactions::PaymentIntent, i64)>,
        unit_name: String,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<(PaymentTransaction, i32, Option<PaymentNextAction>)> {
        use crate::repositories::payment_transactions::PaymentTransactionInterface;
        let next_action = answered
            .as_ref()
            .and_then(|(intent, _)| intent.next_action.clone());
        let answered = &answered;
        let transaction = self
            .store
            .transaction(|conn| {
                async move {
                    let lock = diesel_models::invoices::InvoiceRow::lock_customer_of(
                        conn, tenant_id, invoice_id,
                    )
                    .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let transaction = match answered {
                        Some((intent, amount)) => {
                            let mut existing = self
                                .store
                                .get_payment_tx_by_id_for_update(conn, row.id, tenant_id)
                                .await?;
                            if *amount != existing.amount {
                                existing = diesel_models::payments::PaymentTransactionRowPatch {
                                    id: row.id,
                                    amount: Some(*amount),
                                    ..Default::default()
                                }
                                .patch(conn, tenant_id, row.id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?
                                .into();
                            }
                            self.store
                                .consolidate_intent_and_transaction_tx(
                                    conn,
                                    &Actor::System,
                                    existing,
                                    intent.clone(),
                                )
                                .await?
                        }
                        None => row.clone().into(),
                    };
                    let invoice = diesel_models::invoices::InvoiceRow::find_by_id(
                        conn, tenant_id, invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    if invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft {
                        self.finalize_invoice_tx(
                            conn,
                            &Actor::System,
                            invoice_id,
                            tenant_id,
                            false,
                            &None,
                        )
                        .await?;
                    }
                    Ok::<_, Report<StoreError>>(transaction)
                }
                .scope_boxed()
            })
            .await?;

        if transaction.status == crate::domain::PaymentStatusEnum::Settled {
            self.activate_pending_slot_transactions(tenant_id, invoice_id, at_ts)
                .await?;
        }
        let current = self
            .store
            .get_active_slots_value_with_conn(
                &mut self.store.get_conn().await?,
                tenant_id,
                subscription_id,
                unit_name,
                None,
            )
            .await?;
        Ok((transaction, current as i32, next_action))
    }

    /// A slot upgrade whose payment was refused leaves nothing billed: the decline is recorded
    /// on its row, the pending slots are dropped and the unnumbered draft is closed.
    async fn unwind_slot_upgrade(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        row: &diesel_models::payments::PaymentTransactionRow,
        declined: Option<(crate::domain::payment_transactions::PaymentIntent, i64)>,
    ) -> StoreResult<()> {
        let declined = &declined;
        self.store
            .transaction(|conn| {
                async move {
                    let lock = diesel_models::invoices::InvoiceRow::lock_customer_of(
                        conn, tenant_id, invoice_id,
                    )
                    .await?;
                    match declined {
                        Some((intent, _)) => {
                            diesel_models::payments::PaymentTransactionRowPatch {
                                id: row.id,
                                provider_transaction_id: Some(
                                    Some(intent.external_id.clone()).filter(|id| !id.is_empty()),
                                ),
                                status: Some(intent.status.clone().into()),
                                error_type: Some(intent.last_payment_error.clone()),
                                processed_at: Some(intent.processed_at),
                                ..Default::default()
                            }
                            .patch_unsent(conn, tenant_id, row.id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        }
                        None => {
                            diesel_models::payments::PaymentTransactionRow::delete_unsent_charge(
                                conn, row.id, tenant_id,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        }
                    }
                    diesel_models::slot_transactions::SlotTransactionRow::delete_pending_for_invoice(
                        conn, &lock, invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    // A webhook may have issued it meanwhile: only a draft is closed.
                    let invoice = diesel_models::invoices::InvoiceRow::find_by_id(
                        conn, tenant_id, invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    if invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft {
                        diesel_models::invoices::InvoiceRow::close(conn, invoice_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    Ok(())
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

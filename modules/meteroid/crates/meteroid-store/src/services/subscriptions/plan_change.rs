use crate::StoreResult;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::price_components::resolve_legacy_subscription_fee;
use crate::domain::prices::{
    LegacyPricingData, Price, Pricing, extract_legacy_pricing, fee_type_billing_period,
    resolve_subscription_fee,
};
use crate::domain::products::Product;
use crate::domain::scheduled_events::{
    ComponentMapping, ScheduledEvent, ScheduledEventData, ScheduledEventNew,
};
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, ImmediatePlanChangeResult, MatchedComponent, PlanChangeMode,
    PlanChangePreview, PlanChangePreviewExtended, ProrationSummary, RemovedComponent,
};
use crate::domain::subscription_components::{
    ComponentParameterization, ComponentParameters, SubscriptionComponent,
    SubscriptionComponentNew, SubscriptionComponentNewInternal, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::subscriptions::proration::{calculate_proration, detect_change_direction};
use crate::services::subscriptions::utils::calculate_mrr;
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::ids::{PlanVersionId, PriceComponentId, ProductId, SubscriptionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum TargetPricing {
    V2 {
        product_id: ProductId,
        prices: Vec<Price>,
    },
    Legacy(LegacyPricingData),
}

struct TargetComponent {
    id: PriceComponentId,
    name: String,
    pricing: TargetPricing,
}

impl TargetComponent {
    fn product_id(&self) -> Option<ProductId> {
        match &self.pricing {
            TargetPricing::V2 { product_id, .. } => Some(*product_id),
            TargetPricing::Legacy(_) => None,
        }
    }
}

struct ValidatedPlanChangeContext {
    subscription_details: crate::domain::SubscriptionDetails,
    target_components: Vec<TargetComponent>,
    products: HashMap<ProductId, Product>,
}

async fn load_validated_plan_change_context(
    conn: &mut PgConn,
    store: &crate::Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    new_plan_version_id: PlanVersionId,
) -> StoreResult<ValidatedPlanChangeContext> {
    let subscription_details = store
        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
        .await?;

    validate_subscription_for_plan_change(&subscription_details.subscription.status)?;

    if subscription_details.subscription.plan_version_id == new_plan_version_id {
        return Err(Report::new(StoreError::InvalidArgument(
            "Cannot change to the current plan version".to_string(),
        )));
    }

    let target_plan = PlanRow::get_with_version(conn, new_plan_version_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let target_version = target_plan.version.ok_or_else(|| {
        Report::new(StoreError::ValueNotFound(
            "Target plan version not found".to_string(),
        ))
    })?;

    if target_version.is_draft_version {
        return Err(Report::new(StoreError::InvalidArgument(
            "Cannot switch to a draft plan version".to_string(),
        )));
    }

    if target_version.currency != subscription_details.subscription.currency {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Currency mismatch: subscription uses {} but target plan uses {}",
            subscription_details.subscription.currency, target_version.currency
        ))));
    }

    let target_components = load_target_components_with_prices(
        conn,
        tenant_id,
        new_plan_version_id,
        target_version.currency.clone(),
    )
    .await?;

    let products = load_products_for_components(
        conn,
        tenant_id,
        &subscription_details.price_components,
        &target_components,
    )
    .await?;

    Ok(ValidatedPlanChangeContext {
        subscription_details,
        target_components,
        products,
    })
}

impl Services {
    pub(in crate::services) async fn schedule_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ScheduledEvent> {
        self.store
            .transaction(|conn| {
                async move {
                    // Lock the subscription row to prevent concurrent plan change scheduling
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;

                    let ctx = load_validated_plan_change_context(
                        conn,
                        &self.store,
                        tenant_id,
                        subscription_id,
                        new_plan_version_id,
                    )
                    .await?;

                    // Cancel all pending lifecycle events (plan change, cancellation, pause, etc.)
                    ScheduledEventRow::cancel_pending_lifecycle_events(
                        conn,
                        subscription_id,
                        &tenant_id,
                        "Replaced by new plan change",
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // Build component mappings
                    let component_mappings = build_component_mappings(
                        &ctx.subscription_details.price_components,
                        &ctx.target_components,
                        &ctx.products,
                        &component_params,
                    )?;

                    // Schedule at current_period_end
                    let effective_date = ctx
                        .subscription_details
                        .subscription
                        .current_period_end
                        .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Subscription has no current_period_end".to_string(),
                        ))
                    })?;

                    let events = self
                        .store
                        .schedule_events(
                            conn,
                            vec![ScheduledEventNew {
                                subscription_id,
                                tenant_id,
                                scheduled_time: effective_date.and_time(NaiveTime::MIN),
                                event_data: ScheduledEventData::ApplyPlanChange {
                                    source_plan_version_id: Some(
                                        ctx.subscription_details.subscription.plan_version_id,
                                    ),
                                    new_plan_version_id,
                                    component_mappings,
                                },
                                source: "api".to_string(),
                            }],
                        )
                        .await?;

                    events
                        .into_iter()
                        .next()
                        .ok_or_else(|| Report::new(StoreError::InsertError))
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn preview_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
        mode: Option<PlanChangeMode>,
    ) -> StoreResult<PlanChangePreviewExtended> {
        let mut conn = self.store.get_conn().await?;

        let ctx = load_validated_plan_change_context(
            &mut conn,
            &self.store,
            tenant_id,
            subscription_id,
            new_plan_version_id,
        )
        .await?;

        let is_immediate = matches!(mode, Some(PlanChangeMode::Immediate));

        let effective_date = if is_immediate {
            chrono::Utc::now().naive_utc().date()
        } else {
            ctx.subscription_details
                .subscription
                .current_period_end
                .unwrap_or(ctx.subscription_details.subscription.current_period_start)
        };

        let mut preview = build_plan_change_preview(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            &component_params,
            effective_date,
        )?;

        resolve_preview_slot_counts(&mut conn, tenant_id, subscription_id, &mut preview).await?;

        let precision = crate::constants::Currencies::resolve_currency_precision(
            &ctx.subscription_details.subscription.currency,
        )
        .unwrap_or(2);

        let change_direction = detect_change_direction(
            &preview.matched,
            &preview.added,
            &preview.removed,
            precision,
        );

        let proration = if is_immediate {
            let period_start = ctx.subscription_details.subscription.current_period_start;
            let period_end = ctx
                .subscription_details
                .subscription
                .current_period_end
                .unwrap_or(period_start);

            if effective_date < period_start || effective_date > period_end {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Effective date {} is outside current period [{}, {}]",
                    effective_date, period_start, period_end
                ))));
            }

            let result = calculate_proration(
                &preview.matched,
                &preview.added,
                &preview.removed,
                period_start,
                period_end,
                effective_date,
                precision,
            );

            let days_in_period = (period_end - period_start).num_days() as u32;
            let days_remaining = (period_end - effective_date).num_days() as u32;

            Some(ProrationSummary {
                credits_total_cents: result
                    .lines
                    .iter()
                    .filter(|l| l.is_credit)
                    .map(|l| l.amount_cents)
                    .sum(),
                charges_total_cents: result
                    .lines
                    .iter()
                    .filter(|l| !l.is_credit)
                    .map(|l| l.amount_cents)
                    .sum(),
                net_amount_cents: result.net_amount_cents,
                proration_factor: result.proration_factor,
                days_remaining,
                days_in_period,
            })
        } else {
            None
        };

        Ok(PlanChangePreviewExtended {
            preview,
            proration,
            change_direction,
        })
    }

    pub(in crate::services) async fn cancel_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;

                    let pending_events = ScheduledEventRow::get_pending_events_for_subscription(
                        conn,
                        subscription_id,
                        &tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let plan_change_event = pending_events.iter().find(|e| {
                        e.event_type
                            == diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange
                    });

                    match plan_change_event {
                        Some(event) => {
                            ScheduledEventRow::cancel_event(conn, &event.id, "Cancelled by user")
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            Ok(())
                        }
                        None => Err(Report::new(StoreError::ValueNotFound(
                            "No pending plan change found for this subscription".to_string(),
                        ))),
                    }
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn cancel_scheduled_event(
        &self,
        event_id: common_domain::ids::ScheduledEventId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;

        let event = ScheduledEventRow::get_by_id(&mut conn, event_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if event.subscription_id != subscription_id {
            return Err(Report::new(StoreError::InvalidArgument(
                "Event does not belong to the specified subscription".to_string(),
            )));
        }

        if !matches!(
            event.status,
            diesel_models::enums::ScheduledEventStatus::Pending
        ) {
            return Err(Report::new(StoreError::InvalidArgument(
                "Only pending events can be cancelled".to_string(),
            )));
        }

        // Only user-initiated lifecycle events can be cancelled via API
        if !matches!(
            event.event_type,
            diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange
                | diesel_models::enums::ScheduledEventTypeEnum::CancelSubscription
                | diesel_models::enums::ScheduledEventTypeEnum::PauseSubscription
        ) {
            return Err(Report::new(StoreError::InvalidArgument(
                "This event type cannot be cancelled".to_string(),
            )));
        }

        ScheduledEventRow::cancel_event(&mut conn, &event_id, "Cancelled by user")
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    pub(in crate::services) async fn apply_plan_change_immediate(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        let today = chrono::Utc::now().naive_utc().date();
        self.apply_plan_change_immediate_at(
            subscription_id,
            tenant_id,
            new_plan_version_id,
            component_params,
            today,
        )
        .await
    }

    pub(crate) async fn apply_plan_change_immediate_at(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
        change_date: NaiveDate,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        self.store
            .transaction(|conn| {
                async move {
                    let prepared = self
                        .prepare_plan_change_tx(
                            conn,
                            subscription_id,
                            tenant_id,
                            new_plan_version_id,
                            &component_params,
                            change_date,
                        )
                        .await?;

                    let is_free_trial = prepared.is_free_trial();

                    let should_prorate = !is_free_trial && prepared.proration.net_amount_cents != 0;

                    let adjustment_invoice_id = if should_prorate {
                        let invoice = self
                            .create_adjustment_invoice(
                                conn,
                                tenant_id,
                                &prepared.subscription_details.subscription,
                                &prepared.subscription_details.customer,
                                &prepared.proration,
                            )
                            .await?;
                        if let Some(inv) = &invoice {
                            self.finalize_invoice_tx(conn, inv.id, tenant_id, false, &None)
                                .await?;
                        }
                        invoice.map(|inv| inv.id)
                    } else {
                        None
                    };

                    self.execute_plan_change_tx(
                        conn,
                        &prepared,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;

                    // Free trial: create first invoice at new plan rate (optimistic)
                    let first_invoice_id = if is_free_trial {
                        use crate::services::InvoiceBillingMode;

                        let invoice = self
                            .bill_subscription_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                InvoiceBillingMode::Immediate,
                            )
                            .await?;
                        invoice.map(|inv| inv.invoice.id)
                    } else {
                        None
                    };

                    Ok(ImmediatePlanChangeResult {
                        adjustment_invoice_id,
                        first_invoice_id,
                        effective_date: change_date,
                    })
                }
                .scope_boxed()
            })
            .await
    }

    /// Prepare phase: validates, loads data, calculates proration.
    /// Must be called within a transaction.
    pub(in crate::services) async fn prepare_plan_change_tx(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
        self.prepare_plan_change_inner(
            conn,
            subscription_id,
            tenant_id,
            new_plan_version_id,
            component_params,
            change_date,
        )
        .await
    }

    /// Read-only version of prepare (no row lock). For previews/GetCheckout.
    pub(in crate::services) async fn prepare_plan_change_readonly(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        self.prepare_plan_change_inner(
            conn,
            subscription_id,
            tenant_id,
            new_plan_version_id,
            component_params,
            change_date,
        )
        .await
    }

    async fn prepare_plan_change_inner(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        let ctx = load_validated_plan_change_context(
            conn,
            &self.store,
            tenant_id,
            subscription_id,
            new_plan_version_id,
        )
        .await?;

        let mut preview = build_plan_change_preview(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            component_params,
            change_date,
        )?;

        resolve_preview_slot_counts(conn, tenant_id, subscription_id, &mut preview).await?;

        let precision = crate::constants::Currencies::resolve_currency_precision(
            &ctx.subscription_details.subscription.currency,
        )
        .unwrap_or(2);

        let direction = detect_change_direction(
            &preview.matched,
            &preview.added,
            &preview.removed,
            precision,
        );

        let period_start = ctx.subscription_details.subscription.current_period_start;
        let period_end = ctx
            .subscription_details
            .subscription
            .current_period_end
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Subscription has no current_period_end".to_string(),
                ))
            })?;

        if change_date < period_start || change_date > period_end {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Change date {} is outside current period [{}, {}]",
                change_date, period_start, period_end
            ))));
        }

        let proration = calculate_proration(
            &preview.matched,
            &preview.added,
            &preview.removed,
            period_start,
            period_end,
            change_date,
            precision,
        );

        let component_mappings = build_component_mappings(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            component_params,
        )?;

        Ok(PreparedPlanChange {
            subscription_details: ctx.subscription_details,
            component_mappings,
            proration,
            direction,
            precision,
        })
    }

    /// Execute phase: applies the plan change (component rotation, MRR recalc, event).
    pub(crate) async fn execute_plan_change_tx(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<()> {
        ScheduledEventRow::cancel_pending_lifecycle_events(
            conn,
            subscription_id,
            &tenant_id,
            "Replaced by immediate plan change",
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let component_mappings = &prepared.component_mappings;

        let new_period =
            ComponentMapping::derive_billing_period(component_mappings).map(|p| p.into());

        SubscriptionRow::update_plan_version(
            conn,
            &subscription_id,
            &tenant_id,
            new_plan_version_id,
            new_period.clone(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut components_to_close = Vec::new();
        let mut components_to_insert: Vec<SubscriptionComponentRowNew> = Vec::new();

        for mapping in component_mappings {
            match mapping {
                ComponentMapping::Matched {
                    current_component_id,
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                } => {
                    components_to_close.push(*current_component_id);
                    let row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                        subscription_id,
                        internal: SubscriptionComponentNewInternal {
                            price_component_id: Some(*target_component_id),
                            product_id: Some(*product_id),
                            name: name.clone(),
                            period: *period,
                            fee: fee.clone(),
                            is_override: false,
                            price_id: Some(*price_id),
                            effective_from: change_date,
                        },
                    }
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidArgument(
                            "Failed to convert matched component for plan change".to_string(),
                        )
                    })?;
                    components_to_insert.push(row_new);
                }
                ComponentMapping::Added {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                } => {
                    let row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                        subscription_id,
                        internal: SubscriptionComponentNewInternal {
                            price_component_id: Some(*target_component_id),
                            product_id: *product_id,
                            name: name.clone(),
                            period: *period,
                            fee: fee.clone(),
                            is_override: false,
                            price_id: *price_id,
                            effective_from: change_date,
                        },
                    }
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidArgument(
                            "Failed to convert new component for plan change".to_string(),
                        )
                    })?;
                    components_to_insert.push(row_new);
                }
                ComponentMapping::Removed {
                    current_component_id,
                } => {
                    components_to_close.push(*current_component_id);
                }
            }
        }

        SubscriptionComponentRow::close_components(conn, &components_to_close, change_date)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if !components_to_insert.is_empty() {
            let refs: Vec<&SubscriptionComponentRowNew> = components_to_insert.iter().collect();
            SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        for mapping in component_mappings {
            use crate::domain::slot_transactions::SlotTransactionNewInternal;
            if let ComponentMapping::Added { fee, .. } = mapping
                && let Some(tx) = SlotTransactionNewInternal::from_fee(fee, change_date)
            {
                tx.into_row(subscription_id)
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
        }

        let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id,
            event_type: diesel_models::enums::SubscriptionEventType::Switch,
            details: Some(serde_json::json!({
                "new_plan_version_id": new_plan_version_id.to_string(),
                "mode": "immediate",
            })),
            created_at: chrono::Utc::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: change_date,
        };
        sub_event
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let sub_details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;

        let component_mrr: i64 = calculate_components_mrr_with_slots(
            conn,
            tenant_id,
            subscription_id,
            &sub_details.price_components,
            prepared.precision,
        )
        .await?;

        let add_on_mrr: i64 = sub_details
            .add_ons
            .iter()
            .map(|a| calculate_mrr(&a.fee, &a.period, prepared.precision) * a.quantity as i64)
            .sum();

        let new_mrr = component_mrr + add_on_mrr;
        let old_mrr = prepared.subscription_details.subscription.mrr_cents as i64;
        let mrr_delta = new_mrr - old_mrr;

        if mrr_delta != 0 {
            SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        // Trial → Active transition
        if prepared.is_trial() {
            use diesel_models::enums::CycleActionEnum;
            use diesel_models::subscriptions::SubscriptionCycleRowPatch;

            if prepared.is_free_trial() {
                // Free trial: billing starts fresh from change_date.
                // Reset billing_start_date and anchor to the change_date so the
                // invoice computation aligns with the new period.
                let new_anchor = change_date.day();
                let billing_period: crate::domain::enums::BillingPeriodEnum = new_period
                    .map(Into::into)
                    .unwrap_or(prepared.subscription_details.subscription.period);
                let period_range =
                    calculate_advance_period_range(change_date, new_anchor, true, &billing_period);

                let patch = SubscriptionCycleRowPatch {
                    id: subscription_id,
                    tenant_id,
                    status: Some(SubscriptionStatusEnum::Active.into()),
                    cycle_index: Some(0),
                    next_cycle_action: Some(Some(CycleActionEnum::RenewSubscription)),
                    current_period_start: Some(change_date),
                    current_period_end: Some(Some(period_range.end)),
                    pending_checkout: None,
                    processing_started_at: None,
                    billing_start_date: Some(change_date),
                    billing_day_anchor: Some(new_anchor as i16),
                };
                patch.patch(conn).await?;

                log::info!(
                    "Free trial ended via plan change for subscription {}: period [{}, {}]",
                    subscription_id,
                    change_date,
                    period_range.end,
                );
            } else {
                // Paid trial: keep billing period (already being billed), just transition status.
                // Cancel the EndTrial scheduled event (this IS a real event for paid trials).
                ScheduledEventRow::cancel_pending_subscription_events(
                    conn,
                    subscription_id,
                    &tenant_id,
                    "Plan change during paid trial",
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let patch = SubscriptionCycleRowPatch {
                    id: subscription_id,
                    tenant_id,
                    status: Some(SubscriptionStatusEnum::Active.into()),
                    cycle_index: None,
                    next_cycle_action: None,
                    current_period_start: None,
                    current_period_end: None,
                    pending_checkout: None,
                    processing_started_at: None,
                    billing_start_date: None,
                    billing_day_anchor: None,
                };
                patch.patch(conn).await?;

                log::info!(
                    "Paid trial ended via plan change for subscription {}",
                    subscription_id,
                );
            }
        }

        log::info!(
            "Applied immediate plan change for subscription {}: plan_version={}, direction={:?}, net_proration={}",
            subscription_id,
            new_plan_version_id,
            prepared.direction,
            prepared.proration.net_amount_cents,
        );

        Ok(())
    }
}

/// Intermediate struct holding validated and computed data from the prepare phase.
pub(crate) struct PreparedPlanChange {
    pub subscription_details: crate::domain::SubscriptionDetails,
    component_mappings: Vec<ComponentMapping>,
    pub proration: crate::domain::subscription_changes::ProrationResult,
    direction: ChangeDirection,
    precision: u8,
}

impl PreparedPlanChange {
    pub fn is_free_trial(&self) -> bool {
        self.subscription_details.subscription.status == SubscriptionStatusEnum::TrialActive
            && self
                .subscription_details
                .trial_config
                .as_ref()
                .is_some_and(|t| t.is_free)
    }

    pub fn is_trial(&self) -> bool {
        self.subscription_details.subscription.status == SubscriptionStatusEnum::TrialActive
    }

    /// Build a virtual SubscriptionDetails representing the post-trial-change state.
    /// Used with compute_invoice to get exact first-period amount (coupons, tiers, etc.).
    pub fn build_trial_change_preview(
        &self,
        change_date: NaiveDate,
        new_plan_version_id: PlanVersionId,
    ) -> StoreResult<crate::domain::SubscriptionDetails> {
        use crate::domain::scheduled_events::ComponentMapping;
        use common_domain::ids::{BaseId, SubscriptionPriceComponentId};

        let component_mappings = &self.component_mappings;

        let subscription_id = self.subscription_details.subscription.id;

        let subscription_components: Vec<SubscriptionComponent> = component_mappings
            .iter()
            .filter_map(|m| match m {
                ComponentMapping::Matched {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                    ..
                } => Some(SubscriptionComponent {
                    id: SubscriptionPriceComponentId::new(),
                    price_component_id: Some(*target_component_id),
                    product_id: Some(*product_id),
                    subscription_id,
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    price_id: Some(*price_id),
                    effective_from: change_date,
                    effective_to: None,
                }),
                ComponentMapping::Added {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                } => Some(SubscriptionComponent {
                    id: SubscriptionPriceComponentId::new(),
                    price_component_id: Some(*target_component_id),
                    product_id: *product_id,
                    subscription_id,
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    price_id: *price_id,
                    effective_from: change_date,
                    effective_to: None,
                }),
                ComponentMapping::Removed { .. } => None,
            })
            .collect();

        let billing_period = subscription_components
            .iter()
            .find_map(|c| c.period.as_billing_period_opt())
            .unwrap_or(self.subscription_details.subscription.period);

        let new_anchor = change_date.day() as u16;
        let period_range =
            calculate_advance_period_range(change_date, new_anchor as u32, true, &billing_period);

        let mut virtual_sub = self.subscription_details.subscription.clone();
        virtual_sub.status = SubscriptionStatusEnum::Active;
        virtual_sub.plan_version_id = new_plan_version_id;
        virtual_sub.billing_day_anchor = new_anchor;
        virtual_sub.billing_start_date = Some(change_date);
        virtual_sub.current_period_start = change_date;
        virtual_sub.current_period_end = Some(period_range.end);
        virtual_sub.cycle_index = Some(0);
        virtual_sub.period = billing_period;
        virtual_sub.trial_duration = None;
        virtual_sub.pending_checkout = false;

        Ok(crate::domain::SubscriptionDetails {
            subscription: virtual_sub,
            price_components: subscription_components,
            add_ons: Vec::new(),
            trial_config: None,
            invoicing_entity: self.subscription_details.invoicing_entity.clone(),
            customer: self.subscription_details.customer.clone(),
            schedules: Vec::new(),
            applied_coupons: self.subscription_details.applied_coupons.clone(),
            metrics: self.subscription_details.metrics.clone(),
            checkout_url: None,
            pending_events: Vec::new(),
        })
    }
}

fn validate_subscription_for_plan_change(status: &SubscriptionStatusEnum) -> StoreResult<()> {
    match status {
        SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive => Ok(()),
        _ => Err(Report::new(StoreError::InvalidArgument(format!(
            "Cannot schedule plan change for subscription in {:?} status",
            status
        )))),
    }
}

/// Load target components with prices. For v1 components (no v2 prices, Row has legacy_fee),
/// extract legacy pricing data (no fake IDs). Returns components with TargetPricing.
async fn load_target_components_with_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    currency: String,
) -> StoreResult<Vec<TargetComponent>> {
    let component_rows =
        PriceComponentRow::list_by_plan_version_id(conn, tenant_id, plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let component_ids: Vec<PriceComponentId> = component_rows.iter().map(|c| c.id).collect();

    // Load v2 prices via plan_component_price join table
    let mut prices_by_component: HashMap<PriceComponentId, Vec<Price>> = HashMap::new();
    if !component_ids.is_empty() {
        let pcp_rows = PlanComponentPriceRow::list_by_component_ids(conn, &component_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if !pcp_rows.is_empty() {
            let price_ids: Vec<_> = pcp_rows.iter().map(|pcp| pcp.price_id).collect();
            let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let prices_by_id: HashMap<_, _> = price_rows
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Price::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<_, _>>()?;

            for pcp in &pcp_rows {
                if let Some(price) = prices_by_id.get(&pcp.price_id) {
                    prices_by_component
                        .entry(pcp.plan_component_id)
                        .or_default()
                        .push(price.clone());
                }
            }
        }
    }

    // Extract legacy pricing for v1 components
    let mut legacy_by_component: HashMap<PriceComponentId, LegacyPricingData> = HashMap::new();
    for row in &component_rows {
        if !prices_by_component.contains_key(&row.id)
            && let Some(legacy_json) = &row.legacy_fee
        {
            let legacy = extract_legacy_pricing(legacy_json, currency.clone())?;
            legacy_by_component.insert(row.id, legacy);
        }
    }

    let components = component_rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            if let Some(prices) = prices_by_component.remove(&id) {
                let product_id = row.product_id.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "V2 component {} has no product_id",
                        row.name
                    )))
                })?;
                Ok(TargetComponent {
                    id: row.id,
                    name: row.name,
                    pricing: TargetPricing::V2 { product_id, prices },
                })
            } else if let Some(legacy) = legacy_by_component.remove(&id) {
                Ok(TargetComponent {
                    id: row.id,
                    name: row.name,
                    pricing: TargetPricing::Legacy(legacy),
                })
            } else {
                Err(Report::new(StoreError::InvalidArgument(format!(
                    "Component {} has no pricing data",
                    row.name
                ))))
            }
        })
        .collect::<Result<Vec<_>, Report<StoreError>>>()?;

    Ok(components)
}

async fn load_products_for_components(
    conn: &mut PgConn,
    tenant_id: TenantId,
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
) -> StoreResult<HashMap<ProductId, Product>> {
    let product_ids: Vec<ProductId> = target_components
        .iter()
        .filter_map(|c| c.product_id())
        .chain(current_components.iter().filter_map(|c| c.product_id))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if product_ids.is_empty() {
        return Ok(HashMap::new());
    }

    ProductRow::list_by_ids(conn, &product_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|r| Product::try_from(r).map(|p| (p.id, p)))
        .collect::<Result<HashMap<_, _>, _>>()
}

/// Resolve the fee and period for a target component.
///
/// Period selection priority:
/// 1. Explicit parametrization (from API caller) → use its billing_period
/// 2. Ported period from matched current component (same product_id) → reuse current cadence
/// 3. Single price → use it
/// 4. Multiple prices with no selection → error
fn resolve_target_fee(
    target: &TargetComponent,
    products: &HashMap<ProductId, Product>,
    explicit_params: Option<&ComponentParameters>,
    ported_period: Option<SubscriptionFeeBillingPeriod>,
) -> Result<
    (
        SubscriptionFeeBillingPeriod,
        crate::domain::SubscriptionFee,
        Option<common_domain::ids::PriceId>,
    ),
    Report<StoreError>,
> {
    match &target.pricing {
        TargetPricing::V2 { product_id, prices } => {
            let product = products.get(product_id).ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Product {} not found for component {}",
                    product_id, target.name
                )))
            })?;

            let fee_structure = &product.fee_structure;

            let price = select_target_price(prices, explicit_params, ported_period, &target.name)?;

            let fee = resolve_subscription_fee(fee_structure, &price.pricing, explicit_params)?;
            let period = fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());

            Ok((period, fee, Some(price.id)))
        }
        TargetPricing::Legacy(legacy) => {
            let params = build_legacy_params(explicit_params, ported_period);
            let resolved = resolve_legacy_subscription_fee(legacy, params.as_ref())?;
            Ok((resolved.period, resolved.fee, None))
        }
    }
}

/// Select the right price from a V2 component's price list.
fn select_target_price<'a>(
    prices: &'a [Price],
    explicit_params: Option<&ComponentParameters>,
    ported_period: Option<SubscriptionFeeBillingPeriod>,
    component_name: &str,
) -> Result<&'a Price, Report<StoreError>> {
    // Single price → always use it
    if prices.len() == 1 {
        return Ok(&prices[0]);
    }

    let mut candidates: Vec<&Price> = prices.iter().collect();

    // 1. Filter by explicit params
    if let Some(params) = explicit_params {
        if let Some(bp) = &params.billing_period {
            let target = bp.as_subscription_billing_period();
            candidates.retain(|p| p.cadence.as_subscription_billing_period() == target);
        }
        if let Some(cap) = params.committed_capacity {
            candidates.retain(|p| match &p.pricing {
                Pricing::Capacity { included, .. } => *included == cap,
                _ => true,
            });
        }
    }

    // 2. If still ambiguous, try ported period from matched current component
    if candidates.len() > 1
        && let Some(period) = ported_period
    {
        let by_period: Vec<&Price> = candidates
            .iter()
            .filter(|p| p.cadence.as_subscription_billing_period() == period)
            .copied()
            .collect();
        if !by_period.is_empty() {
            candidates = by_period;
        }
    }

    match candidates.len() {
        1 => Ok(candidates[0]),
        0 => Err(Report::new(StoreError::InvalidArgument(format!(
            "No matching price found for component '{}'",
            component_name
        )))),
        _ => Err(Report::new(StoreError::InvalidArgument(format!(
            "Multiple prices match for component '{}' — provide billing_period or committed_capacity to disambiguate",
            component_name
        )))),
    }
}

/// Build ComponentParameters for legacy pricing resolution from explicit params or ported period.
fn build_legacy_params(
    explicit_params: Option<&ComponentParameters>,
    ported_period: Option<SubscriptionFeeBillingPeriod>,
) -> Option<ComponentParameters> {
    if explicit_params.is_some() {
        return explicit_params.cloned();
    }
    ported_period
        .and_then(|p| p.as_billing_period_opt())
        .map(|bp| ComponentParameters {
            initial_slot_count: None,
            billing_period: Some(bp),
            committed_capacity: None,
        })
}

fn build_component_mappings(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    products: &HashMap<ProductId, Product>,
    component_params: &[ComponentParameterization],
) -> StoreResult<Vec<ComponentMapping>> {
    let params_by_id: HashMap<PriceComponentId, &ComponentParameters> = component_params
        .iter()
        .map(|p| (p.component_id, &p.parameters))
        .collect();

    let mut mappings = Vec::new();
    let mut matched_current_ids = std::collections::HashSet::new();

    // Match target components to current by product_id
    for target in target_components {
        let explicit_params = params_by_id.get(&target.id).copied();

        // Try to find a matching current component by product_id (v2 only)
        let matched_current = target.product_id().and_then(|target_pid| {
            current_components
                .iter()
                .find(|c| c.product_id == Some(target_pid))
        });

        if let Some(current) = matched_current {
            matched_current_ids.insert(current.id);

            let ported_period = current.period;

            let (period, fee, price_id) =
                resolve_target_fee(target, products, explicit_params, Some(ported_period))?;

            let price_id = price_id.ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Matched component {} has no price_id",
                    target.name
                )))
            })?;

            mappings.push(ComponentMapping::Matched {
                current_component_id: current.id,
                target_component_id: target.id,
                product_id: target.product_id().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Matched component {} has no product_id",
                        target.name
                    )))
                })?,
                price_id,
                name: target.name.clone(),
                fee,
                period,
            });
            continue;
        }

        // No match found — this is a new component
        let (period, fee, price_id) = resolve_target_fee(target, products, explicit_params, None)?;

        mappings.push(ComponentMapping::Added {
            target_component_id: target.id,
            product_id: target.product_id(),
            price_id,
            name: target.name.clone(),
            fee,
            period,
        });
    }

    // Current components not matched → removed
    for current in current_components {
        if !matched_current_ids.contains(&current.id) {
            mappings.push(ComponentMapping::Removed {
                current_component_id: current.id,
            });
        }
    }

    Ok(mappings)
}

fn build_plan_change_preview(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    products: &HashMap<ProductId, Product>,
    component_params: &[ComponentParameterization],
    effective_date: chrono::NaiveDate,
) -> StoreResult<PlanChangePreview> {
    let params_by_id: HashMap<PriceComponentId, &ComponentParameters> = component_params
        .iter()
        .map(|p| (p.component_id, &p.parameters))
        .collect();

    let mut matched = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut matched_current_ids = std::collections::HashSet::new();

    for target in target_components {
        let explicit_params = params_by_id.get(&target.id).copied();

        let matched_current = target.product_id().and_then(|target_pid| {
            current_components
                .iter()
                .find(|c| c.product_id == Some(target_pid))
        });

        if let Some(current) = matched_current {
            matched_current_ids.insert(current.id);

            let ported_period = current.period;

            let (new_period, new_fee, _) =
                resolve_target_fee(target, products, explicit_params, Some(ported_period))?;

            matched.push(MatchedComponent {
                product_id: target.product_id().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Matched component {} has no product_id",
                        target.name
                    )))
                })?,
                current_name: current.name.clone(),
                current_fee: current.fee.clone(),
                current_period: current.period,
                new_name: target.name.clone(),
                new_fee,
                new_period,
            });
            continue;
        }

        let (period, fee, _) = resolve_target_fee(target, products, explicit_params, None)?;

        added.push(AddedComponent {
            name: target.name.clone(),
            fee,
            period,
        });
    }

    for current in current_components {
        if !matched_current_ids.contains(&current.id) {
            removed.push(RemovedComponent {
                name: current.name.clone(),
                current_fee: current.fee.clone(),
                current_period: current.period,
            });
        }
    }

    Ok(PlanChangePreview {
        matched,
        added,
        removed,
        effective_date,
    })
}

/// Resolves actual slot counts from `slot_transaction` into the preview's fees.
///
/// `initial_slots` in a `SubscriptionFee::Slot` is only a seed value from subscription creation.
/// The real slot count lives in the `slot_transaction` table. For proration and direction
/// detection we need the actual count, so we patch it into the fee before those calculations.
///
/// For matched components, both `current_fee` and `new_fee` are patched — the customer
/// keeps their seats across plan changes.
async fn resolve_preview_slot_counts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    preview: &mut PlanChangePreview,
) -> StoreResult<()> {
    // Collect unique slot unit names that need resolution
    let mut units: Vec<String> = Vec::new();
    for m in preview.matched.iter() {
        if let SubscriptionFee::Slot { unit, .. } = &m.current_fee
            && !units.contains(unit)
        {
            units.push(unit.clone());
        }
    }
    for r in preview.removed.iter() {
        if let SubscriptionFee::Slot { unit, .. } = &r.current_fee
            && !units.contains(unit)
        {
            units.push(unit.clone());
        }
    }

    // Query actual counts once per unit
    let mut counts: HashMap<String, u32> = HashMap::new();
    for unit in units {
        let actual = SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
            conn,
            tenant_id,
            subscription_id,
            unit.clone(),
            None,
        )
        .await
        .map(|r| r.current_active_slots as u32)
        .unwrap_or(0);
        counts.insert(unit, actual);
    }

    // Patch fees
    fn patch(fee: &mut SubscriptionFee, counts: &HashMap<String, u32>) {
        if let SubscriptionFee::Slot {
            unit,
            initial_slots,
            ..
        } = fee
            && let Some(&actual) = counts.get(unit.as_str())
        {
            *initial_slots = actual;
        }
    }

    for m in &mut preview.matched {
        patch(&mut m.current_fee, &counts);
        patch(&mut m.new_fee, &counts);
    }
    for r in &mut preview.removed {
        patch(&mut r.current_fee, &counts);
    }

    Ok(())
}

/// Calculates MRR for subscription components, using actual slot counts from `slot_transaction`
/// instead of the fee's `initial_slots` (which is just the seed value at creation time).
///
/// `get_subscription_details_with_conn` resolves V2 fees from product/price definitions,
/// which always returns `initial_slots = min_slots`. For correct MRR we need the current
/// slot count from the `slot_transaction` ledger.
pub(crate) async fn calculate_components_mrr_with_slots(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    components: &[crate::domain::subscription_components::SubscriptionComponent],
    precision: u8,
) -> StoreResult<i64> {
    use crate::services::subscriptions::utils::calculate_mrr;

    // Collect unique slot units and query actual counts
    let mut slot_counts: HashMap<String, i64> = HashMap::new();
    for c in components {
        if let SubscriptionFee::Slot { unit, .. } = &c.fee
            && !slot_counts.contains_key(unit)
        {
            let count = SlotTransactionRow::fetch_by_subscription_id_and_unit_locked(
                conn,
                tenant_id,
                subscription_id,
                unit.clone(),
                None,
            )
            .await
            .map(|r| i64::from(r.current_active_slots))
            .map_err(Into::<Report<StoreError>>::into)?;
            slot_counts.insert(unit.clone(), count);
        }
    }

    let mut total_mrr: i64 = 0;
    for c in components {
        match &c.fee {
            SubscriptionFee::Slot {
                unit, unit_rate, ..
            } => {
                let count = slot_counts.get(unit).copied().unwrap_or(0);
                let rate_cents = unit_rate.to_subunit_opt(precision).unwrap_or(0);
                let period_months = i64::from(c.period.as_months());
                if period_months > 0 {
                    total_mrr += (count * rate_cents) / period_months;
                }
            }
            _ => {
                total_mrr += calculate_mrr(&c.fee, &c.period, precision);
            }
        }
    }

    Ok(total_mrr)
}

use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
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
use crate::domain::{SubscriptionAddOn, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::Services;
use crate::services::subscriptions::proration::{
    DayFraction, calculate_proration, component_proration_factor, detect_change_direction,
    pack_charge_line,
};
use crate::services::subscriptions::terminate::removed_add_on;
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, CreditNoteId, PlanVersionId, PriceComponentId, ProductId, SubscriptionId,
    SubscriptionPriceComponentId, TenantId,
};
use common_utils::decimals::ToSubunit;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plan_version_add_ons::PlanVersionAddOnRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::query::credit_inputs;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{BTreeMap, HashMap};

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
    units: crate::domain::credit_units::CreditUnitsById,
    target_minimum: Option<crate::domain::plan_minimums::PlanMinimum>,
    compatible_add_on_ids: Vec<AddOnId>,
}

/// A minimum commitment is a term of the period: it changes at the period's end, never within.
const MINIMUM_COMMITMENT_AT_PERIOD_END: &str = "This subscription or the target plan has a minimum commitment; plan changes take effect at the end of the period.";

impl ValidatedPlanChangeContext {
    fn immediate_allowed(&self) -> bool {
        self.subscription_details.minimum.is_none() && self.target_minimum.is_none()
    }
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

    // A subscription reads all of its amounts one way, and a change prorates the old plan
    // against the new one: a credit quoted tax-included cannot be netted against a charge
    // quoted net. Like currency, the basis is fixed for the subscription's life.
    if target_version.tax_inclusive != subscription_details.subscription.tax_inclusive {
        let describe = |inclusive: bool| {
            if inclusive {
                "tax-inclusive"
            } else {
                "tax-exclusive"
            }
        };
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Tax basis mismatch: subscription prices are {} but target plan prices are {}",
            describe(subscription_details.subscription.tax_inclusive),
            describe(target_version.tax_inclusive)
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

    let target_minimum =
        crate::repositories::plans::load_plan_minimum(conn, new_plan_version_id).await?;

    let units = crate::repositories::credit_units::load_credit_units(
        conn,
        tenant_id,
        target_components
            .iter()
            .flat_map(|target| match &target.pricing {
                TargetPricing::V2 { prices, .. } => prices.as_slice(),
                TargetPricing::Legacy(_) => &[],
            })
            .flat_map(Price::credit_unit_ids),
    )
    .await?;

    let compatible_add_on_ids =
        PlanVersionAddOnRow::list_by_plan_version_id(conn, new_plan_version_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|pva| pva.add_on_id)
            .collect();

    Ok(ValidatedPlanChangeContext {
        subscription_details,
        target_components,
        products,
        units,
        target_minimum,
        compatible_add_on_ids,
    })
}

impl Services {
    pub(in crate::services) async fn schedule_plan_change(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ScheduledEvent> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let component_params = &component_params;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    refuse_while_a_paid_change_is_pending(conn, tenant_id, subscription_id, None)
                        .await?;

                    let ctx = load_validated_plan_change_context(
                        conn,
                        &self.store,
                        tenant_id,
                        subscription_id,
                        new_plan_version_id,
                    )
                    .await?;
                    let customer_id = ctx.subscription_details.subscription.customer_id;

                    ScheduledEventRow::cancel_pending_lifecycle_events(
                        conn,
                        subscription_id,
                        &tenant_id,
                        "Replaced by new plan change",
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let component_mappings = build_component_mappings(
                        &ctx.subscription_details.price_components,
                        &ctx.target_components,
                        &ctx.products,
                        &ctx.units,
                        component_params,
                    )?;

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
                                created_by: actor.clone(),
                            }],
                        )
                        .await?;

                    let event = events
                        .into_iter()
                        .next()
                        .ok_or_else(|| Report::new(StoreError::InsertError))?;

                    let activity = Activity::new(
                        ActivityType::SubscriptionPlanChangeScheduled,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({
                        "new_plan_version_id": new_plan_version_id.to_string(),
                        "effective_at": event.scheduled_time.to_string(),
                    }))
                    .agg_customer(customer_id);
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    Ok(event)
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
        let immediate_allowed = ctx.immediate_allowed();

        let effective_date = if is_immediate {
            common_domain::clock::now().naive_utc().date()
        } else {
            ctx.subscription_details
                .subscription
                .current_period_end
                .unwrap_or(ctx.subscription_details.subscription.current_period_start)
        };

        if is_immediate {
            drop(ctx);
            let prepared = self
                .prepare_plan_change_inner(
                    &mut conn,
                    subscription_id,
                    tenant_id,
                    new_plan_version_id,
                    &component_params,
                    effective_date,
                )
                .await?;
            prepared.ensure_immediate_allowed()?;
            drop(conn);
            let outgoing = if prepared.is_free_trial() {
                Default::default()
            } else {
                self.estimate_outgoing_packs(&prepared, effective_date)
                    .await?
            };
            let proration = prepared.proration_summary(&outgoing);
            return Ok(PlanChangePreviewExtended {
                preview: prepared.preview,
                proration: Some(proration),
                change_direction: prepared.direction,
                immediate_allowed,
            });
        }

        let mut preview = build_plan_change_preview(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            &ctx.units,
            &component_params,
            effective_date,
            &HashMap::new(),
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

        Ok(PlanChangePreviewExtended {
            preview,
            proration: None,
            change_direction,
            immediate_allowed,
        })
    }

    pub(in crate::services) async fn cancel_plan_change(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    credit_inputs::bump_customer(conn, &lock).await?;

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

                    let event = plan_change_event.ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "No pending plan change found for this subscription".to_string(),
                        ))
                    })?;

                    if !ScheduledEventRow::cancel_event(conn, &event.id, "Cancelled by user")
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                    {
                        return Err(Report::new(StoreError::ValueNotFound(
                            "No pending plan change found for this subscription".to_string(),
                        )));
                    }

                    let customer_id =
                        SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let activity = Activity::new(
                        ActivityType::SubscriptionPlanChangeCancelled,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .agg_customer(customer_id);
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn cancel_scheduled_event(
        &self,
        actor: Actor,
        event_id: common_domain::ids::ScheduledEventId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    // Before the event is read: the lifecycle runs an event holding the customer,
                    // so an event read pending here is still pending when cancelled below.
                    SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let lock = SubscriptionRow::lock_customer_of(conn, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let event = ScheduledEventRow::get_by_id(conn, event_id, &tenant_id)
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

                    if !matches!(
                        event.event_type,
                        diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange
                            | diesel_models::enums::ScheduledEventTypeEnum::CancelSubscription
                            | diesel_models::enums::ScheduledEventTypeEnum::PauseSubscription
                            | diesel_models::enums::ScheduledEventTypeEnum::ApplyAmendment
                    ) {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "This event type cannot be cancelled".to_string(),
                        )));
                    }

                    // Customers may only cancel events they themselves scheduled.
                    // Admin/User/ApiToken/System actors may cancel anything.
                    if actor.is_customer() && !event.created_by_customer {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "This scheduled change can only be modified by your account manager"
                                .to_string(),
                        )));
                    }

                    // PauseSubscription has no dedicated "undo" variant; skip the audit row.
                    let activity_type = match event.event_type {
                        diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange => {
                            Some(ActivityType::SubscriptionPlanChangeCancelled)
                        }
                        diesel_models::enums::ScheduledEventTypeEnum::CancelSubscription => {
                            Some(ActivityType::SubscriptionCancellationUndone)
                        }
                        diesel_models::enums::ScheduledEventTypeEnum::ApplyAmendment => {
                            Some(ActivityType::SubscriptionAmendmentCancelled)
                        }
                        _ => None,
                    };

                    if !ScheduledEventRow::cancel_event(conn, &event_id, "Cancelled by user")
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Only pending events can be cancelled".to_string(),
                        )));
                    }

                    if let Some(activity_type) = activity_type {
                        let customer_id =
                            SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;

                        let activity = Activity::new(
                            activity_type,
                            EntityType::Subscription,
                            subscription_id.as_uuid(),
                        )
                        .agg_customer(customer_id);
                        self.store
                            .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn apply_plan_change_immediate(
        &self,
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        let today = common_domain::clock::now().naive_utc().date();
        self.apply_plan_change_immediate_at(
            typed_actor,
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
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: Vec<ComponentParameterization>,
        change_date: NaiveDate,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            let actor = &actor;
            let component_params = &component_params;
            async move {
                let prepared = services
                    .prepare_plan_change_tx(
                        conn,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        component_params,
                        change_date,
                    )
                    .await?;
                refuse_while_a_paid_change_is_pending(conn, tenant_id, subscription_id, None)
                    .await?;
                let customer_id = prepared.subscription_details.customer.id;

                let is_free_trial = prepared.is_free_trial();

                let (adjustment, credit_note_ids) = services
                    .apply_and_invoice_plan_change_tx(
                        conn,
                        actor,
                        &prepared,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;
                let adjustment_invoice_id = adjustment.map(|invoice| invoice.id);

                // Free trial: create first invoice at new plan rate (optimistic)
                let first_invoice_id = if is_free_trial {
                    use crate::services::InvoiceBillingMode;

                    let invoice = services
                        .bill_subscription_tx(
                            conn,
                            actor,
                            tenant_id,
                            subscription_id,
                            InvoiceBillingMode::Immediate,
                        )
                        .await?;
                    invoice.map(|inv| inv.invoice.id)
                } else {
                    None
                };

                let activity = Activity::new(
                    ActivityType::SubscriptionPlanChanged,
                    EntityType::Subscription,
                    subscription_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "change_date": change_date.to_string(),
                }))
                .agg_customer(customer_id);
                services
                    .store
                    .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Ok(ImmediatePlanChangeResult {
                    adjustment_invoice_id,
                    credit_note_ids,
                    first_invoice_id,
                    effective_date: change_date,
                })
            }
            .scope_boxed()
        })
        .await
    }

    /// Applies the change: the old plan settled first (unused fees credited on notes against the
    /// invoices that billed them, its packs' period credit removed), so that credit never pays
    /// the new plan's charges, then invoiced and finalized. Returns the adjustment invoice.
    pub(crate) async fn apply_and_invoice_plan_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<(Option<crate::domain::Invoice>, Vec<CreditNoteId>)> {
        let subscription = &prepared.subscription_details.subscription;
        let notes = self
            .execute_plan_change_tx(
                conn,
                actor,
                prepared,
                subscription.id,
                subscription.tenant_id,
                new_plan_version_id,
                change_date,
            )
            .await?;
        let charges = prepared.charges();
        if prepared.is_free_trial() || charges.lines.is_empty() {
            return Ok((None, notes));
        }
        let invoice = self
            .create_adjustment_invoice(
                conn,
                subscription.tenant_id,
                subscription,
                &prepared.subscription_details.customer,
                &charges,
            )
            .await?;
        if let Some(invoice) = &invoice {
            self.finalize_invoice_tx(
                conn,
                actor,
                invoice.id,
                subscription.tenant_id,
                false,
                &None,
            )
            .await?;
        }
        Ok((invoice, notes))
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
        let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
        credit_inputs::bump_customer(conn, &lock).await?;
        let prepared = self
            .prepare_plan_change_inner(
                conn,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
                change_date,
            )
            .await?;
        prepared.ensure_immediate_allowed()?;
        Ok(prepared)
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

        // Ids the new components are inserted with, stamped on the lines billing them so a later
        // change in the period credits those lines.
        let new_component_ids: HashMap<PriceComponentId, SubscriptionPriceComponentId> = ctx
            .target_components
            .iter()
            .map(|target| (target.id, SubscriptionPriceComponentId::new()))
            .collect();

        let mut preview = build_plan_change_preview(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            &ctx.units,
            component_params,
            change_date,
            &new_component_ids,
        )?;

        resolve_preview_slot_counts(conn, tenant_id, subscription_id, &mut preview).await?;

        // A component the target bills exactly as before is carried over: nothing is credited
        // or charged for it, and its new row continues the lineage of the lines that billed it.
        let carried: HashMap<PriceComponentId, SubscriptionPriceComponentId> = preview
            .matched
            .iter()
            .filter(|m| m.current_period == m.new_period && same_fee(&m.current_fee, &m.new_fee))
            .filter_map(|m| {
                let key = m.current_net_key.as_deref()?;
                let root = ctx
                    .subscription_details
                    .price_components
                    .iter()
                    .find(|c| c.lineage().to_string() == key)?
                    .lineage();
                Some((m.price_component_id?, root))
            })
            .collect();

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

        // Plan changes never re-charge the new plan's one-time (setup) fees:
        // those are signup-only, billed at subscription creation. Only recurring
        // components prorate on an immediate change.
        let added_recurring: Vec<AddedComponent> = preview
            .added
            .iter()
            .filter(|a| !matches!(a.fee, SubscriptionFee::OneTime { .. }))
            .cloned()
            .collect();

        let changed: Vec<MatchedComponent> = preview
            .matched
            .iter()
            .filter(|m| {
                !m.price_component_id
                    .is_some_and(|t| carried.contains_key(&t))
            })
            .cloned()
            .collect();
        let ending_add_ons: Vec<SubscriptionAddOn> = ctx
            .subscription_details
            .add_ons
            .iter()
            .filter(|a| !ctx.compatible_add_on_ids.contains(&a.add_on_id))
            .cloned()
            .collect();
        let removed: Vec<RemovedComponent> = preview
            .removed
            .iter()
            .cloned()
            .chain(ending_add_ons.iter().map(removed_add_on))
            .collect();
        let mut proration = calculate_proration(
            &changed,
            &added_recurring,
            &removed,
            period_start,
            period_end,
            change_date,
            precision,
        );

        let component_mappings = build_component_mappings(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.products,
            &ctx.units,
            component_params,
        )?;

        let pack_lines = new_pack_lines(
            conn,
            &ctx.subscription_details.subscription,
            &component_mappings,
            &new_component_ids,
            &carried,
            (period_start, period_end),
            change_date,
            precision,
        )
        .await?;
        proration.net_amount_cents += pack_lines.iter().map(|l| l.amount_cents).sum::<i64>();
        proration.lines.extend(pack_lines);

        let free_trial = ctx.subscription_details.subscription.status
            == SubscriptionStatusEnum::TrialActive
            && super::trial_is_free_tx(conn, &ctx.subscription_details.subscription).await?;
        Ok(PreparedPlanChange {
            free_trial,
            subscription_details: ctx.subscription_details,
            component_mappings,
            new_component_ids,
            carried,
            settle_outgoing: true,
            preview,
            proration,
            direction,
            precision,
            target_minimum: ctx.target_minimum,
            compatible_add_on_ids: ctx.compatible_add_on_ids,
            ending_add_ons,
        })
    }

    /// Execute phase: settles what the old plan billed for the rest of the period (credit notes,
    /// pack credit removed), then applies the plan change (component rotation, MRR recalc, event).
    /// Returns the credit notes issued.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_plan_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<Vec<CreditNoteId>> {
        // Free trials settle nothing: no money changed hands.
        let settles_outgoing = !prepared.is_free_trial() && prepared.settle_outgoing;
        let credit_note_ids = if !settles_outgoing {
            Vec::new()
        } else {
            self.settle_outgoing_components_tx(conn, actor, prepared, change_date)
                .await?
        };

        ScheduledEventRow::cancel_pending_lifecycle_events(
            conn,
            subscription_id,
            &tenant_id,
            "Replaced by immediate plan change",
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let component_mappings = &prepared.component_mappings;
        let old_plan_version_id = prepared.subscription_details.subscription.plan_version_id;

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
        self.store
            .hooks
            .signals
            .subscription_plan_changed(
                conn,
                tenant_id,
                prepared.subscription_details.subscription.customer_id,
                subscription_id,
                common_domain::clock::now().naive_utc(),
            )
            .await?;

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
                    let mut row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
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
                    if let Some(id) = prepared.new_component_ids.get(target_component_id) {
                        row_new.id = *id;
                    }
                    row_new.lineage_id = prepared.carried.get(target_component_id).copied();
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
                    let mut row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
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
                    if let Some(id) = prepared.new_component_ids.get(target_component_id) {
                        row_new.id = *id;
                    }
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

        // Refresh the subscription minimum snapshot from the new plan version.
        // Operator-set / quote-sourced overrides (`is_override = true`) are preserved —
        // those represent negotiated contracts that survive plan changes. Only plain
        // plan-snapshots (`is_override = false`) get re-snapped.
        refresh_subscription_minimum_on_plan_change(conn, subscription_id, new_plan_version_id)
            .await?;

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

        let ended_addons = SubscriptionAddOnRow::close_incompatible(
            conn,
            &subscription_id,
            &prepared.compatible_add_on_ids,
            change_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        if !ended_addons.is_empty() {
            log::info!(
                "Ended {} incompatible add-on(s) during plan change for subscription {}: {:?}",
                ended_addons.len(),
                subscription_id,
                ended_addons.iter().map(|a| a.add_on_id).collect::<Vec<_>>(),
            );
        }

        let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id,
            event_type: diesel_models::enums::SubscriptionEventType::Switch,
            details: Some(if settles_outgoing {
                // Marks the change whose ended lines are settled, now or once their invoice is issued.
                serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "old_plan_version_id": old_plan_version_id.to_string(),
                    "mode": "immediate",
                    "outgoing": "settled",
                    "notes": credit_note_ids.iter().map(|id| id.as_uuid().to_string()).collect::<Vec<_>>(),
                    "seats": prepared.outgoing_seats(),
                })
            } else {
                serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "old_plan_version_id": old_plan_version_id.to_string(),
                    "mode": "immediate",
                })
            }),
            created_at: common_domain::clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: change_date,
        };
        sub_event
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut sub_details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;

        let component_lines = calculate_components_mrr_with_slots(
            conn,
            tenant_id,
            subscription_id,
            &sub_details.price_components,
            prepared.precision,
        )
        .await?;

        // Both sides of the delta are net: the stored MRR already is, and the recomputed one is
        // floored by the minimum the change snapshotted and netted of included tax, as at
        // creation and at a cycle's plan change.
        let new_mrr = self
            .restate_subscription_mrr(
                conn,
                &mut sub_details,
                component_lines,
                prepared.precision,
                change_date,
            )
            .await?;
        let old_mrr = prepared.subscription_details.subscription.mrr_cents as i64;
        let mrr_delta = new_mrr - old_mrr;

        if mrr_delta != 0 {
            SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            self.create_movement_mrr_log(conn, tenant_id, subscription_id, change_date, mrr_delta)
                .await?;
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
                    terminated_at: None,
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
                    terminated_at: None,
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

        self.emit_subscription_updated_tx(
            conn,
            tenant_id,
            subscription_id,
            crate::domain::outbox_event::SubscriptionUpdateType::PlanChanged,
        )
        .await?;

        Ok(credit_note_ids)
    }
}

/// Intermediate struct holding validated and computed data from the prepare phase.
pub struct PreparedPlanChange {
    pub subscription_details: crate::domain::SubscriptionDetails,
    component_mappings: Vec<ComponentMapping>,
    new_component_ids: HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    /// Target components carried over unchanged, with the lineage root they continue.
    pub(crate) carried: HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    /// False when an earlier release already netted the old plan's credits on the adjustment.
    settle_outgoing: bool,
    preview: PlanChangePreview,
    /// The old components' credits (negative) and the new components' charges, packs included.
    pub proration: crate::domain::subscription_changes::ProrationResult,
    direction: ChangeDirection,
    precision: u8,
    target_minimum: Option<crate::domain::plan_minimums::PlanMinimum>,
    /// In a trial that bills nothing, as activation decides it (`trial_is_free_tx`).
    free_trial: bool,
    compatible_add_on_ids: Vec<AddOnId>,
    /// The subscription's add-ons the target plan version does not offer: they end at the change.
    pub(crate) ending_add_ons: Vec<SubscriptionAddOn>,
}

impl PreparedPlanChange {
    /// The seats of each unit the old plan had when the change read them: a deferred settlement
    /// cannot read them back once the new plan's seats are recorded under the same unit.
    pub(crate) fn outgoing_seats(&self) -> BTreeMap<String, u32> {
        self.preview
            .matched
            .iter()
            .map(|m| &m.current_fee)
            .chain(self.preview.removed.iter().map(|r| &r.current_fee))
            .filter_map(|fee| match fee {
                SubscriptionFee::Slot {
                    unit,
                    initial_slots,
                    ..
                } => Some((unit.clone(), *initial_slots)),
                _ => None,
            })
            .collect()
    }

    /// What the adjustment invoice bills: the new components' charges, included packs (no
    /// charge, their credit granted when it is issued) among them. Credits go on credit notes.
    pub(crate) fn charges(&self) -> crate::domain::subscription_changes::ProrationResult {
        let lines: Vec<_> = self
            .proration
            .lines
            .iter()
            .filter(|line| line.amount_cents > 0 || line.pack.is_some())
            .cloned()
            .collect();
        crate::domain::subscription_changes::ProrationResult {
            net_amount_cents: lines.iter().map(|line| line.amount_cents).sum(),
            lines,
            ..self.proration.clone()
        }
    }

    /// A change paid on an adjustment invoice an earlier release issued: that invoice netted the
    /// old plan's credits as negative lines and billed no pack, so nothing more is settled; the
    /// change inserts the components it billed.
    pub(crate) fn adopt_legacy_adjustment(&mut self, invoice: &crate::domain::Invoice) {
        self.settle_outgoing = false;
        for line in &invoice.line_items {
            if let (Some(target), Some(id)) = (line.price_component_id, line.sub_component_id)
                && let Some(slot) = self.new_component_ids.get_mut(&target)
            {
                *slot = id;
            }
        }
    }

    pub(crate) fn proration_summary(
        &self,
        outgoing: &crate::services::subscriptions::plan_change_credit::OutgoingEstimate,
    ) -> ProrationSummary {
        use crate::domain::subscription_changes::{PackChange, PackChangeKind};
        let lines = &self.proration.lines;
        // The credits as the notes would issue them: each billed line's unused time, net.
        let pack_credits: i64 = outgoing.packs.iter().map(|pack| pack.fee_credit).sum();
        let credits_total_cents = -(outgoing.fee_credits + pack_credits);
        let charges_total_cents: i64 = lines
            .iter()
            .filter(|line| line.amount_cents > 0)
            .map(|line| line.amount_cents)
            .sum();
        let (period_start, period_end, change_date) = (
            self.proration.period_start,
            self.proration.period_end,
            self.proration.change_date,
        );
        let mut pack_changes: Vec<PackChange> = lines
            .iter()
            .filter_map(|line| {
                line.pack.as_ref().map(|pack| PackChange {
                    name: line.name.clone(),
                    kind: PackChangeKind::Charge,
                    amount_cents: line.amount_cents,
                    credit_minor: pack.credit_minor,
                    credit_unit: pack.unit.clone(),
                    credit_precision: pack
                        .unit
                        .as_ref()
                        .map_or(self.precision, |unit| unit.precision),
                    promotional: pack.included,
                })
            })
            .collect();
        pack_changes.extend(outgoing.packs.iter().flat_map(|pack| pack.changes()));
        ProrationSummary {
            credits_total_cents,
            charges_total_cents,
            net_amount_cents: charges_total_cents + credits_total_cents,
            // Credits are issued on credit notes, never netted into a charge.
            net_credit_cents: credits_total_cents,
            // Plan changes don't introduce new arrears components, so no deferred amount.
            arrears_charge_cents: 0,
            proration_factor: self.proration.proration_factor,
            days_remaining: (period_end - change_date).num_days().max(0) as u32,
            days_in_period: (period_end - period_start).num_days().max(0) as u32,
            pack_changes,
        }
    }

    /// `base` (the subscription now, or as it was at a past cycle) moved to the target version, in
    /// memory, for a preview to price. Each component keeps its row (id, effective dates, lineage)
    /// and takes the target fee of its product: the biller's history then never counts a row twice,
    /// and a component that started later is not backdated. Components the target drops are left
    /// out; a product the change does not know (only in `base`'s history) keeps its fee; the
    /// target's new components start on `added_from`. The minimum and the cadence are the target's.
    pub fn hypothetical_details(
        &self,
        target_version_id: PlanVersionId,
        base: &crate::domain::SubscriptionDetails,
        added_from: NaiveDate,
    ) -> crate::domain::SubscriptionDetails {
        let mut details = base.clone();
        let subscription_id = details.subscription.id;
        let current = &self.subscription_details.price_components;

        let mut by_component = HashMap::new();
        let mut by_product = HashMap::new();
        let mut removed_components = std::collections::HashSet::new();
        let mut removed_products = std::collections::HashSet::new();
        let mut added = Vec::new();
        for mapping in &self.component_mappings {
            match mapping {
                ComponentMapping::Matched {
                    current_component_id,
                    product_id,
                    ..
                } => {
                    by_component.insert(*current_component_id, mapping);
                    by_product.insert(*product_id, mapping);
                }
                ComponentMapping::Removed {
                    current_component_id,
                } => {
                    removed_components.insert(*current_component_id);
                    if let Some(product_id) = current
                        .iter()
                        .find(|c| c.id == *current_component_id)
                        .and_then(|c| c.product_id)
                    {
                        removed_products.insert(product_id);
                    }
                }
                ComponentMapping::Added {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                } => added.push(SubscriptionComponent {
                    id: common_domain::ids::SubscriptionPriceComponentId::new(),
                    price_component_id: Some(*target_component_id),
                    product_id: *product_id,
                    subscription_id,
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    price_id: *price_id,
                    effective_from: added_from,
                    effective_to: None,
                    lineage_id: None,
                    added_by_amendment: false,
                }),
            }
        }

        details.price_components = base
            .price_components
            .iter()
            .filter_map(|c| {
                let target = by_component
                    .get(&c.id)
                    .or_else(|| c.product_id.and_then(|p| by_product.get(&p)));
                match target {
                    Some(ComponentMapping::Matched {
                        target_component_id,
                        product_id,
                        price_id,
                        name,
                        fee,
                        period,
                        ..
                    }) => Some(SubscriptionComponent {
                        price_component_id: Some(*target_component_id),
                        product_id: Some(*product_id),
                        price_id: Some(*price_id),
                        name: name.clone(),
                        fee: fee.clone(),
                        period: *period,
                        ..c.clone()
                    }),
                    _ if removed_components.contains(&c.id)
                        || c.product_id.is_some_and(|p| removed_products.contains(&p)) =>
                    {
                        None
                    }
                    _ => Some(c.clone()),
                }
            })
            .chain(added)
            .collect();
        if !details.minimum.as_ref().is_some_and(|m| m.is_override) {
            details.minimum = self.target_minimum.as_ref().map(|m| {
                crate::domain::subscription_minimums::SubscriptionMinimum {
                    subscription_id,
                    amount_subunit: m.amount_subunit,
                    scope: m.scope.clone(),
                    is_override: false,
                }
            });
        }
        if let Some(period) = ComponentMapping::derive_billing_period(&self.component_mappings) {
            details.subscription.period = period;
        }
        details.subscription.plan_version_id = target_version_id;
        details
    }

    /// The target's new slot components: the change seeds their ledger with `initial_slots`.
    pub fn added_slot_units(&self) -> Vec<String> {
        self.component_mappings
            .iter()
            .filter_map(|m| match m {
                ComponentMapping::Added {
                    fee: SubscriptionFee::Slot { unit, .. },
                    ..
                } => Some(unit.clone()),
                _ => None,
            })
            .collect()
    }

    /// A minimum commitment, held or targeted, keeps the change to the period's end.
    pub(crate) fn ensure_immediate_allowed(&self) -> StoreResult<()> {
        if self.subscription_details.minimum.is_some() || self.target_minimum.is_some() {
            return Err(Report::new(StoreError::FailedPrecondition(
                MINIMUM_COMMITMENT_AT_PERIOD_END.to_owned(),
            )));
        }
        Ok(())
    }

    pub fn is_free_trial(&self) -> bool {
        self.free_trial
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
                    lineage_id: None,
                    added_by_amendment: false,
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
                    lineage_id: None,
                    added_by_amendment: false,
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

        let minimum = self
            .target_minimum
            .as_ref()
            .map(|pm| crate::domain::SubscriptionMinimum {
                subscription_id: virtual_sub.id,
                amount_subunit: pm.amount_subunit,
                scope: pm.scope.clone(),
                is_override: false,
            });

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
            usage_scopes: self.subscription_details.usage_scopes.clone(),
            checkout_url: None,
            pending_events: Vec::new(),
            entitlements: Vec::new(),
            minimum,
        })
    }
}

/// A plan change paid by a checkout still awaiting its payment applies when that payment settles:
/// another change meanwhile would be overtaken, or overtake it on stale prices. Refused: the
/// pending checkout is cancelled first (its payment, if it still settles, becomes credit).
pub(crate) async fn refuse_while_a_paid_change_is_pending(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    except: Option<common_domain::ids::CheckoutSessionId>,
) -> StoreResult<()> {
    if let Some(pending) =
        diesel_models::checkout_sessions::CheckoutSessionRow::plan_change_awaiting_payment(
            conn,
            tenant_id,
            subscription_id,
            except,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        return Err(Report::new(StoreError::FailedPrecondition(format!(
            "A plan change for this subscription is awaiting its payment (checkout session {pending}). Wait for the payment to complete or fail, or cancel that checkout session, before changing the subscription again."
        ))));
    }
    Ok(())
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

/// What a matched current component (same product) carries over to the target's price choice.
#[derive(Clone, Copy)]
struct Ported {
    period: SubscriptionFeeBillingPeriod,
    /// The committed capacity of a capacity fee.
    capacity: Option<u64>,
}

impl Ported {
    fn of(current: &SubscriptionComponent) -> Self {
        Ported {
            period: current.period,
            capacity: match current.fee {
                SubscriptionFee::Capacity { included, .. } => Some(included),
                _ => None,
            },
        }
    }

    /// For a component of a product the subscription doesn't have yet: the subscription's own
    /// cadence, its shortest recurring one.
    fn subscription_cadence(current: &[SubscriptionComponent]) -> Option<Self> {
        current
            .iter()
            .map(|component| component.period)
            .filter(|period| *period != SubscriptionFeeBillingPeriod::OneTime)
            .min_by_key(SubscriptionFeeBillingPeriod::as_months)
            .map(|period| Ported {
                period,
                capacity: None,
            })
    }
}

/// Resolve the fee and period for a target component.
///
/// Price selection priority:
/// 1. Explicit parametrization (from API caller) → use its billing_period / committed_capacity
/// 2. Ported cadence, then committed capacity, of the matched current component (same product_id);
///    for a new product, the subscription's cadence
/// 3. Single price → use it
/// 4. Multiple prices with no selection → error
fn resolve_target_fee(
    target: &TargetComponent,
    products: &HashMap<ProductId, Product>,
    units: &crate::domain::credit_units::CreditUnitsById,
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
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

            let price = select_target_price(prices, explicit_params, ported, &target.name)?;

            let fee = resolve_subscription_fee(
                fee_structure,
                &price.pricing,
                price.credit_unit.as_ref(),
                units,
                explicit_params,
            )?;
            let period = fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());

            Ok((period, fee, Some(price.id)))
        }
        TargetPricing::Legacy(legacy) => {
            let params = build_legacy_params(explicit_params, ported);
            let resolved = resolve_legacy_subscription_fee(legacy, params.as_ref())?;
            Ok((resolved.period, resolved.fee, None))
        }
    }
}

/// Select the right price from a V2 component's price list.
fn select_target_price<'a>(
    prices: &'a [Price],
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
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

    // 2. If still ambiguous, try the ported cadence, then the ported committed capacity, of the
    //    matched current component. Each narrows only when it matches something: no guess.
    if candidates.len() > 1
        && let Some(ported) = ported
    {
        let by_period: Vec<&Price> = candidates
            .iter()
            .filter(|p| p.cadence.as_subscription_billing_period() == ported.period)
            .copied()
            .collect();
        if !by_period.is_empty() {
            candidates = by_period;
        }
        if candidates.len() > 1
            && let Some(cap) = ported.capacity
        {
            let by_capacity: Vec<&Price> = candidates
                .iter()
                .filter(|p| matches!(&p.pricing, Pricing::Capacity { included, .. } if *included == cap))
                .copied()
                .collect();
            if !by_capacity.is_empty() {
                candidates = by_capacity;
            }
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

/// Build ComponentParameters for legacy pricing resolution from explicit params or what the
/// matched current component carries over.
fn build_legacy_params(
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
) -> Option<ComponentParameters> {
    if explicit_params.is_some() {
        return explicit_params.cloned();
    }
    let ported = ported?;
    let billing_period = ported.period.as_billing_period_opt();
    (billing_period.is_some() || ported.capacity.is_some()).then_some(ComponentParameters {
        initial_slot_count: None,
        billing_period,
        committed_capacity: ported.capacity,
    })
}

fn build_component_mappings(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    products: &HashMap<ProductId, Product>,
    units: &crate::domain::credit_units::CreditUnitsById,
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

            let (period, fee, price_id) = resolve_target_fee(
                target,
                products,
                units,
                explicit_params,
                Some(Ported::of(current)),
            )?;

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
        let (period, fee, price_id) = resolve_target_fee(
            target,
            products,
            units,
            explicit_params,
            Ported::subscription_cadence(current_components),
        )?;

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

fn same_fee(current: &SubscriptionFee, new: &SubscriptionFee) -> bool {
    matches!(
        (serde_json::to_value(current), serde_json::to_value(new)),
        (Ok(current), Ok(new)) if current == new
    )
}

/// The adjustment-invoice lines of the target's packs. A `once` pack bills only if no invoice of
/// the subscription billed its product's one-time credit yet.
#[allow(clippy::too_many_arguments)]
async fn new_pack_lines(
    conn: &mut PgConn,
    subscription: &crate::domain::Subscription,
    mappings: &[ComponentMapping],
    component_ids: &HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    carried: &HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    (period_start, period_end): (NaiveDate, NaiveDate),
    change_date: NaiveDate,
    precision: u8,
) -> StoreResult<Vec<crate::domain::subscription_changes::ProrationLineItem>> {
    use diesel_models::invoices::InvoiceRow;
    let days_in_period = (period_end - period_start).num_days();
    let days_remaining = (period_end - change_date).num_days();
    let base = DayFraction::remaining(period_start, period_end, change_date);
    let mut lines = Vec::new();
    for mapping in mappings {
        let (target_component_id, product_id, name, fee, period) = match mapping {
            ComponentMapping::Matched {
                target_component_id,
                product_id,
                name,
                fee,
                period,
                ..
            } => (target_component_id, Some(*product_id), name, fee, period),
            ComponentMapping::Added {
                target_component_id,
                product_id,
                name,
                fee,
                period,
                ..
            } => (target_component_id, *product_id, name, fee, period),
            ComponentMapping::Removed { .. } => continue,
        };
        let SubscriptionFee::Credit { once, .. } = fee else {
            continue;
        };
        if carried.contains_key(target_component_id) {
            continue;
        }
        if *once {
            let billed = match product_id {
                Some(product_id) => InvoiceRow::subscription_billed_once_credit(
                    conn,
                    subscription.tenant_id,
                    subscription.id,
                    product_id,
                    None,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?,
                None => true,
            };
            if billed {
                continue;
            }
        }
        let factor = component_proration_factor(period, days_remaining, days_in_period, base);
        if let Some(line) = pack_charge_line(
            name,
            fee,
            factor,
            precision,
            product_id,
            Some(*target_component_id),
            component_ids.get(target_component_id).copied(),
        )? {
            lines.push(line);
        }
    }
    Ok(lines)
}

fn build_plan_change_preview(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    products: &HashMap<ProductId, Product>,
    units: &crate::domain::credit_units::CreditUnitsById,
    component_params: &[ComponentParameterization],
    effective_date: chrono::NaiveDate,
    component_ids: &HashMap<PriceComponentId, SubscriptionPriceComponentId>,
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

            let (new_period, new_fee, _) = resolve_target_fee(
                target,
                products,
                units,
                explicit_params,
                Some(Ported::of(current)),
            )?;

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
                current_net_key: Some(current.lineage().to_string()),
                billed_component_id: component_ids.get(&target.id).copied(),
                price_component_id: Some(target.id),
            });
            continue;
        }

        let (period, fee, _) = resolve_target_fee(
            target,
            products,
            units,
            explicit_params,
            Ported::subscription_cadence(current_components),
        )?;

        let instance_quantity = crate::services::subscriptions::proration::fee_instance_count(&fee);
        added.push(AddedComponent {
            name: target.name.clone(),
            fee,
            period,
            net_key: None,
            billed_component_id: component_ids.get(&target.id).copied(),
            billed_add_on_id: None,
            instance_quantity: Some(instance_quantity),
            price_component_id: Some(target.id),
        });
    }

    for current in current_components {
        if !matched_current_ids.contains(&current.id) {
            removed.push(RemovedComponent {
                name: current.name.clone(),
                current_fee: current.fee.clone(),
                current_period: current.period,
                net_key: Some(current.lineage().to_string()),
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
        let actual = SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            unit.clone(),
            common_domain::clock::now().naive_utc(),
        )
        .await
        .map(|slots| slots.unwrap_or(0) as u32)
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
) -> StoreResult<Vec<crate::services::subscriptions::utils::MrrLine>> {
    use crate::services::subscriptions::utils::{
        MrrLine, calculate_mrr, fee_period_total_cents, monthly_share,
    };

    // Collect unique slot units and query actual counts
    let mut slot_counts: HashMap<String, i64> = HashMap::new();
    for c in components {
        if let SubscriptionFee::Slot { unit, .. } = &c.fee
            && !slot_counts.contains_key(unit)
        {
            let count = SlotTransactionRow::fetch_active_slots(
                conn,
                tenant_id,
                subscription_id,
                unit.clone(),
                common_domain::clock::now().naive_utc(),
            )
            .await
            .map(|slots| i64::from(slots.unwrap_or(0)))
            .map_err(Into::<Report<StoreError>>::into)?;
            slot_counts.insert(unit.clone(), count);
        }
    }

    let mut lines = Vec::with_capacity(components.len());
    for c in components {
        let (monthly, period_total) = match &c.fee {
            SubscriptionFee::Slot {
                unit, unit_rate, ..
            } => {
                let count = slot_counts.get(unit).copied().unwrap_or(0);
                let rate_cents = unit_rate.to_subunit_opt(precision).unwrap_or(0);
                // Rounded through the same `monthly_share` `calculate_mrr` uses: this
                // branch recomputes what creation computed, and a figure truncated here
                // against a half-up one there books a phantom one-cent contraction on the
                // first amendment.
                (
                    monthly_share(count * rate_cents, i64::from(c.period.as_months())),
                    count * rate_cents,
                )
            }
            _ => (
                calculate_mrr(&c.fee, &c.period, precision),
                fee_period_total_cents(&c.fee, precision),
            ),
        };
        lines.push(MrrLine::new(&c.fee, monthly, c.product_id).with_period_total(period_total));
    }

    Ok(lines)
}

/// On plan change, refresh the `subscription_minimum` snapshot from the new plan version's
/// `plan_version_minimum`. **Preserves operator/quote overrides** (`is_override = true`):
/// those represent negotiated contracts and survive plan changes.
///
/// Behaviour matrix:
///
/// | existing snapshot       | new plan has min | result                                       |
/// |-------------------------|------------------|----------------------------------------------|
/// | none                    | none             | nothing                                      |
/// | none                    | yes              | new snapshot, is_override = false            |
/// | plan-snapshot           | none             | snapshot deleted (no floor on new plan)      |
/// | plan-snapshot           | yes              | snapshot replaced from new plan              |
/// | override                | -                | left untouched                               |
pub(crate) async fn refresh_subscription_minimum_on_plan_change(
    conn: &mut crate::store::PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
    new_plan_version_id: common_domain::ids::PlanVersionId,
) -> StoreResult<()> {
    use diesel_models::plan_version_adjustments::{
        PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentRow,
    };
    use diesel_models::subscription_adjustments::{
        SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentComponentRowNew,
        SubscriptionAdjustmentRow, SubscriptionAdjustmentRowNew,
    };

    let existing = SubscriptionAdjustmentRow::find_by_subscription_id(conn, &subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if matches!(&existing, Some(e) if e.is_override) {
        return Ok(());
    }

    // Drop any existing plan-snapshot — it might be stale or the new plan might not have one.
    if existing.is_some() {
        SubscriptionAdjustmentRow::delete_by_subscription_id(conn, &subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    // Snapshot from the new plan version, if any.
    let Some(plan_min_row) =
        PlanVersionAdjustmentRow::find_by_plan_version_id(conn, &new_plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    else {
        return Ok(());
    };

    SubscriptionAdjustmentRowNew {
        subscription_id,
        kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
        amount_subunit: plan_min_row.amount_subunit,
        includes_all_components: plan_min_row.includes_all_components,
        is_override: false,
    }
    .insert(conn)
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    if !plan_min_row.includes_all_components {
        let plan_components =
            PlanVersionAdjustmentComponentRow::list_by_plan_version_id(conn, &new_plan_version_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let sub_components: Vec<SubscriptionAdjustmentComponentRowNew> = plan_components
            .into_iter()
            .map(|c| SubscriptionAdjustmentComponentRowNew {
                subscription_id,
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                product_id: c.product_id,
            })
            .collect();
        SubscriptionAdjustmentComponentRow::insert_batch(conn, &sub_components)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enums::BillingPeriodEnum;
    use common_domain::ids::{BaseId, PriceId, TenantId};

    fn rate_price(cadence: BillingPeriodEnum) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::new(),
            cadence,
            currency: "USD".to_owned(),
            pricing: Pricing::Rate {
                rate: rust_decimal::Decimal::TEN,
            },
            tenant_id: TenantId::new(),
            created_at: common_domain::clock::now().naive_utc(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    #[test]
    fn a_new_product_with_several_cadences_takes_the_subscription_one() {
        let prices = [
            rate_price(BillingPeriodEnum::Monthly),
            rate_price(BillingPeriodEnum::Annual),
        ];
        assert!(select_target_price(&prices, None, None, "Support").is_err());

        let monthly = Ported {
            period: SubscriptionFeeBillingPeriod::Monthly,
            capacity: None,
        };
        let price = select_target_price(&prices, None, Some(monthly), "Support").unwrap();
        assert_eq!(price.cadence, BillingPeriodEnum::Monthly);
    }
}

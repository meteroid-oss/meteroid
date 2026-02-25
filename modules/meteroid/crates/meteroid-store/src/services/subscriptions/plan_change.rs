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
    AddedComponent, MatchedComponent, PlanChangePreview, RemovedComponent,
};
use crate::domain::subscription_components::{
    ComponentParameterization, ComponentParameters, SubscriptionComponent,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::{PlanVersionId, PriceComponentId, ProductId, SubscriptionId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::scheduled_events::ScheduledEventRow;
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

                    let subscription = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    validate_subscription_for_plan_change(&subscription.subscription.status)?;

                    // Cancel all pending lifecycle events (plan change, cancellation, pause, etc.)
                    ScheduledEventRow::cancel_pending_lifecycle_events(
                        conn,
                        subscription_id,
                        &tenant_id,
                        "Replaced by new plan change",
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // Validate target plan version
                    let target_plan =
                        PlanRow::get_with_version(conn, new_plan_version_id, tenant_id)
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

                    if target_version.currency != subscription.subscription.currency {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Currency mismatch: subscription uses {} but target plan uses {}",
                            subscription.subscription.currency, target_version.currency
                        ))));
                    }

                    // Load target components with prices and legacy data
                    let target_components = load_target_components_with_prices(
                        conn,
                        tenant_id,
                        new_plan_version_id,
                        target_version.currency.clone(),
                    )
                    .await?;

                    // Load products for fee_structure resolution (v2 only)
                    let products = load_products_for_components(
                        conn,
                        tenant_id,
                        &subscription.price_components,
                        &target_components,
                    )
                    .await?;

                    // Build component mappings
                    let component_mappings = build_component_mappings(
                        &subscription.price_components,
                        &target_components,
                        &products,
                        &component_params,
                    )?;

                    // Schedule at current_period_end
                    let effective_date =
                        subscription
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
    ) -> StoreResult<PlanChangePreview> {
        let mut conn = self.store.get_conn().await?;

        let subscription = self
            .store
            .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
            .await?;

        validate_subscription_for_plan_change(&subscription.subscription.status)?;

        // Validate target plan version
        let target_plan = PlanRow::get_with_version(&mut conn, new_plan_version_id, tenant_id)
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

        if target_version.currency != subscription.subscription.currency {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Currency mismatch: subscription uses {} but target plan uses {}",
                subscription.subscription.currency, target_version.currency
            ))));
        }

        // Load target components with prices and legacy data
        let target_components = load_target_components_with_prices(
            &mut conn,
            tenant_id,
            new_plan_version_id,
            target_version.currency.clone(),
        )
        .await?;

        // Load products for fee_structure resolution (v2 only)
        let products = load_products_for_components(
            &mut conn,
            tenant_id,
            &subscription.price_components,
            &target_components,
        )
        .await?;

        let effective_date = subscription
            .subscription
            .current_period_end
            .unwrap_or(subscription.subscription.current_period_start);

        build_plan_change_preview(
            &subscription.price_components,
            &target_components,
            &products,
            &component_params,
            effective_date,
        )
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
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;

        let event = ScheduledEventRow::get_by_id(&mut conn, event_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

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

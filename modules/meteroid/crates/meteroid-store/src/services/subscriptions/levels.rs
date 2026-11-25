//! A usage-levels charge's level (R-135): one of the capacity prices of its plan component's
//! cell. A higher level applies at once, prorated as an immediate amendment bills it; a lower one
//! is scheduled for the period's end. Both are amendments overriding the component with the
//! level's price, which `resolve_amendment` checks is a level of its cell.

use crate::StoreResult;
use crate::domain::Price;
use crate::domain::entity_activity::Actor;
use crate::domain::price_components::PriceEntry;
use crate::domain::prices::Pricing;
use crate::domain::subscription_amendment::{
    AddOnChanges, CapacityLevel, ComponentChanges, ComponentLevels, EditComponent,
    LevelChangeOutcome, LevelChangePreview, SubscriptionAmendment,
};
use crate::domain::subscription_changes::{ChangeDirection, PlanChangeMode};
use crate::domain::subscription_components::{SubscriptionComponent, SubscriptionFee};
use crate::domain::{Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{BaseId, PriceId, SubscriptionId, SubscriptionPriceComponentId, TenantId};
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::prices::PriceRow;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use error_stack::Report;

pub(crate) const LOWER_LEVEL_AT_PERIOD_END: &str =
    "A lower level takes effect at the end of the billing period.";

/// The levels of `component`: the capacity prices of its plan component's cell in the
/// subscription's option, currency and cadence, fewest included units first. Empty for a component
/// not from a plan, or not a capacity.
pub(crate) async fn levels_of(
    conn: &mut PgConn,
    subscription: &Subscription,
    component: &SubscriptionComponent,
) -> StoreResult<Vec<Price>> {
    let (Some(price_component_id), SubscriptionFee::Capacity { .. }) =
        (component.price_component_id, &component.fee)
    else {
        return Ok(Vec::new());
    };
    let cells = PlanComponentPriceRow::list_by_component_ids(conn, &[price_component_id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    // Without an option, the subscription predates options: every cell of the charge.
    let mut ids: Vec<PriceId> = cells
        .iter()
        .filter(|cell| {
            subscription
                .plan_option_id
                .is_none_or(|option| option == cell.option_id)
        })
        .map(|cell| cell.price_id)
        .collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut levels: Vec<Price> = PriceRow::list_by_ids(conn, &ids, subscription.tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(Price::try_from)
        .collect::<StoreResult<Vec<_>>>()?
        .into_iter()
        .filter(|price| {
            price.currency.eq_ignore_ascii_case(&subscription.currency)
                && price.cadence.as_subscription_billing_period() == component.period
                && matches!(price.pricing, Pricing::Capacity { .. })
        })
        .collect();
    levels.sort_by_key(included_of);
    Ok(levels)
}

pub(crate) fn included_of(price: &Price) -> u64 {
    match &price.pricing {
        Pricing::Capacity { included, .. } => *included,
        _ => 0,
    }
}

/// The units `fee` includes now; none for a fee that is no capacity.
pub(crate) fn fee_included(fee: &SubscriptionFee) -> Option<u64> {
    match fee {
        SubscriptionFee::Capacity { included, .. } => Some(*included),
        _ => None,
    }
}

/// A move of `component` to `price`: the level, of its cell, it moves to, and whether it is a
/// move up (applied at once) or down (at the period's end).
pub(crate) fn level_direction(
    component: &SubscriptionComponent,
    levels: &[Price],
    price_id: PriceId,
) -> StoreResult<(u64, u64, ChangeDirection)> {
    let level = levels.iter().find(|p| p.id == price_id).ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(format!(
            "Price {} is not a level of {} in this subscription's option and currency",
            price_id, component.name
        )))
    })?;
    let from = fee_included(&component.fee).unwrap_or(0);
    let to = included_of(level);
    let direction = match to.cmp(&from) {
        std::cmp::Ordering::Greater => ChangeDirection::Upgrade,
        std::cmp::Ordering::Less => ChangeDirection::Downgrade,
        std::cmp::Ordering::Equal => ChangeDirection::Lateral,
    };
    Ok((from, to, direction))
}

fn component_of(
    details: &SubscriptionDetails,
    subscription_component_id: SubscriptionPriceComponentId,
) -> StoreResult<&SubscriptionComponent> {
    details
        .price_components
        .iter()
        .find(|c| c.id == subscription_component_id)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Component {} not found on subscription",
                subscription_component_id
            )))
        })
}

/// R-106: a change still waiting would land after a level change and undo it. Read before the
/// move is built, and again under the subscription's lock as it is written.
pub(in crate::services) async fn refuse_while_a_change_is_pending(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    let pending = SubscriptionChangeRow::list_pending(
        conn,
        &tenant_id,
        subscription_id,
        &[
            SubscriptionChangeKindEnum::Amendment,
            SubscriptionChangeKindEnum::PlanChange,
        ],
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    if !pending.is_empty() {
        return Err(Report::new(StoreError::FailedPrecondition(
            "A change is already scheduled for this subscription: cancel it to change level"
                .to_string(),
        )));
    }
    Ok(())
}

/// The amendment a move to `price_id` is, and how it applies.
struct LevelMove {
    amendment: SubscriptionAmendment,
    direction: ChangeDirection,
    from: u64,
    to: u64,
}

async fn level_move(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    subscription_component_id: SubscriptionPriceComponentId,
    price_id: PriceId,
) -> StoreResult<LevelMove> {
    let component = component_of(details, subscription_component_id)?;
    let levels = levels_of(conn, &details.subscription, component).await?;
    if levels.len() < 2 {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "{} has no other level to move to",
            component.name
        ))));
    }
    let (from, to, direction) = level_direction(component, &levels, price_id)?;
    if direction == ChangeDirection::Lateral {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "{} already includes {} units",
            component.name, to
        ))));
    }
    refuse_while_a_change_is_pending(
        conn,
        details.subscription.tenant_id,
        details.subscription.id,
    )
    .await?;
    Ok(LevelMove {
        amendment: SubscriptionAmendment {
            apply_mode: if direction == ChangeDirection::Upgrade {
                PlanChangeMode::Immediate
            } else {
                PlanChangeMode::EndOfPeriod
            },
            component_changes: ComponentChanges {
                edited: vec![EditComponent {
                    subscription_component_id,
                    name: None,
                    price_entry: PriceEntry::Existing(price_id),
                }],
                added: Vec::new(),
                removed: Vec::new(),
            },
            add_on_changes: AddOnChanges::default(),
        },
        direction,
        from,
        to,
    })
}

impl Services {
    /// The subscription's usage-levels charges, each with the levels of its cell.
    pub(in crate::services) async fn list_capacity_levels(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<ComponentLevels>> {
        let mut conn = self.store.get_conn().await?;
        let details = self
            .store
            .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
            .await?;
        let mut out = Vec::new();
        for component in &details.price_components {
            let (Some(price_component_id), Some(included)) =
                (component.price_component_id, fee_included(&component.fee))
            else {
                continue;
            };
            let levels = levels_of(&mut conn, &details.subscription, component).await?;
            if levels.is_empty() {
                continue;
            }
            out.push(ComponentLevels {
                subscription_component_id: component.id,
                price_component_id,
                name: component.name.clone(),
                period: component.period,
                included,
                levels: levels
                    .iter()
                    .filter_map(|price| match &price.pricing {
                        Pricing::Capacity {
                            rate,
                            included,
                            overage_rate,
                        } => Some(CapacityLevel {
                            price_id: price.id,
                            included: *included,
                            rate: *rate,
                            overage_rate: *overage_rate,
                            current: component.price_id == Some(price.id),
                        }),
                        _ => None,
                    })
                    .collect(),
            });
        }
        Ok(out)
    }

    /// What moving `subscription_component_id` to the level `price_id` bills, and when.
    pub(in crate::services) async fn preview_capacity_level_change(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        subscription_component_id: SubscriptionPriceComponentId,
        price_id: PriceId,
    ) -> StoreResult<LevelChangePreview> {
        let level = {
            let mut conn = self.store.get_conn().await?;
            let details = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                .await?;
            level_move(&mut conn, &details, subscription_component_id, price_id).await?
        };
        let apply_mode = level.amendment.apply_mode;
        let preview = self
            .preview_amendment(subscription_id, tenant_id, level.amendment)
            .await?;
        Ok(LevelChangePreview {
            direction: level.direction,
            apply_mode,
            effective_date: preview.preview.effective_date,
            from_included: level.from,
            to_included: level.to,
            preview,
        })
    }

    /// Moves `subscription_component_id` to the level `price_id`: a higher level at once, a lower
    /// one at the period's end (R-135).
    pub(in crate::services) async fn change_capacity_level(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        subscription_component_id: SubscriptionPriceComponentId,
        price_id: PriceId,
    ) -> StoreResult<LevelChangeOutcome> {
        let level = {
            let mut conn = self.store.get_conn().await?;
            let details = self
                .store
                .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                .await?;
            level_move(&mut conn, &details, subscription_component_id, price_id).await?
        };
        match level.amendment.apply_mode {
            PlanChangeMode::Immediate => self
                .apply_level_change_immediate(actor, subscription_id, tenant_id, level.amendment)
                .await
                .map(LevelChangeOutcome::Applied),
            PlanChangeMode::EndOfPeriod => self
                .schedule_level_change(actor, subscription_id, tenant_id, level.amendment)
                .await
                .map(|event| LevelChangeOutcome::Scheduled(Box::new(event))),
        }
    }
}

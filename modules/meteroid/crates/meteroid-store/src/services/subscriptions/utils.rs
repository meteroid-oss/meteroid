use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::price_components::{PriceEntry, ProductRef};
use crate::domain::subscriptions::PaymentMethodsConfig;
use crate::domain::{
    CreateSubscriptionAddOns, CreateSubscriptionComponents, CreatedSubscription, PriceComponent,
    Subscription, SubscriptionAddOnNewInternal, SubscriptionComponentNewInternal,
    SubscriptionDetails, SubscriptionFee, SubscriptionNew,
};
use crate::errors::StoreError;
use crate::services::subscriptions::insert::context::ResolvedCustomComponents;
use crate::store::{PgConn, Store};
use chrono::NaiveDate;
use common_domain::ids::{ConnectorId, ProductFamilyId};
use common_utils::decimals::ToSubunit;
use diesel_models::errors::DatabaseError;
use error_stack::Report;
use itertools::Itertools;
use std::collections::HashMap;

use crate::domain::add_ons::AddOn;
use crate::domain::coupons::{Coupon, CouponDiscount};
use crate::domain::outbox_event::OutboxEvent;
use crate::repositories::historical_rates::HistoricalRatesInterface;
use diesel_models::DbResult;
use diesel_models::applied_coupons::{AppliedCouponRow, AppliedCouponRowNew};
use diesel_models::coupon_plans::CouponPlanRow;
use diesel_models::coupons::CouponRow;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use rust_decimal::prelude::*;

use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use common_domain::ids::{AppliedCouponId, BaseId, CouponId, PlanVersionId, TenantId};
use diesel_models::plans::PlanRow;

/// A product/price that needs to be created inside the persist transaction.
#[derive(Debug, Clone)]
pub struct PendingMaterialization {
    /// Index into the components vector this materialization belongs to.
    pub component_index: usize,
    pub name: String,
    pub product_ref: ProductRef,
    pub price_entry: PriceEntry,
    pub product_family_id: ProductFamilyId,
    pub currency: String,
}

/// Validates charge_automatically: requires Online config + at least one online provider.
pub fn validate_charge_automatically_with_provider_ids(
    charge_automatically: bool,
    payment_methods_config: Option<&PaymentMethodsConfig>,
    card_provider_id: Option<ConnectorId>,
    direct_debit_provider_id: Option<ConnectorId>,
) -> StoreResult<()> {
    if !charge_automatically {
        return Ok(());
    }

    let is_online = match payment_methods_config {
        None | Some(PaymentMethodsConfig::Online { .. }) => true,
        Some(PaymentMethodsConfig::BankTransfer { .. } | PaymentMethodsConfig::External) => false,
    };

    if !is_online {
        return Err(Report::new(StoreError::InvalidArgument(
            "charge_automatically requires payment_methods_config to be Online".to_string(),
        )));
    }

    if card_provider_id.is_none() && direct_debit_provider_id.is_none() {
        return Err(Report::new(StoreError::InvalidArgument(
            "charge_automatically requires the invoicing entity to have a payment provider configured (card or direct debit)".to_string(),
        )));
    }

    Ok(())
}

pub fn calculate_mrr(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    let mut total_cents = 0;

    let period_as_months = i64::from(period.as_months());

    match fee {
        SubscriptionFee::Rate { rate } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Recurring { quantity, rate, .. } => {
            let total = rate * Decimal::from(*quantity);
            total_cents = total.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Capacity { rate, .. } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => {
            total_cents =
                i64::from(*initial_slots) * unit_rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::OneTime { .. } | SubscriptionFee::Usage { .. } => {
            // doesn't count as mrr
        }
    }

    let _mrr = total_cents / period_as_months;

    let mrr_monthly = Decimal::from(total_cents) / Decimal::from(period_as_months);

    mrr_monthly.to_i64().unwrap_or(0)
}

pub fn process_create_subscription_add_ons(
    create: &Option<CreateSubscriptionAddOns>,
    add_ons: &[AddOn],
    products: &HashMap<common_domain::ids::ProductId, crate::domain::Product>,
    prices: &HashMap<common_domain::ids::PriceId, crate::domain::prices::Price>,
    product_family_id: ProductFamilyId,
    currency: &str,
) -> Result<
    (
        Vec<SubscriptionAddOnNewInternal>,
        Vec<PendingMaterialization>,
    ),
    Report<StoreError>,
> {
    let mut processed_add_ons = Vec::new();
    let mut pending_materializations = Vec::new();

    if let Some(create) = create {
        for cs_ao in &create.add_ons {
            let add_on = add_ons.iter().find(|x| x.id == cs_ao.add_on_id).ok_or(
                StoreError::ValueNotFound(format!("add-on {} not found", cs_ao.add_on_id)),
            )?;

            if cs_ao.quantity < 1 {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "add-on {} quantity must be >= 1",
                    cs_ao.add_on_id
                ))));
            }

            if let Some(max) = add_on.max_instances_per_subscription
                && cs_ao.quantity > max
            {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "add-on {} quantity {} exceeds max_instances_per_subscription {}",
                    cs_ao.add_on_id, cs_ao.quantity, max
                ))));
            }

            let resolved = add_on
                .resolve_customized(products, prices, &cs_ao.customization)
                .map_err(Report::new)?;

            let idx = processed_add_ons.len();

            // If price_id is None and the override uses PriceEntry::New, we need materialization
            if resolved.price_id.is_none()
                && let Some(PriceEntry::New(_)) = &resolved.price_entry
            {
                pending_materializations.push(PendingMaterialization {
                    component_index: idx,
                    name: resolved.name.clone(),
                    product_ref: ProductRef::Existing(add_on.product_id),
                    price_entry: resolved.price_entry.clone().unwrap(),
                    product_family_id,
                    currency: currency.to_string(),
                });
            }

            processed_add_ons.push(SubscriptionAddOnNewInternal {
                add_on_id: add_on.id,
                name: resolved.name,
                period: resolved.period,
                fee: resolved.fee,
                product_id: resolved.product_id,
                price_id: resolved.price_id,
                quantity: cs_ao.quantity,
            });
        }
    }

    Ok((processed_add_ons, pending_materializations))
}

pub fn process_create_subscription_coupons(
    subscription: &SubscriptionRowNew,
    coupons: &[Coupon],
) -> Result<Vec<AppliedCouponRowNew>, Report<StoreError>> {
    let processed_coupons = coupons
        .iter()
        .unique_by(|x| x.id)
        .map(|x| AppliedCouponRowNew {
            id: AppliedCouponId::new(),
            subscription_id: subscription.id,
            coupon_id: x.id,
            customer_id: subscription.customer_id,
            is_active: true,
            applied_amount: None,
            applied_count: None,
            last_applied_at: None,
        })
        .collect();

    Ok(processed_coupons)
}

pub async fn apply_coupons(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
    subscriptions: &[CreatedSubscription],
    tenant_id: TenantId,
) -> DbResult<()> {
    validate_coupons(tx_conn, subscription_coupons, subscriptions, tenant_id).await?;
    apply_coupons_internal(tx_conn, subscription_coupons).await
}

/// Applies coupons without validation.
///
/// Use this for async payment flows where coupons were already validated before charging.
/// The customer already paid the discounted price, so we honor it even if the coupon
/// became invalid (e.g., hit redemption limit) during the async payment window.
/// TODO use a coupon_reservation table with cleanup linked checkouts to avoid any over-redemption due to async flows
pub async fn apply_coupons_without_validation(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
) -> DbResult<()> {
    apply_coupons_internal(tx_conn, subscription_coupons).await
}

async fn apply_coupons_internal(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
) -> DbResult<()> {
    if subscription_coupons.is_empty() {
        return Ok(());
    }

    AppliedCouponRow::insert_batch(tx_conn, subscription_coupons.to_vec()).await?;

    CouponRow::update_last_redemption_at(
        tx_conn,
        &subscription_coupons
            .iter()
            .map(|c| c.coupon_id)
            .unique()
            .collect::<Vec<_>>(),
        chrono::Utc::now().naive_utc(),
    )
    .await?;

    let subscriptions_by_coupon: HashMap<CouponId, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);

    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        CouponRow::inc_redemption_count(tx_conn, coupon_id, subscriptions_count as i32).await?;
    }

    Ok(())
}

/// validate coupons can be applied to subscriptions
/// must be inside tx to handle concurrent inserts
pub async fn validate_coupons(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&AppliedCouponRowNew],
    subscriptions: &[CreatedSubscription],
    tenant_id: TenantId,
) -> DbResult<()> {
    if subscription_coupons.is_empty() {
        return Ok(());
    }

    let coupons_ids = subscription_coupons
        .iter()
        .map(|x| x.coupon_id)
        .unique()
        .collect::<Vec<_>>();

    let coupons = CouponRow::list_by_ids_for_update(tx_conn, &coupons_ids, &tenant_id).await?;

    // Fetch plan restrictions for coupons
    let coupon_plan_restrictions = CouponPlanRow::list_by_coupon_ids(tx_conn, &coupons_ids).await?;

    // Fetch plan_ids for all plan_version_ids in subscriptions
    let plan_version_ids: Vec<PlanVersionId> =
        subscriptions.iter().map(|s| s.plan_version_id).collect();
    let plan_version_to_plan =
        PlanVersionRow::get_plan_ids_by_version_ids(tx_conn, &plan_version_ids).await?;

    let now = chrono::Utc::now().naive_utc();

    for coupon in &coupons {
        let applied_coupon = subscription_coupons
            .iter()
            .find(|x| x.coupon_id == coupon.id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "Applied coupon {} not found",
                coupon.code
            ))))?;

        let subscription = subscriptions
            .iter()
            .find(|x| x.id == applied_coupon.subscription_id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "Subscription {} not found",
                applied_coupon.subscription_id
            ))))?;

        // check expired coupons
        if coupon.expires_at.is_some_and(|x| x <= now) {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is expired",
                coupon.code
            )))
            .into());
        }
        // check archived coupons
        if coupon.archived_at.is_some() {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is archived",
                coupon.code
            )))
            .into());
        }

        // check disabled coupons
        if coupon.disabled {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} is disabled",
                coupon.code
            )))
            .into());
        }

        // TEMPORARY CHECK: currency is the same as in subscription
        let discount: CouponDiscount =
            serde_json::from_value(coupon.discount.clone()).map_err(|_| {
                Report::new(DatabaseError::ValidationError(format!(
                    "Discount serde error for coupon {}",
                    &coupon.code
                )))
            })?;

        if discount
            .currency()
            .is_some_and(|x| x != subscription.currency)
        {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} currency does not match subscription currency",
                coupon.code
            )))
            .into());
        }

        // check plan restrictions
        if let Some(allowed_plans) = coupon_plan_restrictions.get(&coupon.id)
            && !allowed_plans.is_empty()
        {
            let plan_id = plan_version_to_plan
                .get(&subscription.plan_version_id)
                .ok_or(Report::new(DatabaseError::ValidationError(format!(
                    "Plan not found for subscription {}",
                    subscription.id
                ))))?;

            if !allowed_plans.contains(plan_id) {
                return Err(Report::new(DatabaseError::ValidationError(format!(
                    "coupon {} cannot be applied to this plan",
                    coupon.code
                )))
                .into());
            }
        }
    }

    // check if the coupon has reached its redemption limit
    let subscriptions_by_coupon: HashMap<CouponId, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);
    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        let coupon = coupons
            .iter()
            .find(|x| x.id == coupon_id)
            .ok_or(Report::new(DatabaseError::ValidationError(format!(
                "coupon {coupon_id} not found"
            ))))?;

        if let Some(redemption_limit) = coupon.redemption_limit
            && redemption_limit < subscriptions_count as i32 + coupon.redemption_count
        {
            return Err(Report::new(DatabaseError::ValidationError(format!(
                "coupon {} has reached its maximum redemptions",
                coupon.code
            )))
            .into());
        }
    }

    // check non-reusable coupons - these cannot be reused by the same customer
    let non_reusable_coupons = coupons.iter().filter(|x| !x.reusable).collect::<Vec<_>>();
    if !non_reusable_coupons.is_empty() {
        // Build a list of (coupon_id, customer_id) pairs for non-reusable coupons
        let customer_coupon_pairs: Vec<(CouponId, common_domain::ids::CustomerId)> =
            subscription_coupons
                .iter()
                .filter_map(|applied| {
                    let coupon = non_reusable_coupons
                        .iter()
                        .find(|c| c.id == applied.coupon_id)?;
                    Some((coupon.id, applied.customer_id))
                })
                .collect();

        // Check if any of these pairs already exist in the database
        let existing_pairs =
            AppliedCouponRow::find_existing_customer_coupon_pairs(tx_conn, &customer_coupon_pairs)
                .await?;

        // If any customer is trying to reuse a non-reusable coupon, reject it
        for coupon in non_reusable_coupons {
            let reused_by_customer = subscription_coupons.iter().find(|applied| {
                applied.coupon_id == coupon.id
                    && existing_pairs.contains(&(coupon.id, applied.customer_id))
            });

            if let Some(applied) = reused_by_customer {
                return Err(Report::new(DatabaseError::ValidationError(format!(
                    "coupon {} is not reusable and has already been used by customer {}",
                    coupon.code, applied.customer_id
                )))
                .into());
            }
        }
    }

    Ok(())
}

// TODO check with the other calculate_coupons_discount
#[allow(dead_code)]
pub async fn calculate_coupons_discount(
    store: &Store,
    coupons: &[Coupon],
    subscription_currency: &String,
    amount: Decimal,
) -> StoreResult<Decimal> {
    if (amount == Decimal::ZERO) || coupons.is_empty() {
        return Ok(amount);
    }

    let mut total = amount;

    for coupon in coupons {
        if !coupon.is_infinite() {
            continue;
        }

        match &coupon.discount {
            CouponDiscount::Percentage(percentage) => {
                total = total * percentage / Decimal::new(100, 0);
            }
            CouponDiscount::Fixed {
                currency,
                amount: fixed_amount,
            } => {
                let discount_amount = if currency == subscription_currency {
                    *fixed_amount
                } else {
                    let rate = store
                        .get_historical_rate(
                            currency,
                            subscription_currency,
                            chrono::Utc::now().date_naive(),
                        )
                        .await?
                        .ok_or(StoreError::ValueNotFound(format!(
                            "historical rate from {currency} to {subscription_currency}"
                        )))?
                        .rate;

                    fixed_amount * Decimal::from_f32(rate).unwrap_or(Decimal::ZERO)
                };

                total = (total - discount_amount).max(Decimal::ZERO);
            }
        }
    }

    Ok(total)
}

pub fn extract_billing_period(
    components: &[SubscriptionComponentNewInternal],
    add_ons: &[SubscriptionAddOnNewInternal],
) -> BillingPeriodEnum {
    components
        .iter()
        .map(|x| &x.period)
        .chain(add_ons.iter().map(|x| &x.period))
        .filter_map(SubscriptionFeeBillingPeriod::as_billing_period_opt)
        .min()
        .unwrap_or(BillingPeriodEnum::Monthly)
}

pub fn process_create_subscription_components(
    param: &Option<CreateSubscriptionComponents>,
    map: &HashMap<PlanVersionId, Vec<PriceComponent>>,
    sub: &SubscriptionNew,
    products: &HashMap<common_domain::ids::ProductId, crate::domain::Product>,
    resolved: &ResolvedCustomComponents,
    product_family_id: ProductFamilyId,
    currency: &str,
) -> Result<
    (
        Vec<SubscriptionComponentNewInternal>,
        Vec<PendingMaterialization>,
    ),
    Report<StoreError>,
> {
    let mut processed_components = Vec::new();
    let mut pending_materializations = Vec::new();

    let (parameterized_components, remove_components) = if let Some(p) = param {
        (&p.parameterized_components, &p.remove_components)
    } else {
        (&Vec::new(), &Vec::new())
    };

    let binding = vec![];
    let plan_price_components = map.get(&sub.plan_version_id).unwrap_or(&binding);

    for c in plan_price_components {
        let component_id = c.id;

        // Check overridden_components first (pre-resolved in gather phase).
        // If there's also a parameterization for this component, apply its parameters
        // (e.g. initial_slot_count) to the override's fee.
        if let Some(resolved_override) = resolved.overrides.get(&component_id) {
            let mut fee = resolved_override.fee.clone();
            if let Some(parameterized) = parameterized_components
                .iter()
                .find(|p| p.component_id == component_id)
            {
                fee.apply_parameters(&parameterized.parameters);
            }
            let idx = processed_components.len();
            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: resolved_override.price_component_id,
                product_id: resolved_override.existing_product_id(),
                name: resolved_override.name.clone(),
                period: resolved_override.period,
                fee,
                is_override: true,
                price_id: resolved_override.existing_price_id(),
            });
            if resolved_override.needs_materialization() {
                pending_materializations.push(PendingMaterialization {
                    component_index: idx,
                    name: resolved_override.name.clone(),
                    product_ref: resolved_override.product_ref.clone(),
                    price_entry: resolved_override.price_entry.clone(),
                    product_family_id,
                    currency: currency.to_string(),
                });
            }
            continue;
        }

        // Check parameterized_components (without override)
        if let Some(parameterized) = parameterized_components
            .iter()
            .find(|p| p.component_id == component_id)
        {
            use crate::domain::price_components::ComponentParameters;
            let params = ComponentParameters {
                initial_slot_count: parameterized.parameters.initial_slot_count,
                billing_period: parameterized.parameters.billing_period,
                committed_capacity: parameterized.parameters.committed_capacity,
            };
            let resolved = c
                .resolve_fee(products, Some(&params))
                .map_err(Report::new)?;

            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: Some(c.id),
                product_id: c.product_id,
                name: c.name.clone(),
                period: resolved.period,
                fee: resolved.fee,
                is_override: false,
                price_id: resolved.price_id,
            });
            continue;
        }

        // Check if the component is in remove_components
        if remove_components.contains(&component_id) {
            continue;
        }

        // Default: resolve via v2 path or legacy path
        let resolved = c.resolve_fee(products, None).map_err(Report::new)?;

        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: Some(c.id),
            product_id: c.product_id,
            name: c.name.clone(),
            period: resolved.period,
            fee: resolved.fee,
            is_override: false,
            price_id: resolved.price_id,
        });
    }

    // Append pre-resolved extra components
    for extra in &resolved.extras {
        let idx = processed_components.len();
        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: None,
            product_id: extra.existing_product_id(),
            name: extra.name.clone(),
            period: extra.period,
            fee: extra.fee.clone(),
            is_override: false,
            price_id: extra.existing_price_id(),
        });
        if extra.needs_materialization() {
            pending_materializations.push(PendingMaterialization {
                component_index: idx,
                name: extra.name.clone(),
                product_ref: extra.product_ref.clone(),
                price_entry: extra.price_entry.clone(),
                product_family_id,
                currency: currency.to_string(),
            });
        }
    }

    Ok((processed_components, pending_materializations))
}

impl SubscriptionDetails {
    pub fn calculate_cancellable_end_of_period_date(&self, now: NaiveDate) -> Option<NaiveDate> {
        // to calculate last billing period :
        // if there is a commitment, use that commitment (currently no commitment so let's ignore)
        // else, we take the longest period from the main components (rate/slots/capacity), as that's what the user has already paid
        // else, that mean we're arrear and it's monthly. TODO
        let period = self
            .price_components
            .iter()
            .filter(|c| c.is_standard())
            .map(|c| c.period)
            .max_by(|a, b| a.as_months().cmp(&b.as_months()))
            .and_then(|p| p.as_billing_period_opt())
            .unwrap_or(BillingPeriodEnum::Monthly);

        let periods = calculate_advance_period_range(
            now,
            u32::from(self.subscription.billing_day_anchor),
            false,
            &period,
        );

        Some(periods.end)
    }
}

pub async fn is_paid_trial(
    conn: &mut PgConn,
    plan_version_id: PlanVersionId,
    tenant_id: TenantId,
    has_trial: bool,
) -> StoreResult<bool> {
    if !has_trial {
        return Ok(false);
    }

    let plan_with_version = PlanRow::get_with_version(conn, plan_version_id, tenant_id).await?;
    Ok(plan_with_version
        .version
        .map(|v| !v.trial_is_free)
        .unwrap_or(false))
}

impl Services {
    pub(super) async fn insert_created_outbox_events_tx(
        &self,
        conn: &mut PgConn,
        created: &[CreatedSubscription],
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let ids = created.iter().map(|c| c.id).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(());
        }
        let subscriptions: Vec<Subscription> =
            SubscriptionRow::list_subscriptions_by_ids(conn, &tenant_id, &ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?;
        let outbox_events: Vec<OutboxEvent> = subscriptions
            .into_iter()
            .map(|s| OutboxEvent::subscription_created(s.into()))
            .collect();
        self.store
            .internal
            .insert_outbox_events_tx(conn, outbox_events)
            .await
    }
}

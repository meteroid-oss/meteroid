use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{
    CreateSubscriptionAddOns, CreateSubscriptionComponents, CreatedSubscription, PriceComponent,
    Subscription, SubscriptionAddOnCustomization, SubscriptionAddOnNewInternal,
    SubscriptionComponentNewInternal, SubscriptionDetails, SubscriptionFee, SubscriptionNew,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use chrono::NaiveDate;
use common_utils::decimals::ToSubunit;
use diesel_models::errors::DatabaseError;
use error_stack::{Report, report};
use itertools::Itertools;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::add_ons::AddOn;
use crate::domain::coupons::{Coupon, CouponDiscount};
use crate::domain::outbox_event::OutboxEvent;
use crate::repositories::historical_rates::HistoricalRatesInterface;
use diesel_models::DbResult;
use diesel_models::applied_coupons::{AppliedCouponRow, AppliedCouponRowNew};
use diesel_models::coupons::CouponRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use rust_decimal::prelude::*;

use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use common_domain::ids::{CouponId, PlanVersionId, TenantId};
use error_stack::Result;

pub fn calculate_mrr(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    let mut total_cents = 0;

    let period_as_months = period.as_months() as i64;

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
                (*initial_slots as i64) * unit_rate.to_subunit_opt(precision).unwrap_or(0);
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
) -> Result<Vec<SubscriptionAddOnNewInternal>, StoreError> {
    let mut processed_add_ons = Vec::new();

    if let Some(create) = create {
        for cs_ao in &create.add_ons {
            let add_on = add_ons.iter().find(|x| x.id == cs_ao.add_on_id).ok_or(
                StoreError::ValueNotFound(format!("add-on {} not found", cs_ao.add_on_id)),
            )?;

            match &cs_ao.customization {
                SubscriptionAddOnCustomization::None => {
                    let (period, fee) = add_on.fee.to_subscription_fee()?;
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: add_on.name.clone(),
                        period,
                        fee,
                    });
                }
                SubscriptionAddOnCustomization::Override(override_) => {
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: override_.name.clone(),
                        period: override_.period.clone(),
                        fee: override_.fee.clone(),
                    });
                }
                SubscriptionAddOnCustomization::Parameterization(param) => {
                    let (period, fee) = add_on.fee.to_subscription_fee_parameterized(
                        &param.initial_slot_count,
                        &param.billing_period,
                        &param.committed_capacity,
                    )?;
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: add_on.name.clone(),
                        period,
                        fee,
                    });
                }
            }
        }
    }

    Ok(processed_add_ons)
}

pub fn process_create_subscription_coupons(
    subscription: &SubscriptionRowNew,
    coupons: &[Coupon],
) -> Result<Vec<AppliedCouponRowNew>, StoreError> {
    let processed_coupons = coupons
        .iter()
        .unique_by(|x| x.id)
        .map(|x| AppliedCouponRowNew {
            id: Uuid::now_v7(),
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

    let now = chrono::Utc::now().naive_utc();

    for coupon in coupons.iter() {
        let applied_coupon = subscription_coupons
            .iter()
            .find(|x| x.coupon_id == coupon.id)
            .ok_or(report!(DatabaseError::ValidationError(format!(
                "Applied coupon {} not found",
                coupon.code
            ))))?;

        let subscription = subscriptions
            .iter()
            .find(|x| x.id == applied_coupon.subscription_id)
            .ok_or(report!(DatabaseError::ValidationError(format!(
                "Subscription {} not found",
                applied_coupon.subscription_id
            ))))?;

        // check expired coupons
        if coupon.expires_at.is_some_and(|x| x <= now) {
            return Err(report!(DatabaseError::ValidationError(format!(
                "coupon {} is expired",
                coupon.code
            )))
            .into());
        }
        // check archived coupons
        if coupon.archived_at.is_some() {
            return Err(report!(DatabaseError::ValidationError(format!(
                "coupon {} is archived",
                coupon.code
            )))
            .into());
        }

        // TEMPORARY CHECK: currency is the same as in subscription
        let discount: CouponDiscount =
            serde_json::from_value(coupon.discount.clone()).map_err(|_| {
                report!(DatabaseError::ValidationError(format!(
                    "Discount serde error for coupon {}",
                    &coupon.code
                )))
            })?;

        if discount
            .currency()
            .is_some_and(|x| x != subscription.currency)
        {
            return Err(report!(DatabaseError::ValidationError(format!(
                "coupon {} currency does not match subscription currency",
                coupon.code
            )))
            .into());
        }
    }

    // check if the coupon has reached its redemption limit
    let subscriptions_by_coupon: HashMap<CouponId, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);
    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        let coupon = coupons.iter().find(|x| x.id == coupon_id).ok_or(report!(
            DatabaseError::ValidationError(format!("coupon {} not found", coupon_id))
        ))?;

        if let Some(redemption_limit) = coupon.redemption_limit {
            if redemption_limit < subscriptions_count as i32 + coupon.redemption_count {
                return Err(report!(DatabaseError::ValidationError(format!(
                    "coupon {} has reached its maximum redemptions",
                    coupon.code
                )))
                .into());
            }
        }
    }

    // check non-reusable coupons
    let non_reusable_coupons = coupons.iter().filter(|x| !x.reusable).collect::<Vec<_>>();
    if !non_reusable_coupons.is_empty() {
        let non_reusable_coupons_ids = non_reusable_coupons
            .iter()
            .map(|x| x.id)
            .collect::<Vec<_>>();

        let db_customers_by_coupon =
            CouponRow::customers_count(tx_conn, &non_reusable_coupons_ids).await?;

        for coupon in non_reusable_coupons {
            if db_customers_by_coupon.contains_key(&coupon.id) {
                return Err(report!(DatabaseError::ValidationError(format!(
                    "coupon {} is not reusable",
                    coupon.code
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
                total = total * percentage / Decimal::new(100, 0)
            }
            CouponDiscount::Fixed {
                currency,
                amount: fixed_amount,
            } => {
                let discount_amount = if currency != subscription_currency {
                    let rate = store
                        .get_historical_rate(
                            currency,
                            subscription_currency,
                            chrono::Utc::now().date_naive(),
                        )
                        .await?
                        .ok_or(StoreError::ValueNotFound(format!(
                            "historical rate from {} to {}",
                            currency, subscription_currency
                        )))?
                        .rate;

                    fixed_amount * Decimal::from_f32(rate).unwrap_or(Decimal::ZERO)
                } else {
                    *fixed_amount
                };

                total = (total - discount_amount).max(Decimal::ZERO)
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
        .filter_map(|x| x.as_billing_period_opt())
        .min()
        .unwrap_or(BillingPeriodEnum::Monthly)
}

pub fn process_create_subscription_components(
    param: &Option<CreateSubscriptionComponents>,
    map: &HashMap<PlanVersionId, Vec<PriceComponent>>,
    sub: &SubscriptionNew,
) -> Result<Vec<SubscriptionComponentNewInternal>, StoreError> {
    let mut processed_components = Vec::new();

    let (parameterized_components, overridden_components, extra_components, remove_components) =
        if let Some(p) = param {
            (
                &p.parameterized_components,
                &p.overridden_components,
                &p.extra_components,
                &p.remove_components,
            )
        } else {
            (&Vec::new(), &Vec::new(), &Vec::new(), &Vec::new())
        };

    let binding = vec![];
    let plan_price_components = map.get(&sub.plan_version_id).unwrap_or(&binding);

    let mut removed_components = Vec::new();

    // TODO should we add a quick_param or something to not require the component id when creating subscription without complex parameterization ?
    // basically a top level params with period, initial slots, committed capacity, that can be overriden at the component level

    for c in plan_price_components {
        let c = c.clone();
        let component_id = c.id;

        // Check parameterized_components
        if let Some(parameterized) = parameterized_components
            .iter()
            .find(|p| p.component_id == component_id)
        {
            let (period, fee) = c.fee.to_subscription_fee_parameterized(
                &parameterized.parameters.initial_slot_count,
                &parameterized.parameters.billing_period,
                &parameterized.parameters.committed_capacity,
            )?;
            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: Some(c.id),
                product_id: c.product_id,
                name: c.name.clone(),
                period,
                fee,
                is_override: false,
            });
            continue;
        }

        // Check overridden_components
        if let Some(overridden) = overridden_components
            .iter()
            .find(|o| o.component_id == component_id)
        {
            let mut component = overridden.component.clone();
            component.is_override = true;
            processed_components.push(component);
            continue;
        }

        // Check if the component is in remove_components
        if remove_components.contains(&component_id) {
            removed_components.push(component_id);
            continue;
        }

        let (period, fee) = c.fee.to_subscription_fee()?;

        // If the component is not in any of the lists, add it as is
        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: Some(c.id),
            product_id: c.product_id,
            name: c.name.clone(),
            period,
            fee,
            is_override: false,
        });
    }

    // Add extra components
    for extra in extra_components {
        processed_components.push(extra.component.clone());
    }

    Ok(processed_components)
}

impl SubscriptionDetails {
    pub fn calculate_cancellable_end_of_period_date(&self, now: NaiveDate) -> Option<NaiveDate> {
        // to calculate last billing period :
        // if there is a commitment, use that commitment (currently no commitment so let's ignore)
        // else, we take the longest period from the main components (rate/slots/capacity), as that's what the user has already paid
        // else, that mean we're arrear and it's monthly. TODO

        let standard_components = self
            .price_components
            .iter()
            .filter(|c| c.is_standard())
            .collect::<Vec<_>>();
        let period = standard_components
            .iter()
            .map(|c| c.period.clone())
            .max_by(|a, b| a.as_months().cmp(&b.as_months()))
            .and_then(|p| p.as_billing_period_opt())
            .unwrap_or(BillingPeriodEnum::Monthly);

        let periods = calculate_advance_period_range(
            now,
            self.subscription.billing_day_anchor as u32,
            false,
            &period,
        );

        Some(periods.end)
    }
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
                .map(|s| s.try_into())
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

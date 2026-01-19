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
use error_stack::Report;
use itertools::Itertools;
use std::collections::HashMap;

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
use common_domain::ids::{AppliedCouponId, BaseId, CouponId, PlanVersionId, TenantId};

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
) -> Result<Vec<SubscriptionAddOnNewInternal>, Report<StoreError>> {
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
                        period: override_.period,
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
) -> Result<Vec<SubscriptionComponentNewInternal>, Report<StoreError>> {
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

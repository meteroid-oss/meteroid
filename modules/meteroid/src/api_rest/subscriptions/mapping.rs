use crate::api_rest::currencies;
use crate::api_rest::shared::conversions::FromRestOpt;
use crate::api_rest::subscriptions::model::{
    Subscription, SubscriptionActivationCondition, SubscriptionCreateRequest, SubscriptionDetails,
    SubscriptionStatus,
};
use crate::errors::RestApiError;
use common_domain::ids::{CouponId, CustomerId};
use meteroid_store::domain;
use meteroid_store::domain::{CreateSubscription, SubscriptionNew, SubscriptionStatusEnum};
use uuid::Uuid;

pub fn domain_to_rest(s: domain::Subscription) -> Result<Subscription, RestApiError> {
    Ok(Subscription {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day_anchor: s.billing_day_anchor as i16,
        currency: currencies::mapping::from_str(s.currency.as_str())?,
        plan_id: s.plan_id,
        plan_name: s.plan_name,
        plan_version_id: s.plan_version_id,
        plan_version: s.version,
        status: subscription_status_to_rest(s.status),
        current_period_start: s.current_period_start,
        current_period_end: s.current_period_end,
    })
}

pub fn domain_to_rest_details(
    s: domain::SubscriptionDetails,
) -> Result<SubscriptionDetails, RestApiError> {
    Ok(SubscriptionDetails {
        id: s.subscription.id,
        customer_id: s.subscription.customer_id,
        customer_name: s.subscription.customer_name,
        customer_alias: s.subscription.customer_alias,
        billing_day_anchor: s.subscription.billing_day_anchor as i16,
        currency: currencies::mapping::from_str(s.subscription.currency.as_str())?,
        plan_id: s.subscription.plan_id,
        plan_name: s.subscription.plan_name,
        plan_version_id: s.subscription.plan_version_id,
        plan_version: s.subscription.version,
        status: subscription_status_to_rest(s.subscription.status),
        current_period_start: s.subscription.current_period_start,
        current_period_end: s.subscription.current_period_end,
    })
}

pub fn rest_to_domain_create_request(
    resolved_customer_id: CustomerId,
    resolved_coupon_ids: Option<Vec<CouponId>>,
    created_by: Uuid,
    sub: SubscriptionCreateRequest,
) -> Result<CreateSubscription, RestApiError> {
    let converted = CreateSubscription {
        subscription: SubscriptionNew {
            plan_version_id: sub.plan_version_id,
            customer_id: resolved_customer_id,
            trial_duration: sub.trial_days,
            start_date: sub.start_date,
            end_date: sub.end_date,
            billing_day_anchor: sub.billing_day_anchor,
            net_terms: sub.net_terms,
            invoice_memo: sub.invoice_memo,
            created_by,
            activation_condition: match sub.activation_condition {
                SubscriptionActivationCondition::OnStart => {
                    domain::enums::SubscriptionActivationCondition::OnStart
                }
                SubscriptionActivationCondition::OnCheckout => {
                    domain::enums::SubscriptionActivationCondition::OnCheckout
                }
                SubscriptionActivationCondition::Manual => {
                    domain::enums::SubscriptionActivationCondition::Manual
                }
            },
            payment_strategy: None,      // todo
            auto_advance_invoices: true, // todo
            invoice_threshold: rust_decimal::Decimal::from_rest_opt(sub.invoice_threshold)?,
            billing_start_date: None,   // todo
            charge_automatically: true, // todo
        },
        price_components: None, // todo
        add_ons: None,          // todo
        coupons: resolved_coupon_ids.map(|ids| domain::CreateSubscriptionCoupons {
            coupons: ids
                .into_iter()
                .map(|coupon_id| domain::CreateSubscriptionCoupon { coupon_id })
                .collect(),
        }),
    };

    Ok(converted)
}

fn subscription_status_to_rest(status: SubscriptionStatusEnum) -> SubscriptionStatus {
    match status {
        SubscriptionStatusEnum::Active => SubscriptionStatus::Active,
        SubscriptionStatusEnum::Paused => SubscriptionStatus::Paused,
        SubscriptionStatusEnum::PendingActivation => SubscriptionStatus::PendingActivation,
        SubscriptionStatusEnum::PendingCharge => SubscriptionStatus::PendingCharge,
        SubscriptionStatusEnum::TrialActive => SubscriptionStatus::TrialActive,
        SubscriptionStatusEnum::TrialExpired => SubscriptionStatus::TrialExpired,
        SubscriptionStatusEnum::Suspended => SubscriptionStatus::Suspended,
        SubscriptionStatusEnum::Cancelled => SubscriptionStatus::Cancelled,
        SubscriptionStatusEnum::Completed => SubscriptionStatus::Completed,
        SubscriptionStatusEnum::Superseded => SubscriptionStatus::Superseded,
    }
}

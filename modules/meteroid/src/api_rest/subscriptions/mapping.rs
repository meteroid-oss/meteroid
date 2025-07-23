use crate::api_rest::currencies;
use crate::api_rest::subscriptions::model::{
    Subscription, SubscriptionAddOnCustomization, SubscriptionCreateRequest, SubscriptionDetails,
};
use crate::errors::RestApiError;
use common_domain::ids::{CouponId, CustomerId};
use meteroid_store::domain;
use meteroid_store::domain::{CreateSubscription, SubscriptionNew};
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
        status: s.status.into(),
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
        status: s.subscription.status.into(),
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
            activation_condition: sub.activation_condition.into(),
            payment_strategy: None,      // todo
            auto_advance_invoices: true, // todo
            invoice_threshold: sub.invoice_threshold,
            billing_start_date: None,   // todo
            charge_automatically: true, // todo
        },
        price_components: sub
            .price_components
            .map(|pc| domain::CreateSubscriptionComponents {
                parameterized_components: pc
                    .parameterized_components
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                overridden_components: pc
                    .overridden_components
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                extra_components: pc
                    .extra_components
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                remove_components: pc.remove_components.unwrap_or_default(),
            }),
        add_ons: sub.add_ons.map(|add_ons| domain::CreateSubscriptionAddOns {
            add_ons: add_ons
                .into_iter()
                .map(|add_on| domain::CreateSubscriptionAddOn {
                    add_on_id: add_on.add_on_id,
                    customization: match add_on.customization {
                        None => domain::SubscriptionAddOnCustomization::None,
                        Some(SubscriptionAddOnCustomization::Parameterization(p)) => {
                            domain::SubscriptionAddOnCustomization::Parameterization(p.into())
                        }
                        Some(SubscriptionAddOnCustomization::Override(o)) => {
                            domain::SubscriptionAddOnCustomization::Override(o.into())
                        }
                    },
                })
                .collect(),
        }),
        coupons: resolved_coupon_ids.map(|ids| domain::CreateSubscriptionCoupons {
            coupons: ids
                .into_iter()
                .map(|coupon_id| domain::CreateSubscriptionCoupon { coupon_id })
                .collect(),
        }),
    };

    Ok(converted)
}

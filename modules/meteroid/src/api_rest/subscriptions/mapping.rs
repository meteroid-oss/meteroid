use crate::api_rest::currencies;
use crate::api_rest::shared::conversions::FromRestOpt;
use crate::api_rest::subscriptions::model::{
    Subscription, SubscriptionActivationCondition, SubscriptionCreateRequest, SubscriptionDetails,
};
use crate::errors::RestApiError;
use common_domain::ids::CustomerId;
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
    })
}

pub fn rest_to_domain_create_request(
    resolved_customer_id: CustomerId,
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
        coupons: None,          // todo
    };

    Ok(converted)
}

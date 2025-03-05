use crate::api_rest::subscriptions::model::{Subscription, SubscriptionDetails};
use meteroid_store::domain;

pub fn domain_to_rest(s: domain::Subscription) -> Subscription {
    Subscription {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day_anchor: s.billing_day_anchor as i16,
        currency: s.currency,
    }
}

pub fn domain_to_rest_details(s: domain::SubscriptionDetails) -> SubscriptionDetails {
    SubscriptionDetails {
        id: s.subscription.id,
        customer_id: s.subscription.customer_id,
        customer_name: s.subscription.customer_name,
        customer_alias: s.subscription.customer_alias,
        billing_day_anchor: s.subscription.billing_day_anchor as i16,
        currency: s.subscription.currency,
    }
}

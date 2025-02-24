use crate::api_rest::subscriptions::model::{Subscription, SubscriptionDetails};
use meteroid_store::domain;

pub fn domain_to_rest(s: domain::Subscription) -> Subscription {
    Subscription {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day: s.billing_day,
        currency: s.currency,
    }
}

pub fn domain_to_rest_details(s: domain::SubscriptionDetails) -> SubscriptionDetails {
    SubscriptionDetails {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day: s.billing_day,
        currency: s.currency,
    }
}

use crate::api_rest::subscriptions::model::Subscription;
use crate::errors::RestApiError;
use meteroid_store::domain;

pub fn domain_to_rest(s: domain::Subscription) -> Result<Subscription, RestApiError> {
    Ok(Subscription {
        id: s.id,
        customer_id: s.customer_id,
        customer_name: s.customer_name,
        customer_alias: s.customer_alias,
        billing_day: s.billing_day,
        tenant_id: s.tenant_id,
        currency: s.currency,
    })
}

use crate::api_rest::subscriptions::model::Subscription;
use crate::errors::RestApiError;
use meteroid_store::domain;

pub fn domain_to_rest(s: domain::Subscription) -> Result<Subscription, RestApiError> {
    Ok(Subscription { id: s.id })
}

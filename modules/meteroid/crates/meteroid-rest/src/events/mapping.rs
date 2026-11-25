use crate::events::model;
use common_domain::ids::{AliasOr, CustomerId};
use common_utils::misc::UnwrapInfallible;
use metering_grpc::meteroid::metering::v1 as grpc;
use meteroid_store::clients::usage;
use std::str::FromStr;

/// An event's `customer_id` as v1 reads it: a Meteroid id, else the customer's alias, bare.
fn customer_of(raw: &str) -> grpc::event::CustomerId {
    let id_or_alias: AliasOr<CustomerId> = AliasOr::from_str(raw).unwrap_infallible();
    match id_or_alias {
        AliasOr::Id(id) => grpc::event::CustomerId::MeteroidCustomerId(id.to_string()),
        AliasOr::Alias(alias) => grpc::event::CustomerId::ExternalCustomerAlias(alias),
    }
}

pub fn rest_event_to_grpc(event: model::Event) -> grpc::Event {
    let customer_id = customer_of(&event.customer_id);

    grpc::Event {
        id: event.event_id,
        code: event.code,
        customer_id: Some(customer_id),
        timestamp: event.timestamp.unwrap_or_default(),
        properties: event.properties,
    }
}

pub fn usage_client_response_to_rest(
    resp: usage::IngestEventsResult,
) -> model::IngestEventsResponse {
    model::IngestEventsResponse {
        failures: resp
            .failures
            .into_iter()
            .map(|f| model::IngestFailure {
                event_id: f.event_id,
                reason: f.reason,
            })
            .collect(),
    }
}

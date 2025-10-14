use crate::api_rest::events::model;
use common_domain::ids::{AliasOr, CustomerId};
use common_utils::misc::UnwrapInfallible;
use metering_grpc::meteroid::metering::v1 as grpc;
use meteroid_store::clients::usage;
use std::str::FromStr;

pub fn rest_event_to_grpc(event: model::Event) -> grpc::Event {
    let id_or_alias: AliasOr<CustomerId> =
        AliasOr::from_str(event.customer_id.as_str()).unwrap_infallible();

    let customer_id = match id_or_alias {
        AliasOr::Id(id) => grpc::event::CustomerId::MeteroidCustomerId(id.to_string()),
        AliasOr::Alias(alias) => grpc::event::CustomerId::ExternalCustomerAlias(alias),
    };

    grpc::Event {
        id: event.event_id,
        code: event.code,
        customer_id: Some(customer_id),
        timestamp: event.timestamp,
        properties: event.properties,
    }
}

pub fn rest_request_to_usage_client(req: model::IngestEventsRequest) -> usage::IngestEventsRequest {
    usage::IngestEventsRequest {
        events: req.events.into_iter().map(rest_event_to_grpc).collect(),
        allow_backfilling: req.allow_backfilling.unwrap_or(false),
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

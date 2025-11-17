use super::EventsServiceComponents;
use crate::api::events::error::EventsApiError;
use common_grpc::middleware::server::auth::RequestExt;
use metering_grpc::meteroid::metering::v1::{
    event::CustomerId, query_raw_events_request::SortOrder as MeteringSortOrder,
};
use meteroid_grpc::meteroid::api::events::v1::events_service_server::EventsService;
use meteroid_grpc::meteroid::api::events::v1::{
    EventSummary, SearchEventsRequest, SearchEventsResponse, search_events_request::SortOrder,
};
use meteroid_store::clients::usage::EventSearchOptions;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl EventsService for EventsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn search_events(
        &self,
        request: Request<SearchEventsRequest>,
    ) -> Result<Response<SearchEventsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let sort_order = match req.sort_order() {
            SortOrder::TimestampDesc => MeteringSortOrder::TimestampDesc,
            SortOrder::TimestampAsc => MeteringSortOrder::TimestampAsc,
            SortOrder::IngestedDesc => MeteringSortOrder::IngestedDesc,
            SortOrder::IngestedAsc => MeteringSortOrder::IngestedAsc,
        };

        let search_options = EventSearchOptions {
            from: req.from,
            to: req.to,
            limit: req.limit,
            offset: req.offset,
            search: req.search,
            event_codes: req.event_codes,
            customer_ids: req.customer_ids,
            sort_order: sort_order.into(),
        };

        let result = self
            .usage_client
            .search_events(&tenant_id, search_options)
            .await
            .map_err(EventsApiError::from)?;

        let events = result
            .events
            .into_iter()
            .map(|event| EventSummary {
                id: event.id,
                code: event.code,
                customer_id: match event.customer_id {
                    Some(CustomerId::MeteroidCustomerId(id)) => id,
                    Some(CustomerId::ExternalCustomerAlias(alias)) => alias,
                    None => "unknown".to_string(),
                },
                timestamp: event
                    .timestamp
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .map(|dt| prost_types::Timestamp {
                        seconds: dt.timestamp(),
                        nanos: dt.timestamp_subsec_nanos() as i32,
                    })
                    .ok(),
                ingested_at: None, // TODO: Add ingested_at to Event proto
                properties: event.properties,
            })
            .collect();

        Ok(Response::new(SearchEventsResponse { events }))
    }
}

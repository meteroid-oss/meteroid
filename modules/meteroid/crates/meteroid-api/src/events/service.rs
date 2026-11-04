use super::EventsServiceComponents;
use crate::events::error::EventsApiError;
use common_domain::identifiers::validate_code;
use common_domain::ids::CustomerId as InternalCustomerId;
use common_domain::ids::TenantId;
use common_grpc::middleware::server::auth::RequestExt;
use metering_grpc::meteroid::metering::v1::{
    PropertyFilter as MeteringPropertyFilter, event::CustomerId,
    property_filter::FilterOp as MeteringFilterOp,
    query_raw_events_request::SortOrder as MeteringSortOrder,
};
use meteroid_grpc::meteroid::api::events::v1::events_service_server::EventsService;
use meteroid_grpc::meteroid::api::events::v1::{
    EventSummary, SearchEventsRequest, SearchEventsResponse,
    event_property_filter::FilterOp as EventFilterOp, search_events_request::SortOrder,
};
use meteroid_store::Store;
use meteroid_store::clients::usage::EventSearchOptions;
use meteroid_store::repositories::customers::CustomersInterface;
use std::collections::HashMap;
use tonic::{Request, Response, Status};

/// Resolved internal customer, keyed by the raw identity string carried on an event.
#[derive(Default)]
struct ResolvedCustomer {
    local_id: String,
    name: String,
}

#[tonic::async_trait]
impl EventsService for EventsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn search_events(
        &self,
        request: Request<SearchEventsRequest>,
    ) -> Result<Response<SearchEventsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        for code in &req.event_codes {
            validate_code(code).map_err(|e| Status::invalid_argument(e.to_string()))?;
        }

        let sort_order = match req.sort_order() {
            SortOrder::TimestampDesc => MeteringSortOrder::TimestampDesc,
            SortOrder::TimestampAsc => MeteringSortOrder::TimestampAsc,
            SortOrder::IngestedDesc => MeteringSortOrder::IngestedDesc,
            SortOrder::IngestedAsc => MeteringSortOrder::IngestedAsc,
        };

        let filters = req
            .filters
            .into_iter()
            .map(|f| {
                let op = match f.op() {
                    EventFilterOp::In => MeteringFilterOp::In,
                    EventFilterOp::NotIn => MeteringFilterOp::NotIn,
                };
                MeteringPropertyFilter {
                    property: f.property,
                    op: op as i32,
                    values: f.values,
                }
            })
            .collect();

        let search_options = EventSearchOptions {
            from: req.from,
            to: req.to,
            limit: req.limit,
            offset: req.offset,
            search: req.search,
            event_codes: req.event_codes,
            customer_ids: req.customer_ids,
            filters,
            sort_order: sort_order.into(),
        };

        let result = self
            .usage_client
            .search_events(&tenant_id, search_options)
            .await
            .map_err(EventsApiError::from)?;

        let raw_events = result.events;

        // The raw identity string this page will expose for each event (internal id
        // or external alias), used both as the response `customer_id` and as the
        // resolution-map key below.
        let raw_key = |customer_id: &Option<CustomerId>| -> String {
            match customer_id {
                Some(CustomerId::MeteroidCustomerId(id)) => id.clone(),
                Some(CustomerId::ExternalCustomerAlias(alias)) => alias.clone(),
                None => "unknown".to_string(),
            }
        };

        let resolved = resolve_customers(&self.store, tenant_id, &raw_events).await?;

        let events = raw_events
            .into_iter()
            .map(|event| {
                let key = raw_key(&event.customer_id);
                let resolved = resolved.get(&key);
                EventSummary {
                    id: event.id,
                    code: event.code,
                    customer_id: key,
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
                    customer_local_id: resolved.map(|r| r.local_id.clone()).unwrap_or_default(),
                    customer_name: resolved.map(|r| r.name.clone()).unwrap_or_default(),
                }
            })
            .collect();

        Ok(Response::new(SearchEventsResponse { events }))
    }
}

/// Batch-resolve the customer identity carried by each event (an internal id or an
/// external alias) to the internal customer's id + display name, so the API can
/// render a named, linkable customer. Two batched queries at most (ids, aliases).
async fn resolve_customers(
    store: &Store,
    tenant_id: TenantId,
    events: &[metering_grpc::meteroid::metering::v1::Event],
) -> Result<HashMap<String, ResolvedCustomer>, Status> {
    let mut id_lookups: Vec<(String, InternalCustomerId)> = Vec::new();
    let mut aliases: Vec<String> = Vec::new();
    for event in events {
        match &event.customer_id {
            Some(CustomerId::MeteroidCustomerId(id)) => {
                if let Ok(parsed) = InternalCustomerId::from_proto(id) {
                    id_lookups.push((id.clone(), parsed));
                }
            }
            Some(CustomerId::ExternalCustomerAlias(alias)) => aliases.push(alias.clone()),
            None => {}
        }
    }

    let mut resolved: HashMap<String, ResolvedCustomer> = HashMap::new();

    let unique_ids: Vec<InternalCustomerId> = {
        let mut seen = std::collections::HashSet::new();
        id_lookups
            .iter()
            .filter(|(_, id)| seen.insert(*id))
            .map(|(_, id)| *id)
            .collect()
    };
    if !unique_ids.is_empty() {
        let by_id: HashMap<InternalCustomerId, (String, String)> = store
            .list_customers_by_ids(tenant_id, unique_ids)
            .await
            .map_err(EventsApiError::from)?
            .into_iter()
            .map(|c| (c.id, (c.id.as_proto(), c.name)))
            .collect();
        for (raw, id) in &id_lookups {
            if let Some((local_id, name)) = by_id.get(id) {
                resolved.insert(
                    raw.clone(),
                    ResolvedCustomer {
                        local_id: local_id.clone(),
                        name: name.clone(),
                    },
                );
            }
        }
    }

    aliases.sort();
    aliases.dedup();
    if !aliases.is_empty() {
        for brief in store
            .find_customer_ids_by_aliases(tenant_id, aliases)
            .await
            .map_err(EventsApiError::from)?
        {
            if let Some(alias) = brief.alias {
                resolved.insert(
                    alias,
                    ResolvedCustomer {
                        local_id: brief.id.as_proto(),
                        name: brief.name,
                    },
                );
            }
        }
    }

    Ok(resolved)
}

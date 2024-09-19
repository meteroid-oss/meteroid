use chrono::{DateTime, Utc};
use metering_grpc::meteroid::metering::v1::events_service_server::EventsService as EventsServiceGrpc;
use opentelemetry::KeyValue;
use std::sync::Arc;

use crate::cache::CUSTOMER_ID_CACHE;
use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::event::CustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestFailure, IngestRequest, IngestResponse};
use tonic::{Request, Response, Status};
use tracing::error;

use crate::ingest::domain::{FailedEvent, ProcessedEvent};
use crate::ingest::sinks::Sink;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use meteroid_grpc::meteroid::internal::v1::ResolveCustomerExternalIdsRequest;

#[derive(Clone)]
pub struct EventsService {
    pub internal_client: InternalServiceClient<LayeredClientService>,
    pub sink: Arc<dyn Sink + Send + Sync>,
}

impl EventsService {
    pub fn new(
        internal_client: InternalServiceClient<LayeredClientService>,
        sink: Arc<dyn Sink + Send + Sync>,
    ) -> Self {
        EventsService {
            internal_client,
            sink,
        }
    }
}

#[tonic::async_trait]
impl EventsServiceGrpc for EventsService {
    #[tracing::instrument(skip(self, request))]
    async fn ingest(
        &self,
        request: Request<IngestRequest>,
    ) -> Result<Response<IngestResponse>, Status> {
        let tenant_id = request.tenant()?.to_string();

        let req = request.into_inner();

        let events = req.events;

        let allow_backfilling = req.allow_backfilling;

        if events.is_empty() {
            return Err(Status::invalid_argument("No events provided"));
        } else if events.len() > 500 {
            return Err(Status::invalid_argument("Too many events provided"));
        }

        let mut failed_events = vec![];

        // optional checks related to the tenant ? (or cloud only, ex: free limits)

        // - get the customer_id from external_customer_id as necessary
        let mut resolved = vec![];
        let mut unresolved = vec![];
        let mut unresolved_ids = vec![];

        let now = chrono::Utc::now();

        for event in events {
            match validate_event(&event, &now, allow_backfilling) {
                Ok((id, ts)) => match id {
                    CustomerId::MeteroidCustomerId(meteroid_id) => resolved.push(
                        to_processed_event(event, meteroid_id, tenant_id.clone(), ts),
                    ),
                    CustomerId::ExternalCustomerId(external_id) => {
                        let from_cache =
                            CUSTOMER_ID_CACHE.get(&(tenant_id.clone(), external_id.clone()));
                        match from_cache {
                            Some(meteroid_id) => resolved.push(to_processed_event(
                                event,
                                meteroid_id.clone(),
                                tenant_id.clone(),
                                ts,
                            )),
                            None => {
                                unresolved_ids.push(external_id.clone());
                                unresolved.push((event, external_id.clone(), ts))
                            }
                        }
                    }
                },
                Err(e) => {
                    failed_events.push(FailedEvent {
                        event,
                        reason: e.to_string(),
                    });
                }
            };
        }

        if !unresolved_ids.is_empty() {
            // we call the api to resolve customers by external id & tenant

            let mut client = self.internal_client.clone();

            let res = client
                .resolve_customer_external_ids(ResolveCustomerExternalIdsRequest {
                    tenant_id: tenant_id.clone(),
                    external_ids: unresolved_ids,
                })
                .await
                .map_err(|e| {
                    Status::internal("Unable to resolve external ids")
                        .set_source(Arc::new(e))
                        .clone()
                })?;

            let res = res.into_inner();

            res.unresolved_ids.into_iter().for_each(|external_id| {
                failed_events.push(FailedEvent {
                    event: unresolved
                        .iter()
                        .find(|(_, id, _)| id == &external_id)
                        .unwrap()
                        .0
                        .clone(),
                    reason: "Unable to resolve external id".to_string(),
                })
            });

            res.customers.into_iter().for_each(|customer| {
                CUSTOMER_ID_CACHE.insert(
                    (tenant_id.clone(), customer.external_id.clone()),
                    customer.meteroid_id.clone(),
                );
                let (event, _, ts) = unresolved
                    .iter()
                    .find(|(_, id, _)| id == &customer.external_id)
                    .unwrap();

                resolved.push(to_processed_event(
                    event.clone(),
                    customer.meteroid_id,
                    tenant_id.clone(),
                    *ts,
                ))
            })
        }

        let default_attributes = &[
            KeyValue {
                key: "tenant_id".into(),
                value: tenant_id.into(),
            }, // add key ?
        ];

        let res = self
            .sink
            .send(resolved, default_attributes)
            .await
            .map_err(|e| {
                Status::internal("Unable to send events")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let mut failures: Vec<IngestFailure> = failed_events
            .into_iter()
            .map(|e| IngestFailure {
                idempotency_key: e.event.event_id,
                reason: e.reason,
            })
            .collect();

        failures.extend(res.into_iter().map(|rec| IngestFailure {
            idempotency_key: rec.event.event_id,
            reason: rec.error.to_string(),
        }));

        if !failures.is_empty() {
            error!("Failed count {}", failures.len());
        }
        Ok(Response::new(IngestResponse { failures }))
    }
}

fn to_processed_event(
    event: Event,
    customer_id: String,
    tenant_id: String,
    ts: DateTime<Utc>,
) -> ProcessedEvent {
    ProcessedEvent {
        event_id: event.event_id,
        event_name: event.event_name,
        customer_id,
        tenant_id,
        event_timestamp: ts.naive_utc(),
        properties: event.properties,
    }
}

fn validate_event(
    event: &Event,
    now: &DateTime<Utc>,
    allow_backfill: bool,
) -> Result<(CustomerId, DateTime<Utc>), String> {
    let customer = event.customer_id.as_ref().ok_or("No customer provided")?;

    let ts_opt = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
        .map(|ts| ts.to_utc())
        .ok();
    let ts = match ts_opt {
        Some(ts) => {
            let diff = ts - *now;

            if diff > chrono::Duration::hours(1) {
                Err(format!("Timestamp is too far in the future : {}", diff))
            } else if !allow_backfill && -diff > chrono::Duration::days(1) {
                // TODO use tenant grace period
                Err(format!("Timestamp is too far in the past : {}", diff).to_string())
            } else {
                Ok(ts)
            }
        }
        None => Ok(*now),
    }?;

    Ok((customer.clone(), ts))
}

use chrono::{DateTime, Utc};
use common_domain::identifiers::validate_code;
use common_domain::ids::{CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::event::CustomerId as ProtoCustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestFailure};
use opentelemetry::KeyValue;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::Status;

use crate::cache::CUSTOMER_ID_CACHE;
use crate::ingest::domain::{FailedEvent, RawEvent};
use crate::ingest::sinks::Sink;
use common_grpc::middleware::client::LayeredClientService;
use meteroid_grpc::meteroid::internal::v1::ResolveCustomerAliasesRequest;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;

pub struct IngestResult {
    pub failures: Vec<IngestFailure>,
}

pub struct EventProcessor {
    pub internal_client: InternalServiceClient<LayeredClientService>,
    pub sink: Arc<dyn Sink + Send + Sync>,
}

impl EventProcessor {
    pub fn new(
        internal_client: InternalServiceClient<LayeredClientService>,
        sink: Arc<dyn Sink + Send + Sync>,
    ) -> Self {
        Self {
            internal_client,
            sink,
        }
    }

    pub async fn process_events(
        &self,
        events: Vec<Event>,
        tenant_id: TenantId,
        allow_backfilling: bool,
        fail_on_error: bool,
        keep_last: bool,
    ) -> Result<IngestResult, Status> {
        if events.is_empty() {
            return Err(Status::invalid_argument("No events provided"));
        }

        if events.len() > 500 {
            return Err(Status::invalid_argument(format!(
                "Too many events provided: {}. Maximum is 500",
                events.len()
            )));
        }

        let events_count = events.len();

        tracing::info!(
            "Processing {} events for tenant {}",
            events_count,
            tenant_id
        );

        let mut failed_events = vec![];
        let mut resolved = vec![];

        let mut unresolved_by_alias: HashMap<String, Vec<(Event, DateTime<Utc>, usize)>> =
            HashMap::new();

        let now = Utc::now();
        let stamp = |event, customer_id, ts, position| {
            to_domain_event(event, customer_id, tenant_id, ts, now, position, keep_last)
        };

        for (position, event) in events.into_iter().enumerate() {
            match validate_event(&event, &now, allow_backfilling) {
                Ok((id, ts)) => match id {
                    ProtoCustomerId::MeteroidCustomerId(id) => match CustomerId::from_proto(id) {
                        Ok(customer_id) => resolved.push(stamp(event, customer_id, ts, position)),
                        Err(e) => failed_events.push(FailedEvent {
                            event,
                            reason: e.to_string(),
                        }),
                    },
                    ProtoCustomerId::ExternalCustomerAlias(alias) => {
                        let from_cache = CUSTOMER_ID_CACHE.get(&(tenant_id, alias.clone()));
                        match from_cache {
                            Some(meteroid_id) => {
                                resolved.push(stamp(event, meteroid_id, ts, position))
                            }
                            None => {
                                unresolved_by_alias
                                    .entry(alias)
                                    .or_default()
                                    .push((event, ts, position));
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
            }
        }

        if fail_on_error && !failed_events.is_empty() {
            return Ok(IngestResult {
                failures: into_failures(failed_events),
            });
        }

        if !unresolved_by_alias.is_empty() {
            let unresolved_aliases: Vec<String> = unresolved_by_alias.keys().cloned().collect();

            tracing::info!(
                "Resolving {} customer aliases for {} unresolved events",
                unresolved_aliases.len(),
                unresolved_by_alias.values().map(Vec::len).sum::<usize>()
            );

            let res = self
                .internal_client
                .clone()
                .resolve_customer_aliases(ResolveCustomerAliasesRequest {
                    tenant_id: tenant_id.as_proto(),
                    aliases: unresolved_aliases,
                })
                .await
                .map_err(|e| {
                    Status::internal("Unable to resolve external ids")
                        .set_source(Arc::new(e))
                        .clone()
                })?;

            let res = res.into_inner();

            for unresolved_alias in res.unresolved_aliases {
                if let Some(events_for_alias) = unresolved_by_alias.remove(&unresolved_alias) {
                    for (event, _, _) in events_for_alias {
                        failed_events.push(FailedEvent {
                            event,
                            reason: format!("Unable to resolve customer alias: {unresolved_alias}"),
                        });
                    }
                }
            }

            for customer in res.customers {
                let customer_id = CustomerId::from_proto(customer.local_id.clone())?;

                CUSTOMER_ID_CACHE.insert((tenant_id, customer.alias.clone()), customer_id);

                if let Some(events_for_alias) = unresolved_by_alias.remove(&customer.alias) {
                    tracing::debug!(
                        "Resolved alias {} to customer {}, processing {} events",
                        customer.alias,
                        customer_id,
                        events_for_alias.len()
                    );

                    for (event, ts, position) in events_for_alias {
                        resolved.push(stamp(event, customer_id, ts, position));
                    }
                }
            }

            // Defensive: aliases neither resolved nor reported as unresolved (shouldn't happen).
            for (alias, events_for_alias) in unresolved_by_alias {
                tracing::warn!(
                    "Alias {} was not in resolved or unresolved lists, marking {} events as failed",
                    alias,
                    events_for_alias.len()
                );
                for (event, _, _) in events_for_alias {
                    failed_events.push(FailedEvent {
                        event,
                        reason: format!("Customer alias not found in resolution response: {alias}"),
                    });
                }
            }

            if fail_on_error && !failed_events.is_empty() {
                return Ok(IngestResult {
                    failures: into_failures(failed_events),
                });
            }
        }

        // Alias lookups put events out of order; Kafka order is what the consumer's first-wins uses.
        resolved.sort_by_key(|event| event.ingested_at);

        let default_attributes = &[KeyValue::new("tenant_id", tenant_id.as_proto())];

        tracing::info!(
            "Sending {} resolved events to sink (originally {} events, {} failed validation)",
            resolved.len(),
            events_count,
            failed_events.len()
        );

        let sink_result = self
            .sink
            .send(resolved, default_attributes)
            .await
            .map_err(|e| {
                Status::internal("Unable to send events")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let mut failures = into_failures(failed_events);
        let sink_failures = sink_result.len();
        failures.extend(sink_result.into_iter().map(|rec| IngestFailure {
            event_id: rec.event.id,
            reason: rec.error.to_string(),
        }));

        if sink_failures > 0 {
            tracing::error!("{sink_failures} event(s) failed to reach the sink");
        }

        Ok(IngestResult { failures })
    }
}

fn into_failures(events: Vec<FailedEvent>) -> Vec<IngestFailure> {
    events
        .into_iter()
        .map(|e| IngestFailure {
            event_id: e.event.id,
            reason: e.reason,
        })
        .collect()
}

/// `ingested_at` is `now` plus the event's position in the request, so copies sent in one
/// request never tie and "first" and "last" stay well defined.
fn to_domain_event(
    event: Event,
    customer_id: CustomerId,
    tenant_id: TenantId,
    ts: DateTime<Utc>,
    now: DateTime<Utc>,
    position: usize,
    keep_last: bool,
) -> RawEvent {
    RawEvent {
        id: event.id,
        code: event.code,
        customer_id,
        tenant_id,
        timestamp: ts.naive_utc(),
        ingested_at: (now + chrono::Duration::nanoseconds(position as i64)).naive_utc(),
        properties: event.properties,
        keep_last,
    }
}

pub fn validate_event(
    event: &Event,
    now: &DateTime<Utc>,
    allow_backfill: bool,
) -> Result<(ProtoCustomerId, DateTime<Utc>), String> {
    validate_code(&event.code).map_err(|e| e.to_string())?;

    let customer = event.customer_id.as_ref().ok_or("No customer provided")?;

    let ts = if event.timestamp.is_empty() {
        *now
    } else {
        match DateTime::parse_from_rfc3339(&event.timestamp) {
            Ok(parsed_ts) => {
                let ts = parsed_ts.to_utc();
                let diff = ts - *now;

                if diff > chrono::Duration::hours(1) {
                    return Err(format!("Timestamp is too far in the future : {diff}"));
                }

                if !allow_backfill && -diff > chrono::Duration::days(1) {
                    return Err(format!("Timestamp is too far in the past : {diff}"));
                }

                ts
            }
            Err(e) => {
                return Err(format!("Invalid timestamp format: {e}"));
            }
        }
    };

    Ok((customer.clone(), ts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::errors::IngestError;
    use crate::ingest::sinks::FailedRecord;
    use common_config::auth::InternalAuthConfig;
    use common_grpc::middleware::client::build_layered_client_service;
    use meteroid_grpc::meteroid::internal::v1::internal_service_server::{
        InternalService, InternalServiceServer,
    };
    use meteroid_grpc::meteroid::internal::v1::{
        ListBillableMetricsRequest, ListBillableMetricsResponse, ReportApiKeyUsageRequest,
        ReportApiKeyUsageResponse, ResolveApiKeyRequest, ResolveApiKeyResponse,
        ResolveCustomerAliasesResponse, ResolvedId,
    };
    use tokio::sync::Mutex;
    use tonic::transport::Endpoint;
    use tonic::transport::server::TcpIncoming;
    use uuid::Uuid;

    struct AliasResolver;

    #[tonic::async_trait]
    impl InternalService for AliasResolver {
        async fn resolve_customer_aliases(
            &self,
            request: tonic::Request<ResolveCustomerAliasesRequest>,
        ) -> Result<tonic::Response<ResolveCustomerAliasesResponse>, Status> {
            let customers = request
                .into_inner()
                .aliases
                .into_iter()
                .map(|alias| ResolvedId {
                    alias,
                    local_id: CustomerId::from(Uuid::from_u128(7)).as_proto(),
                })
                .collect();
            Ok(tonic::Response::new(ResolveCustomerAliasesResponse {
                customers,
                unresolved_aliases: vec![],
            }))
        }

        async fn resolve_api_key(
            &self,
            _: tonic::Request<ResolveApiKeyRequest>,
        ) -> Result<tonic::Response<ResolveApiKeyResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn report_api_key_usage(
            &self,
            _: tonic::Request<ReportApiKeyUsageRequest>,
        ) -> Result<tonic::Response<ReportApiKeyUsageResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn list_billable_metrics(
            &self,
            _: tonic::Request<ListBillableMetricsRequest>,
        ) -> Result<tonic::Response<ListBillableMetricsResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
    }

    #[derive(Default)]
    struct CapturingSink(Mutex<Vec<RawEvent>>);

    #[tonic::async_trait]
    impl Sink for CapturingSink {
        async fn send(
            &self,
            events: Vec<RawEvent>,
            _: &[KeyValue],
        ) -> Result<Vec<FailedRecord>, IngestError> {
            self.0.lock().await.extend(events);
            Ok(vec![])
        }
    }

    fn internal_client() -> InternalServiceClient<LayeredClientService> {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = incoming.local_addr().unwrap();
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(InternalServiceServer::new(AliasResolver))
                .serve_with_incoming(incoming),
        );
        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect_lazy();
        InternalServiceClient::new(build_layered_client_service(
            channel,
            &InternalAuthConfig {
                hmac_secret: "secret".to_string().into(),
            },
        ))
    }

    fn event(id: &str, customer: ProtoCustomerId) -> Event {
        Event {
            id: id.to_string(),
            code: "calls".to_string(),
            customer_id: Some(customer),
            timestamp: String::new(),
            properties: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn events_reach_the_sink_in_request_order_across_alias_resolution() {
        let sink = Arc::new(CapturingSink::default());
        let processor = EventProcessor::new(internal_client(), sink.clone());
        let direct =
            || ProtoCustomerId::MeteroidCustomerId(CustomerId::from(Uuid::from_u128(8)).as_proto());
        let alias = || ProtoCustomerId::ExternalCustomerAlias(Uuid::now_v7().to_string());

        let events = vec![
            event("0", alias()),
            event("1", direct()),
            event("2", alias()),
            event("3", direct()),
        ];
        let result = processor
            .process_events(
                events,
                TenantId::from(Uuid::from_u128(1)),
                false,
                true,
                true,
            )
            .await
            .unwrap();
        assert!(result.failures.is_empty());

        let sent = sink.0.lock().await;
        let sent_order: Vec<&str> = sent.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(sent_order, ["0", "1", "2", "3"]);
        assert!(sent.iter().all(|e| e.keep_last));

        let start = sent[0].ingested_at;
        for (position, event) in sent.iter().enumerate() {
            assert_eq!(
                event.ingested_at - start,
                chrono::Duration::nanoseconds(position as i64)
            );
        }
    }
}

use chrono::{DateTime, Utc};
use metering_grpc::meteroid::metering::v1::event::CustomerId;
use metering_grpc::meteroid::metering::v1::{Event, IngestFailure};
use opentelemetry::KeyValue;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::Status;
use tracing::error;

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
        tenant_id: String,
        allow_backfilling: bool,
        fail_on_error: bool,
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

        let mut unresolved_by_alias: HashMap<String, Vec<(Event, DateTime<Utc>)>> = HashMap::new();

        let now = Utc::now();

        for event in events {
            match validate_event(&event, &now, allow_backfilling) {
                Ok((id, ts)) => match id {
                    CustomerId::MeteroidCustomerId(id) => {
                        resolved.push(to_domain_event(event, id, tenant_id.clone(), ts, now))
                    }
                    CustomerId::ExternalCustomerAlias(alias) => {
                        let from_cache = CUSTOMER_ID_CACHE.get(&(tenant_id.clone(), alias.clone()));
                        match from_cache {
                            Some(meteroid_id) => resolved.push(to_domain_event(
                                event,
                                meteroid_id.clone(),
                                tenant_id.clone(),
                                ts,
                                now,
                            )),
                            None => {
                                unresolved_by_alias
                                    .entry(alias)
                                    .or_default()
                                    .push((event, ts));
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
            let failures: Vec<IngestFailure> = failed_events
                .into_iter()
                .map(|e| IngestFailure {
                    event_id: e.event.id,
                    reason: e.reason,
                })
                .collect();

            return Ok(IngestResult { failures });
        }

        if !unresolved_by_alias.is_empty() {
            let unresolved_aliases: Vec<String> = unresolved_by_alias.keys().cloned().collect();

            tracing::info!(
                "Resolving {} customer aliases for {} unresolved events",
                unresolved_aliases.len(),
                unresolved_by_alias.values().map(|v| v.len()).sum::<usize>()
            );

            let res = self
                .internal_client
                .clone()
                .resolve_customer_aliases(ResolveCustomerAliasesRequest {
                    tenant_id: tenant_id.clone(),
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
                    for (event, _) in events_for_alias {
                        failed_events.push(FailedEvent {
                            event,
                            reason: format!(
                                "Unable to resolve customer alias: {}",
                                unresolved_alias
                            ),
                        });
                    }
                }
            }

            for customer in res.customers {
                CUSTOMER_ID_CACHE.insert(
                    (tenant_id.clone(), customer.alias.clone()),
                    customer.local_id.clone(),
                );

                if let Some(events_for_alias) = unresolved_by_alias.remove(&customer.alias) {
                    tracing::debug!(
                        "Resolved alias {} to customer {}, processing {} events",
                        customer.alias,
                        customer.local_id,
                        events_for_alias.len()
                    );

                    for (event, ts) in events_for_alias {
                        resolved.push(to_domain_event(
                            event,
                            customer.local_id.clone(),
                            tenant_id.clone(),
                            ts,
                            now,
                        ));
                    }
                }
            }

            // Handle any remaining unresolved events (shouldn't happen but be defensive)
            for (alias, events_for_alias) in unresolved_by_alias {
                tracing::warn!(
                    "Alias {} was not in resolved or unresolved lists, marking {} events as failed",
                    alias,
                    events_for_alias.len()
                );
                for (event, _) in events_for_alias {
                    failed_events.push(FailedEvent {
                        event,
                        reason: format!(
                            "Customer alias not found in resolution response: {}",
                            alias
                        ),
                    });
                }
            }

            // If fail_on_error is true and we have resolution failures, return immediately
            if fail_on_error && !failed_events.is_empty() {
                let failures: Vec<IngestFailure> = failed_events
                    .into_iter()
                    .map(|e| IngestFailure {
                        event_id: e.event.id,
                        reason: e.reason,
                    })
                    .collect();

                return Ok(IngestResult { failures });
            }
        }

        let default_attributes = &[KeyValue::new("tenant_id", tenant_id)];

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

        // Collect all failures
        let mut failures: Vec<IngestFailure> = failed_events
            .into_iter()
            .map(|e| IngestFailure {
                event_id: e.event.id,
                reason: e.reason,
            })
            .collect();

        failures.extend(sink_result.into_iter().map(|rec| IngestFailure {
            event_id: rec.event.id,
            reason: rec.error.to_string(),
        }));

        if !failures.is_empty() {
            error!("Failed count {}", failures.len());
        }

        Ok(IngestResult { failures })
    }
}

fn to_domain_event(
    event: Event,
    customer_id: String,
    tenant_id: String,
    ts: DateTime<Utc>,
    now: DateTime<Utc>,
) -> RawEvent {
    RawEvent {
        id: event.id,
        code: event.code,
        customer_id,
        tenant_id,
        timestamp: ts.naive_utc(),
        ingested_at: now.naive_utc(),
        properties: event.properties,
    }
}

pub fn validate_event(
    event: &Event,
    now: &DateTime<Utc>,
    allow_backfill: bool,
) -> Result<(CustomerId, DateTime<Utc>), String> {
    let customer = event.customer_id.as_ref().ok_or("No customer provided")?;

    let ts = if event.timestamp.is_empty() {
        *now
    } else {
        match chrono::DateTime::parse_from_rfc3339(&event.timestamp) {
            Ok(parsed_ts) => {
                let ts = parsed_ts.to_utc();
                let diff = ts - *now;

                if diff > chrono::Duration::hours(1) {
                    return Err(format!("Timestamp is too far in the future : {}", diff));
                }

                if !allow_backfill && -diff > chrono::Duration::days(1) {
                    return Err(format!("Timestamp is too far in the past : {}", diff));
                }

                ts
            }
            Err(e) => {
                return Err(format!("Invalid timestamp format: {}", e));
            }
        }
    };

    Ok((customer.clone(), ts))
}

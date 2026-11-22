use metering_grpc::meteroid::metering::v1::internal_events_service_server::InternalEventsService as InternalEventsServiceGrpc;
use std::sync::Arc;

use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::enqueue_event_change_request::EventChangeKind;
use metering_grpc::meteroid::metering::v1::{
    EnqueueEventChangeRequest, EnqueueEventChangeResponse, InternalIngestRequest,
    InternalIngestResponse,
};
use tonic::{Request, Response, Status};

use crate::ingest::common::{EventProcessor, validate_timestamp};
use crate::ingest::domain::{EventKind, RawEvent};
use crate::ingest::sinks::Sink;
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;

#[derive(Clone)]
pub struct InternalEventsService {
    processor: Arc<EventProcessor>,
}

impl InternalEventsService {
    pub fn new(
        internal_client: InternalServiceClient<LayeredClientService>,
        sink: Arc<dyn Sink + Send + Sync>,
    ) -> Self {
        InternalEventsService {
            processor: Arc::new(EventProcessor::new(internal_client, sink)),
        }
    }
}

#[tonic::async_trait]
impl InternalEventsServiceGrpc for InternalEventsService {
    #[tracing::instrument(skip(self, request))]
    async fn ingest_internal(
        &self,
        request: Request<InternalIngestRequest>,
    ) -> Result<Response<InternalIngestResponse>, Status> {
        let req = request.into_inner();

        if req.tenant_id.is_empty() {
            return Err(Status::invalid_argument("Tenant ID is required"));
        }

        let result = self
            .processor
            .process_events(
                req.events,
                TenantId::from_proto(req.tenant_id)?,
                req.allow_backfilling,
                req.fail_on_error,
                req.keep_last_events,
            )
            .await?;

        Ok(Response::new(InternalIngestResponse {
            failures: result.failures,
        }))
    }

    #[tracing::instrument(skip(self, request))]
    async fn enqueue_event_change(
        &self,
        request: Request<EnqueueEventChangeRequest>,
    ) -> Result<Response<EnqueueEventChangeResponse>, Status> {
        let change = to_change(request.into_inner())?;
        self.processor.enqueue_change(change).await?;
        Ok(Response::new(EnqueueEventChangeResponse {}))
    }
}

/// The change as a Kafka message: an amend's new row gets the caller's `ingested_at` stamp (within
/// the 35-day id window, at most 1 h ahead), or now. The stored code is kept, so none is sent.
fn to_change(req: EnqueueEventChangeRequest) -> Result<RawEvent, Status> {
    if req.keep_last_events {
        return Err(Status::invalid_argument(
            "keep-last tenants correct an event by sending it again",
        ));
    }
    if req.event_id.is_empty() {
        return Err(Status::invalid_argument("Event ID is required"));
    }
    let now = Utc::now();
    let ingested_at = match &req.ingested_at {
        None => now,
        Some(stamp) => DateTime::from_timestamp(stamp.seconds, stamp.nanos as u32)
            .filter(|stamp| *stamp - now <= chrono::Duration::hours(1))
            .filter(|stamp| now - *stamp < chrono::Duration::days(35))
            .ok_or_else(|| Status::invalid_argument("Invalid ingested_at"))?,
    };
    let (kind, timestamp, properties) = match req.kind() {
        EventChangeKind::Amend => {
            if req.timestamp.is_empty() {
                return Err(Status::invalid_argument("An amend needs the new timestamp"));
            }
            let ts = validate_timestamp(&req.timestamp, &now, req.allow_backfilling)
                .map_err(Status::invalid_argument)?;
            (EventKind::Amend, ts, req.properties)
        }
        EventChangeKind::Void => (EventKind::Void, now, Default::default()),
        EventChangeKind::Unspecified => {
            return Err(Status::invalid_argument("The change kind is required"));
        }
    };
    Ok(RawEvent {
        id: req.event_id,
        code: String::new(),
        customer_id: CustomerId::from_proto(req.customer_id)?,
        tenant_id: TenantId::from_proto(req.tenant_id)?,
        timestamp: timestamp.naive_utc(),
        ingested_at: ingested_at.naive_utc(),
        properties,
        keep_last: false,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn request(kind: EventChangeKind) -> EnqueueEventChangeRequest {
        EnqueueEventChangeRequest {
            tenant_id: TenantId::from(Uuid::from_u128(1)).as_proto(),
            customer_id: CustomerId::from(Uuid::from_u128(2)).as_proto(),
            event_id: "e".to_string(),
            kind: kind as i32,
            timestamp: Utc::now().to_rfc3339(),
            properties: [("v".to_string(), "1".to_string())].into(),
            allow_backfilling: false,
            keep_last_events: false,
            ingested_at: None,
        }
    }

    #[test]
    fn a_change_is_keyed_like_its_event() {
        let amend = to_change(request(EventChangeKind::Amend)).unwrap();
        assert_eq!(amend.kind, EventKind::Amend);
        assert_eq!(
            amend.key(),
            format!("{}:e", TenantId::from(Uuid::from_u128(1)))
        );
        assert_eq!(amend.properties["v"], "1");

        let void = to_change(request(EventChangeKind::Void)).unwrap();
        assert_eq!(void.kind, EventKind::Void);
        assert!(void.properties.is_empty());
    }

    fn stamp_with(req: &mut EnqueueEventChangeRequest, seconds: i64, nanos: i32) {
        let stamp = req.ingested_at.get_or_insert_default();
        (stamp.seconds, stamp.nanos) = (seconds, nanos);
    }

    #[test]
    fn an_amend_keeps_the_callers_stamp() {
        let stamp = Utc::now() - chrono::Duration::minutes(5) + chrono::Duration::nanoseconds(7);
        let mut req = request(EventChangeKind::Amend);
        stamp_with(
            &mut req,
            stamp.timestamp(),
            stamp.timestamp_subsec_nanos() as i32,
        );
        assert_eq!(to_change(req).unwrap().ingested_at, stamp.naive_utc());
    }

    #[test]
    fn invalid_changes_are_refused() {
        let mut keep_last = request(EventChangeKind::Void);
        keep_last.keep_last_events = true;
        let mut no_timestamp = request(EventChangeKind::Amend);
        no_timestamp.timestamp.clear();
        let mut too_old = request(EventChangeKind::Amend);
        too_old.timestamp = (Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        let mut stamped_ahead = request(EventChangeKind::Amend);
        stamp_with(
            &mut stamped_ahead,
            (Utc::now() + chrono::Duration::days(1)).timestamp(),
            0,
        );

        let mut stamped_before_the_id_window = request(EventChangeKind::Void);
        stamp_with(
            &mut stamped_before_the_id_window,
            (Utc::now() - chrono::Duration::days(36)).timestamp(),
            0,
        );

        for req in [
            keep_last,
            no_timestamp,
            too_old,
            stamped_ahead,
            stamped_before_the_id_window,
            request(EventChangeKind::Unspecified),
        ] {
            assert_eq!(
                to_change(req).unwrap_err().code(),
                tonic::Code::InvalidArgument
            );
        }
    }
}

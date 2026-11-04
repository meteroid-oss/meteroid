pub mod webhooks {
    use meteroid_app::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::webhooks::out::v1 as server;
    use meteroid_store::domain::enums as domain_enum;
    use meteroid_store::domain::webhooks_out as domain;

    pub fn disabled_reason_to_server(
        value: &domain_enum::WebhookEndpointDisabledReason,
    ) -> server::WebhookEndpointDisabledReasonEnum {
        match value {
            domain_enum::WebhookEndpointDisabledReason::Manual => {
                server::WebhookEndpointDisabledReasonEnum::Manual
            }
            domain_enum::WebhookEndpointDisabledReason::AutoFailures => {
                server::WebhookEndpointDisabledReasonEnum::AutoFailures
            }
            domain_enum::WebhookEndpointDisabledReason::Gone => {
                server::WebhookEndpointDisabledReasonEnum::Gone
            }
        }
    }

    pub fn delivery_status_to_server(
        value: &domain_enum::WebhookDeliveryStatus,
    ) -> server::WebhookDeliveryStatusEnum {
        match value {
            domain_enum::WebhookDeliveryStatus::Pending => {
                server::WebhookDeliveryStatusEnum::Pending
            }
            domain_enum::WebhookDeliveryStatus::InFlight => {
                server::WebhookDeliveryStatusEnum::InFlight
            }
            domain_enum::WebhookDeliveryStatus::Succeeded => {
                server::WebhookDeliveryStatusEnum::Succeeded
            }
            domain_enum::WebhookDeliveryStatus::Failed => server::WebhookDeliveryStatusEnum::Failed,
            domain_enum::WebhookDeliveryStatus::Cancelled => {
                server::WebhookDeliveryStatusEnum::Cancelled
            }
        }
    }

    pub fn delivery_status_from_server(
        value: &server::WebhookDeliveryStatusEnum,
    ) -> domain_enum::WebhookDeliveryStatus {
        match value {
            server::WebhookDeliveryStatusEnum::Pending => {
                domain_enum::WebhookDeliveryStatus::Pending
            }
            server::WebhookDeliveryStatusEnum::InFlight => {
                domain_enum::WebhookDeliveryStatus::InFlight
            }
            server::WebhookDeliveryStatusEnum::Succeeded => {
                domain_enum::WebhookDeliveryStatus::Succeeded
            }
            server::WebhookDeliveryStatusEnum::Failed => domain_enum::WebhookDeliveryStatus::Failed,
            server::WebhookDeliveryStatusEnum::Cancelled => {
                domain_enum::WebhookDeliveryStatus::Cancelled
            }
        }
    }

    pub fn endpoint_to_server(value: &domain::WebhookEndpoint) -> server::WebhookEndpoint {
        server::WebhookEndpoint {
            id: value.id.as_proto(),
            url: value.url.clone(),
            description: value.description.clone(),
            event_types: value.event_types.clone(),
            disabled: value.disabled,
            disabled_reason: value
                .disabled_reason
                .as_ref()
                .map(|r| disabled_reason_to_server(r) as i32),
            created_at: value.created_at.as_proto(),
            updated_at: value.updated_at.as_proto(),
            last_success_at: value.last_success_at.as_proto(),
            last_failure_at: value.last_failure_at.as_proto(),
            consecutive_failures: value.consecutive_failures,
            max_in_flight: value.max_in_flight,
            headers: value.headers.iter().map(header_to_server).collect(),
            needs_setup: value.needs_setup(),
            paused_until: value.paused_until.as_proto(),
            failing_since: value.failing_since.as_proto(),
            rate_limit_per_sec: value.rate_limit_per_sec,
        }
    }

    pub fn header_to_server(value: &domain::WebhookEndpointHeader) -> server::WebhookHeader {
        server::WebhookHeader {
            name: value.name.clone(),
            value: value.value.clone(),
            sensitive: value.sensitive,
            set: value.set,
        }
    }

    pub fn header_from_server(value: server::WebhookHeaderInput) -> domain::WebhookHeaderInput {
        domain::WebhookHeaderInput {
            name: value.name.trim().to_string(),
            value: value.value,
            sensitive: value.sensitive,
        }
    }

    pub fn delivery_to_server(
        value: &domain::WebhookDelivery,
        event_type: String,
        attempts: i64,
    ) -> server::WebhookDelivery {
        server::WebhookDelivery {
            id: value.id.as_proto(),
            message_id: value.message_id.as_proto(),
            endpoint_id: value.endpoint_id.as_proto(),
            event_type,
            status: delivery_status_to_server(&value.status) as i32,
            attempt_count: value.attempt_count,
            next_attempt_at: value.next_attempt_at.as_proto(),
            last_response_status: value.last_response_status.map(i32::from),
            last_error: value.last_error.clone(),
            created_at: value.created_at.as_proto(),
            completed_at: value.completed_at.as_proto(),
            manual: value.manual,
            attempts,
        }
    }

    pub fn attempt_to_server(value: &domain::WebhookAttempt) -> server::WebhookAttempt {
        server::WebhookAttempt {
            id: value.id.as_proto(),
            attempt_number: value.attempt_number,
            attempted_at: value.attempted_at.as_proto(),
            duration_ms: value.duration_ms,
            response_status: value.response_status.map(i32::from),
            response_body: value.response_body.clone(),
            error: value.error.clone(),
            succeeded: value.succeeded,
        }
    }
}

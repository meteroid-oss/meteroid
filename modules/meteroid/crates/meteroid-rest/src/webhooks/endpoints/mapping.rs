use meteroid_store::domain::enums as domain_enums;
use meteroid_store::domain::webhooks_out as domain;

use super::model::*;

pub fn disabled_reason_to_rest(
    reason: domain_enums::WebhookEndpointDisabledReason,
) -> WebhookEndpointDisabledReason {
    match reason {
        domain_enums::WebhookEndpointDisabledReason::Manual => {
            WebhookEndpointDisabledReason::Manual
        }
        domain_enums::WebhookEndpointDisabledReason::AutoFailures => {
            WebhookEndpointDisabledReason::AutoFailures
        }
        domain_enums::WebhookEndpointDisabledReason::Gone => WebhookEndpointDisabledReason::Gone,
    }
}

pub fn delivery_status_to_rest(
    status: domain_enums::WebhookDeliveryStatus,
) -> WebhookDeliveryStatus {
    match status {
        domain_enums::WebhookDeliveryStatus::Pending => WebhookDeliveryStatus::Pending,
        domain_enums::WebhookDeliveryStatus::InFlight => WebhookDeliveryStatus::InFlight,
        domain_enums::WebhookDeliveryStatus::Succeeded => WebhookDeliveryStatus::Succeeded,
        domain_enums::WebhookDeliveryStatus::Failed => WebhookDeliveryStatus::Failed,
        domain_enums::WebhookDeliveryStatus::Cancelled => WebhookDeliveryStatus::Cancelled,
    }
}

pub fn delivery_status_to_domain(
    status: WebhookDeliveryStatus,
) -> domain_enums::WebhookDeliveryStatus {
    match status {
        WebhookDeliveryStatus::Pending => domain_enums::WebhookDeliveryStatus::Pending,
        WebhookDeliveryStatus::InFlight => domain_enums::WebhookDeliveryStatus::InFlight,
        WebhookDeliveryStatus::Succeeded => domain_enums::WebhookDeliveryStatus::Succeeded,
        WebhookDeliveryStatus::Failed => domain_enums::WebhookDeliveryStatus::Failed,
        WebhookDeliveryStatus::Cancelled => domain_enums::WebhookDeliveryStatus::Cancelled,
    }
}

pub fn header_from_rest(header: WebhookHeaderInput) -> domain::WebhookHeaderInput {
    domain::WebhookHeaderInput {
        name: header.name.trim().to_string(),
        value: header.value,
        sensitive: header.sensitive,
    }
}

pub fn endpoint_to_rest(endpoint: domain::WebhookEndpoint) -> WebhookEndpoint {
    let needs_setup = endpoint.needs_setup();
    WebhookEndpoint {
        id: endpoint.id,
        url: endpoint.url,
        description: endpoint.description,
        event_types: endpoint.event_types,
        disabled: endpoint.disabled,
        disabled_reason: endpoint.disabled_reason.map(disabled_reason_to_rest),
        consecutive_failures: endpoint.consecutive_failures,
        max_in_flight: endpoint.max_in_flight,
        headers: endpoint
            .headers
            .into_iter()
            .map(|h| WebhookHeader {
                name: h.name,
                value: h.value,
                sensitive: h.sensitive,
                set: h.set,
            })
            .collect(),
        needs_setup,
        rate_limit_per_sec: endpoint.rate_limit_per_sec,
        paused_until: endpoint.paused_until.map(Into::into),
        created_at: endpoint.created_at.into(),
        updated_at: endpoint.updated_at.map(Into::into),
        last_success_at: endpoint.last_success_at.map(Into::into),
        last_failure_at: endpoint.last_failure_at.map(Into::into),
    }
}

pub fn delivery_to_rest(delivery: domain::WebhookDelivery, event_type: String) -> WebhookDelivery {
    WebhookDelivery {
        id: delivery.id,
        message_id: delivery.message_id,
        endpoint_id: delivery.endpoint_id,
        event_type,
        status: delivery_status_to_rest(delivery.status),
        attempt_count: delivery.attempt_count,
        next_attempt_at: delivery.next_attempt_at.map(Into::into),
        last_response_status: delivery.last_response_status.map(i32::from),
        last_error: delivery.last_error,
        manual: delivery.manual,
        created_at: delivery.created_at.into(),
        completed_at: delivery.completed_at.map(Into::into),
    }
}

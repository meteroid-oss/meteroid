pub mod endpoint {
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use crate::api::webhooksout::error::WebhookApiError;
    use crate::api::webhooksout::mapping::event_type;
    use meteroid_grpc::meteroid::api::webhooks::out::v1::{
        CreateWebhookEndpointRequest, WebhookEndpoint as WebhookEndpointProto,
    };
    use meteroid_store::domain::enums::WebhookOutEventTypeEnum;
    use meteroid_store::domain::webhooks::{WebhookOutEndpoint, WebhookOutEndpointNew};
    use secrecy::ExposeSecret;
    use uuid::Uuid;

    pub fn to_proto(endpoint: WebhookOutEndpoint) -> WebhookEndpointProto {
        WebhookEndpointProto {
            id: endpoint.id.to_string(),
            url: endpoint.url.to_string(),
            description: endpoint.description.clone(),
            secret: endpoint.secret.expose_secret().to_string(),
            events_to_listen: endpoint
                .events_to_listen
                .iter()
                .map(|e| event_type::to_proto(&e).into())
                .collect(),
            enabled: endpoint.enabled,
            created_at: Some(chrono_to_timestamp(endpoint.created_at)),
        }
    }

    pub fn new_req_to_domain(
        tenant_id: Uuid,
        req: CreateWebhookEndpointRequest,
    ) -> Result<WebhookOutEndpointNew, WebhookApiError> {
        let url = url::Url::parse(req.url.as_str())
            .map_err(|e| WebhookApiError::InvalidArgument(format!("Invalid URL: {}", e)))?;

        let events_to_listen: Vec<WebhookOutEventTypeEnum> = req
            .events_to_listen()
            .map(|e| event_type::to_domain(&e))
            .collect();

        Ok(WebhookOutEndpointNew {
            tenant_id,
            url,
            description: req.description,
            events_to_listen,
            enabled: true,
        })
    }
}

pub mod event_type {
    use meteroid_grpc::meteroid::api::webhooks::out::v1::WebhookEventType as WebhookEventTypeProto;
    use meteroid_store::domain::enums::WebhookOutEventTypeEnum;

    pub fn to_domain(event_type: &WebhookEventTypeProto) -> WebhookOutEventTypeEnum {
        match event_type {
            WebhookEventTypeProto::CustomerCreated => WebhookOutEventTypeEnum::CustomerCreated,
            WebhookEventTypeProto::SubscriptionCreated => {
                WebhookOutEventTypeEnum::SubscriptionCreated
            }
            WebhookEventTypeProto::InvoiceCreated => WebhookOutEventTypeEnum::InvoiceCreated,
            WebhookEventTypeProto::InvoiceFinalized => WebhookOutEventTypeEnum::InvoiceFinalized,
        }
    }

    pub fn to_proto(event_type: &WebhookOutEventTypeEnum) -> WebhookEventTypeProto {
        match event_type {
            WebhookOutEventTypeEnum::CustomerCreated => WebhookEventTypeProto::CustomerCreated,
            WebhookOutEventTypeEnum::SubscriptionCreated => {
                WebhookEventTypeProto::SubscriptionCreated
            }
            WebhookOutEventTypeEnum::InvoiceCreated => WebhookEventTypeProto::InvoiceCreated,
            WebhookOutEventTypeEnum::InvoiceFinalized => WebhookEventTypeProto::InvoiceFinalized,
        }
    }
}

pub mod event {
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use crate::api::webhooksout::mapping::event_type;
    use meteroid_grpc::meteroid::api::webhooks::out::v1::WebhookEvent as WebhookEventProto;
    use meteroid_store::domain::webhooks::WebhookOutEvent;

    pub fn to_proto(event: &WebhookOutEvent) -> WebhookEventProto {
        WebhookEventProto {
            id: event.id.to_string(),
            event_type: event_type::to_proto(&event.event_type).into(),
            created_at: Some(chrono_to_timestamp(event.created_at)),
            http_status_code: event.http_status_code.map(|x| x as i32),
            request_body: event.request_body.clone(),
            response_body: event.response_body.clone(),
            error_message: event.error_message.clone(),
        }
    }
}

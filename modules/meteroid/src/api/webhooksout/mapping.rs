pub mod endpoint {
    use crate::api::shared::mapping::datetime::datetime_to_timestamp;
    use crate::api::webhooksout::mapping::event_type;
    use meteroid_grpc::meteroid::api::webhooks::out::v1::WebhookEndpoint as WebhookEndpointProto;
    use meteroid_repository::webhook_out_endpoints::WebhookOutEndpoint as WebhookEndpointDb;
    use secrecy::{ExposeSecret, SecretString};
    use tonic::Status;

    pub fn to_proto(
        endpoint: &WebhookEndpointDb,
        crypt_key: &SecretString,
    ) -> Result<WebhookEndpointProto, Status> {
        let secret = crate::crypt::decrypt(crypt_key, endpoint.secret.as_str())
            .map_err(|x| x.current_context().clone())?;

        let endpoint = WebhookEndpointProto {
            id: endpoint.id.to_string(),
            url: endpoint.url.clone(),
            description: endpoint.description.clone(),
            secret: secret.expose_secret().to_string(),
            events_to_listen: endpoint
                .events_to_listen
                .iter()
                .map(|e| event_type::to_proto(&e).into())
                .collect(),
            enabled: endpoint.enabled,
            created_at: Some(datetime_to_timestamp(endpoint.created_at)),
        };

        Ok(endpoint)
    }
}

pub mod event_type {
    use meteroid_grpc::meteroid::api::webhooks::out::v1::WebhookEventType as WebhookEventTypeProto;
    use meteroid_repository::WebhookOutEventTypeEnum as WebhookEventTypeDb;
    pub fn to_db(event_type: &WebhookEventTypeProto) -> WebhookEventTypeDb {
        match event_type {
            WebhookEventTypeProto::CustomerCreated => WebhookEventTypeDb::CUSTOMER_CREATED,
            WebhookEventTypeProto::SubscriptionCreated => WebhookEventTypeDb::SUBSCRIPTION_CREATED,
            WebhookEventTypeProto::InvoiceCreated => WebhookEventTypeDb::INVOICE_CREATED,
            WebhookEventTypeProto::InvoiceFinalized => WebhookEventTypeDb::INVOICE_FINALIZED,
        }
    }

    pub fn to_proto(event_type: &WebhookEventTypeDb) -> WebhookEventTypeProto {
        match event_type {
            WebhookEventTypeDb::CUSTOMER_CREATED => WebhookEventTypeProto::CustomerCreated,
            WebhookEventTypeDb::SUBSCRIPTION_CREATED => WebhookEventTypeProto::SubscriptionCreated,
            WebhookEventTypeDb::INVOICE_CREATED => WebhookEventTypeProto::InvoiceCreated,
            WebhookEventTypeDb::INVOICE_FINALIZED => WebhookEventTypeProto::InvoiceFinalized,
        }
    }
}

pub mod event {
    use crate::api::shared::mapping::datetime::datetime_to_timestamp;
    use crate::api::webhooksout::mapping::event_type;
    use meteroid_grpc::meteroid::api::webhooks::out::v1::WebhookEvent as WebhookEventProto;
    use meteroid_repository::webhook_out_events::ListWebhookOutEvent as ListWebhookOutEventDb;

    pub fn to_proto(event: &ListWebhookOutEventDb) -> WebhookEventProto {
        WebhookEventProto {
            id: event.id.to_string(),
            event_type: event_type::to_proto(&event.event_type).into(),
            created_at: Some(datetime_to_timestamp(event.created_at)),
            http_status_code: event.http_status_code.map(|x| x as i32),
            request_body: event.request_body.clone(),
            response_body: event.response_body.clone(),
            error_message: event.error_message.clone(),
        }
    }
}

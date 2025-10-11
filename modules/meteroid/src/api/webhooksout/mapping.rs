pub mod endpoint {
    use crate::api::webhooksout::error::WebhookApiError;
    use crate::api::webhooksout::mapping::event_type;
    use common_domain::ids::TenantId;
    use meteroid_grpc::meteroid::api::webhooks::out::v1::{
        CreateWebhookEndpointRequest, ListWebhookEndpointsRequest, ListWebhookEndpointsResponse,
        WebhookEndpoint as WebhookEndpointProto,
        WebhookEndpointListItem as WebhookEndpointListItemProto,
    };
    use meteroid_store::domain::WebhookPage;
    use meteroid_store::domain::enums::WebhookOutEventTypeEnum;
    use meteroid_store::domain::webhooks::{
        WebhookOutEndpoint, WebhookOutEndpointListItem, WebhookOutEndpointNew,
        WebhookOutListEndpointFilter,
    };
    use secrecy::ExposeSecret;

    pub fn to_proto(endpoint: WebhookOutEndpoint) -> WebhookEndpointProto {
        WebhookEndpointProto {
            id: endpoint.id.to_string(),
            url: endpoint.url.to_string(),
            description: endpoint.description.clone(),
            secret: endpoint.secret.expose_secret().to_string(),
            events_to_listen: endpoint
                .events_to_listen
                .iter()
                .map(|e| event_type::to_proto(e).into())
                .collect(),
            disabled: endpoint.disabled,
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
        }
    }

    pub fn list_item_to_proto(
        endpoint: WebhookOutEndpointListItem,
    ) -> WebhookEndpointListItemProto {
        WebhookEndpointListItemProto {
            id: endpoint.id.to_string(),
            url: endpoint.url.to_string(),
            description: endpoint.description.clone(),
            events_to_listen: endpoint
                .events_to_listen
                .iter()
                .map(|e| event_type::to_proto(e).into())
                .collect(),
            disabled: endpoint.disabled,
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
        }
    }

    pub fn page_to_proto(
        page: WebhookPage<WebhookOutEndpointListItem>,
    ) -> ListWebhookEndpointsResponse {
        ListWebhookEndpointsResponse {
            data: page.data.into_iter().map(list_item_to_proto).collect(),
            iterator: page.iterator,
            prev_iterator: page.prev_iterator,
            done: page.done,
        }
    }

    pub fn new_req_to_domain(
        tenant_id: TenantId,
        req: CreateWebhookEndpointRequest,
    ) -> Result<WebhookOutEndpointNew, WebhookApiError> {
        let url = url::Url::parse(req.url.as_str())
            .map_err(|e| WebhookApiError::InvalidArgument(format!("Invalid URL: {e}")))?;

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

    pub fn list_request_to_domain_filter(
        req: ListWebhookEndpointsRequest,
    ) -> WebhookOutListEndpointFilter {
        WebhookOutListEndpointFilter {
            limit: req.limit,
            iterator: req.iterator,
        }
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
            WebhookEventTypeProto::BillableMetricCreated => {
                WebhookOutEventTypeEnum::BillableMetricCreated
            }
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
            WebhookOutEventTypeEnum::BillableMetricCreated => {
                WebhookEventTypeProto::BillableMetricCreated
            }
        }
    }
}

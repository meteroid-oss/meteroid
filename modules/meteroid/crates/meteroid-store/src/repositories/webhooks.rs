use crate::domain::enums::WebhookOutEventTypeEnum;
use crate::domain::webhooks::{
    WebhookInEvent, WebhookInEventNew, WebhookOutCreateMessageResult, WebhookOutEndpoint,
    WebhookOutEndpointListItem, WebhookOutEndpointNew, WebhookOutListEndpointFilter,
    WebhookOutListMessageAttemptFilter, WebhookOutMessageAttempt, WebhookOutMessageNew,
};
use crate::domain::WebhookPage;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use diesel_models::webhooks::WebhookInEventRowNew;
use error_stack::ResultExt;
use strum::IntoEnumIterator;
use svix::api::{EndpointIn, EventTypeIn};
use svix::error::Error;
use tracing::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait WebhooksInterface {
    async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint>;

    async fn get_webhook_out_endpoint(
        &self,
        tenant_id: Uuid,
        endpoint_id: String,
    ) -> StoreResult<WebhookOutEndpoint>;

    async fn list_webhook_out_endpoints(
        &self,
        tenant_id: Uuid,
        filter: Option<WebhookOutListEndpointFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutEndpointListItem>>;

    async fn list_message_attempts_out(
        &self,
        tenant_id: Uuid,
        endpoint_id: String,
        filter: Option<WebhookOutListMessageAttemptFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutMessageAttempt>>;

    async fn insert_webhook_message_out(
        &self,
        tenant_id: Uuid,
        msg: WebhookOutMessageNew,
    ) -> StoreResult<WebhookOutCreateMessageResult>;

    // this will have its own CRUD
    async fn insert_webhook_out_event_types(&self) -> StoreResult<()>;

    async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent>;
}

#[async_trait::async_trait]
impl WebhooksInterface for Store {
    async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint> {
        let app = self.svix_application(endpoint.tenant_id).await?;
        let svix = self.svix()?;

        let created = svix
            .endpoint()
            .create(
                app.id.clone(),
                EndpointIn {
                    channels: None,
                    description: endpoint.description,
                    disabled: None,
                    filter_types: Some(
                        endpoint
                            .events_to_listen
                            .into_iter()
                            .map(|e| e.to_string())
                            .collect(),
                    ),
                    metadata: None,
                    rate_limit: None,
                    secret: None,
                    uid: None,
                    url: endpoint.url.into(),
                    version: None,
                },
                None,
            )
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to create svix endpoint".into(),
            ))?;

        self.get_webhook_out_endpoint(endpoint.tenant_id, created.id.clone())
            .await
    }

    async fn get_webhook_out_endpoint(
        &self,
        tenant_id: Uuid,
        endpoint_id: String,
    ) -> StoreResult<WebhookOutEndpoint> {
        let svix = self.svix()?;

        let endpoint = svix
            .endpoint()
            .get(tenant_id.to_string(), endpoint_id.clone())
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to get svix endpoint".into(),
            ))?;

        let secret = svix
            .endpoint()
            .get_secret(tenant_id.to_string(), endpoint.id.clone())
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to get svix endpoint secret".into(),
            ))?;

        Ok(WebhookOutEndpoint {
            description: Some(endpoint.description),
            id: endpoint.id,
            url: endpoint.url,
            secret: secret.key.into(),
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
            events_to_listen: WebhookOutEventTypeEnum::from_svix_channels(&endpoint.channels)?,
            disabled: endpoint.disabled.unwrap_or(false),
        })
    }

    async fn list_webhook_out_endpoints(
        &self,
        tenant_id: Uuid,
        filter: Option<WebhookOutListEndpointFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutEndpointListItem>> {
        // move it to some init_webhook_for_tenant place
        self.svix_application(tenant_id).await?;

        let svix = self.svix()?;

        svix.endpoint()
            .list(tenant_id.to_string(), filter.map(Into::into))
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to list svix endpoints".into(),
            ))
            .and_then(TryInto::try_into)
    }

    async fn list_message_attempts_out(
        &self,
        tenant_id: Uuid,
        endpoint_id: String,
        filter: Option<WebhookOutListMessageAttemptFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutMessageAttempt>> {
        let svix = self.svix()?;

        // note: in os version svix doesn't return the message inside the attempt
        svix.message_attempt()
            .list_by_endpoint(tenant_id.to_string(), endpoint_id, filter.map(Into::into))
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to list svix message attempts".into(),
            ))
            .map(Into::into)
    }

    async fn insert_webhook_message_out(
        &self,
        tenant_id: Uuid,
        msg: WebhookOutMessageNew,
    ) -> StoreResult<WebhookOutCreateMessageResult> {
        if let Some(svix_api) = &self.svix {
            let message_result = svix_api
                .message()
                .create(tenant_id.to_string(), msg.into(), None)
                .await;

            if let Err(Error::Http(ref e)) = message_result {
                if e.status.as_u16() == 409 {
                    return Ok(WebhookOutCreateMessageResult::Conflict);
                }
                if e.status.as_u16() == 404 {
                    return Ok(WebhookOutCreateMessageResult::NotFound);
                }
            }

            message_result
                .map(|res| WebhookOutCreateMessageResult::Created(res.into()))
                .change_context(StoreError::WebhookServiceError(
                    "Failed to send svix message".into(),
                ))
        } else {
            Ok(WebhookOutCreateMessageResult::SvixNotConfigured)
        }
    }

    /// naive hack, will be replaced with a proper CRUD for event types
    async fn insert_webhook_out_event_types(&self) -> StoreResult<()> {
        if let Some(svix_api) = &self.svix {
            for event_type in WebhookOutEventTypeEnum::iter() {
                let created = svix_api
                    .event_type()
                    .create(
                        EventTypeIn {
                            archived: None,
                            deprecated: None,
                            description: event_type.to_string(),
                            feature_flag: None,
                            group_name: Some(event_type.group()),
                            name: event_type.to_string(),
                            schemas: None,
                        },
                        None,
                    )
                    .await;

                if let Err(Error::Http(ref e)) = created {
                    if e.status.as_u16() == 409 {
                        log::info!(
                            "Webhook event type {} already exists",
                            event_type.to_string()
                        );

                        continue;
                    }
                }

                log::info!("Webhook event type {} created", event_type.to_string());

                created.change_context(StoreError::WebhookServiceError(
                    "Failed to create svix event type".into(),
                ))?;
            }
        } else {
            log::warn!("Svix disabled!");
        }
        Ok(())
    }

    async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent> {
        let mut conn = self.get_conn().await?;

        let insertable: WebhookInEventRowNew = event.into();

        insertable
            .insert(&mut conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}

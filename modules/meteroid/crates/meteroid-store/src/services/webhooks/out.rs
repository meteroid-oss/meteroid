use crate::domain::WebhookPage;
use crate::domain::enums::WebhookOutEventTypeEnum;
use crate::domain::webhooks::{
    WebhookInEvent, WebhookInEventNew, WebhookOutCreateMessageResult, WebhookOutEndpoint,
    WebhookOutEndpointListItem, WebhookOutEndpointNew, WebhookOutListEndpointFilter,
    WebhookOutListMessageAttemptFilter, WebhookOutMessageAttempt, WebhookOutMessageNew,
    WebhookPortalAccess,
};
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use crate::{Store, StoreResult};
use backon::{ConstantBuilder, Retryable};
use cached::proc_macro::cached;
use common_domain::ids::TenantId;
use diesel_models::organizations::OrganizationRow;
use diesel_models::tenants::TenantRow;
use diesel_models::webhooks::WebhookInEventRowNew;
use error_stack::{Report, ResultExt};
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter, clock};
use itertools::Itertools;
use nonzero_ext::nonzero;
use std::time::Duration;
use strum::IntoEnumIterator;
use svix::api::{AppPortalAccessIn, ApplicationIn, EndpointIn, EventTypeIn, MessageIn};
use svix::error::Error;
use tracing::log;

static API_RATE_LIMITER: std::sync::OnceLock<
    RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware>,
> = std::sync::OnceLock::new();

struct ApiRateLimiter;

impl ApiRateLimiter {
    pub fn get()
    -> &'static RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware> {
        API_RATE_LIMITER.get_or_init(|| RateLimiter::direct(Quota::per_second(nonzero!(50u32))))
    }
}

#[allow(deprecated)]
impl ServicesEdge {
    pub async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint> {
        let svix = self.services.svix()?;

        let created = svix
            .endpoint()
            .create(
                endpoint.tenant_id.to_string(),
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
                    headers: None,
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

    pub async fn get_webhook_out_endpoint(
        &self,
        tenant_id: TenantId,
        endpoint_id: String,
    ) -> StoreResult<WebhookOutEndpoint> {
        let svix = self.services.svix()?;

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

        let events_to_listen = WebhookOutEventTypeEnum::from_svix_endpoint(&endpoint)?;

        Ok(WebhookOutEndpoint {
            description: Some(endpoint.description),
            id: endpoint.id,
            url: endpoint.url,
            secret: secret.key.into(),
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
            events_to_listen,
            disabled: endpoint.disabled.unwrap_or(false),
        })
    }

    pub async fn list_webhook_out_endpoints(
        &self,
        tenant_id: TenantId,
        filter: Option<WebhookOutListEndpointFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutEndpointListItem>> {
        let svix = self.services.svix()?;

        let result = svix
            .endpoint()
            .list(tenant_id.to_string(), filter.map(Into::into))
            .await;

        if let Err(Error::Http(ref e)) = result {
            if e.status.as_u16() == 404 {
                return Ok(WebhookPage {
                    data: vec![],
                    done: true,
                    iterator: None,
                    prev_iterator: None,
                });
            }
        }

        result
            .change_context(StoreError::WebhookServiceError(
                "Failed to list svix endpoints".into(),
            ))
            .and_then(TryInto::try_into)
    }

    async fn list_message_attempts_out(
        &self,
        tenant_id: TenantId,
        endpoint_id: String,
        filter: Option<WebhookOutListMessageAttemptFilter>,
    ) -> StoreResult<WebhookPage<WebhookOutMessageAttempt>> {
        let svix = self.services.svix()?;

        // note: in os version svix doesn't return the message inside the attempt
        svix.message_attempt()
            .list_by_endpoint(tenant_id.to_string(), endpoint_id, filter.map(Into::into))
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to list svix message attempts".into(),
            ))
            .map(Into::into)
    }

    pub async fn insert_webhook_message_out(
        &self,
        tenant_id: TenantId,
        msg: WebhookOutMessageNew,
    ) -> StoreResult<WebhookOutCreateMessageResult> {
        if let Some(svix_api) = &self.services.svix {
            let types = get_endpoint_events_to_listen_cached(&self, tenant_id).await?;

            if !types.contains(&msg.event_type) {
                return Ok(WebhookOutCreateMessageResult::NotFound);
            }

            ApiRateLimiter::get()
                .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
                .await;

            let message_in: MessageIn = msg.try_into()?;

            let message_result = (|| async {
                svix_api
                    .message()
                    .create(tenant_id.to_string(), message_in.clone(), None)
                    .await
            })
                .retry(ConstantBuilder::default().with_jitter())
                .when(|err| matches!(err, Error::Http(e) if e.status.as_u16() == 429 || e.status.as_u16() >= 500))
                .notify(|err: &Error, dur: Duration| {
                    log::warn!("Retrying svix api error {:?} after {:?}", err, dur);
                })
                .await;

            if let Err(Error::Http(ref e)) = message_result {
                match e.status.as_u16() {
                    404 => return Ok(WebhookOutCreateMessageResult::NotFound),
                    409 => return Ok(WebhookOutCreateMessageResult::Conflict),
                    _ => (),
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
    pub async fn insert_webhook_out_event_types(&self) -> StoreResult<()> {
        if let Some(svix_api) = &self.services.svix {
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
                        log::info!("Webhook event type {} already exists", event_type);

                        continue;
                    }
                }

                log::info!("Webhook event type {} created", event_type);

                created.change_context(StoreError::WebhookServiceError(
                    "Failed to create svix event type".into(),
                ))?;
            }
        } else {
            log::warn!("Svix disabled!");
        }
        Ok(())
    }

    pub async fn get_webhook_portal_access(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<WebhookPortalAccess> {
        let svix = self.services.svix()?;

        let app_in = svix_application_in(&self.store, tenant_id).await?;

        let access_in = AppPortalAccessIn {
            application: Some(app_in),
            expiry: None, // 7 days by default
            feature_flags: None,
            read_only: None,
        };

        svix.authentication()
            .app_portal_access(tenant_id.to_string(), access_in, None)
            .await
            .map(Into::into)
            .change_context(StoreError::WebhookServiceError(
                "Failed to get webhook portal access".into(),
            ))
    }

    pub async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent> {
        let mut conn = self.services.store.get_conn().await?;

        let insertable: WebhookInEventRowNew = event.into();

        insertable
            .insert(&mut conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}

#[cached(
    result = true,
    size = 100,
    time = 60, // 1m
    key = "TenantId",
    convert = r#"{ tenant_id }"#
)]
async fn get_endpoint_events_to_listen_cached(
    services: &ServicesEdge,
    tenant_id: TenantId,
) -> StoreResult<Vec<WebhookOutEventTypeEnum>> {
    let endpoints = services
        .list_webhook_out_endpoints(
            tenant_id,
            Some(WebhookOutListEndpointFilter {
                limit: Some(250), // svix allows max of 50 endpoints per application, 250 is their max for api request
                iterator: None,
            }),
        )
        .await?
        .data;

    Ok(endpoints
        .into_iter()
        .filter(|x| !x.disabled)
        .flat_map(|x| x.events_to_listen)
        .unique()
        .collect::<Vec<_>>())
}

/// todo optimize
async fn svix_application_in(store: &Store, tenant_id: TenantId) -> StoreResult<ApplicationIn> {
    let mut conn = store.get_conn().await?;

    let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let org = OrganizationRow::get_by_id(&mut conn, tenant.organization_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let app_name = format!("{} | {}", org.trade_name, tenant.name);

    Ok(ApplicationIn {
        metadata: None,
        name: app_name,
        rate_limit: None,
        uid: Some(tenant_id.to_string()),
    })
}

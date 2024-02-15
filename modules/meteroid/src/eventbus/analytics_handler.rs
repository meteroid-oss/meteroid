use secrecy::{ExposeSecret, SecretString};
use segment::message::{Track, User};
use segment::{Client, Message};
use serde_json::Value;
use uuid::Uuid;

use common_build_info::BuildInfo;
use common_config::analytics::AnalyticsConfig;
use common_repository::Pool;

use crate::eventbus::{
    Event, EventBusError, EventData, EventDataDetails, EventHandler, TenantEventDataDetails,
};

pub struct AnalyticsHandler {
    pool: Pool,
    client: segment::HttpClient,
    api_key: SecretString,
    context: Value,
}

impl AnalyticsHandler {
    pub fn new(config: AnalyticsConfig, pool: Pool) -> Self {
        let build_info = BuildInfo::get();

        // https://segment.com/docs/connections/spec/common/#context
        let context = serde_json::json!({
            "version": build_info.version,
            "profile": build_info.profile,
            "os": {
                "family": build_info.target_family,
                "name": build_info.target_os,
                "arch": build_info.target_arch,
            },
            "git_info": build_info.git_info,
        });

        AnalyticsHandler {
            pool,
            client: segment::HttpClient::default(),
            api_key: config.api_key,
            context,
        }
    }

    async fn get_db_connection(&self) -> Result<deadpool_postgres::Object, EventBusError> {
        self.pool
            .get()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))
    }

    fn actor_as_user(actor: Option<Uuid>) -> User {
        if let Some(ref actor) = actor {
            User::UserId {
                user_id: actor.clone().as_hyphenated().to_string(),
            }
        } else {
            User::AnonymousId {
                anonymous_id: "unknown".to_string(),
            }
        }
    }

    async fn send_track(&self, event_name: String, actor: Option<Uuid>, properties: Value) {
        let result = self
            .client
            .send(
                self.api_key.expose_secret().to_string(),
                Message::from(Track {
                    user: Self::actor_as_user(actor),
                    event: event_name,
                    properties,
                    context: Some(self.context.clone()),
                    ..Default::default()
                }),
            )
            .await;

        if let Err(err) = result {
            log::error!("Error sending event to segment. {:?}", err);
        }
    }

    #[tracing::instrument(skip_all)]
    async fn api_token_created(
        &self,
        event: &Event,
        event_data_details: &EventDataDetails,
    ) -> Result<(), EventBusError> {
        let conn = self.get_db_connection().await?;

        let api_token = meteroid_repository::api_tokens::get_api_token_by_id()
            .bind(&conn, &event_data_details.entity_id)
            .one()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "api-token-created".to_string(),
            event.actor,
            serde_json::json!({
                "api_token_id": event_data_details.entity_id,
                "tenant_id": api_token.tenant_id,
            }),
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn billable_metric_created(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "billable-metric-created".to_string(),
            event.actor,
            serde_json::json!({
                "billable_metric_id": event_data_details.entity_id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn customer_created(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let conn = self.get_db_connection().await?;

        let customer = meteroid_repository::customers::get_customer_by_id()
            .bind(&conn, &event_data_details.entity_id)
            .one()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "customer-created".to_string(),
            event.actor,
            serde_json::json!({
                "customer_id": customer.id,
            }),
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn subscription_created(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let conn = self.get_db_connection().await?;

        let subscription = meteroid_repository::subscriptions::subscription_by_id()
            .bind(
                &conn,
                &event_data_details.entity_id,
                &event_data_details.tenant_id,
            )
            .one()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "subscription-created".to_string(),
            event.actor,
            serde_json::json!({
                "subscription_id": subscription.subscription_id,
                "tenant_id": subscription.tenant_id,
                "customer_id": subscription.customer_id,
                "currency": subscription.currency,
                "version": subscription.version,
            }),
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn invoice_draft(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let conn = self.get_db_connection().await?;

        let invoice = meteroid_repository::invoices::get_tenant_invoice_by_id()
            .bind(
                &conn,
                &event_data_details.entity_id,
                &event_data_details.tenant_id,
            )
            .one()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "invoice-draft".to_string(),
            event.actor,
            serde_json::json!({
                "invoice_id": invoice.id,
                "customer_id": invoice.customer_id,
                "subscription_id": invoice.subscription_id,
                "currency": invoice.currency,
            }),
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn invoice_finalized(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let conn = self.get_db_connection().await?;

        let invoice = meteroid_repository::invoices::get_tenant_invoice_by_id()
            .bind(
                &conn,
                &event_data_details.entity_id,
                &event_data_details.tenant_id,
            )
            .one()
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "invoice-finalized".to_string(),
            event.actor,
            serde_json::json!({
                "invoice_id": invoice.id,
                "customer_id": invoice.customer_id,
                "subscription_id": invoice.subscription_id,
                "currency": invoice.currency,
            }),
        )
        .await;

        Ok(())
    }
}

#[async_trait::async_trait]
impl EventHandler<Event> for AnalyticsHandler {
    #[tracing::instrument(skip_all)]
    async fn handle(&self, event: Event) -> Result<(), EventBusError> {
        log::debug!("Handling event: {:?}", event);

        match &event.event_data {
            EventData::ApiTokenCreated(details) => self.api_token_created(&event, details).await?,
            EventData::BillableMetricCreated(details) => {
                self.billable_metric_created(&event, details).await?
            }
            EventData::CustomerCreated(details) => self.customer_created(&event, details).await?,
            EventData::SubscriptionCreated(details) => {
                self.subscription_created(&event, details).await?
            }
            EventData::InvoiceCreated(details) => self.invoice_draft(&event, details).await?,
            EventData::InvoiceFinalized(details) => self.invoice_finalized(&event, details).await?,
            _ => {
                log::debug!("Skipping event: {:?}", &event);
                return Ok(());
            }
        };

        Ok(())
    }
}

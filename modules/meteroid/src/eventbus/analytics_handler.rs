use chrono::Datelike;
use opentelemetry::propagation::Injector;
use secrecy::{ExposeSecret, SecretString};
use segment::message::{Track, User};
use segment::{Client, Message};
use serde_json::Value;
use uuid::Uuid;

use common_build_info::BuildInfo;
use common_config::analytics::AnalyticsConfig;
use common_eventbus::{Event, EventData, EventDataDetails, EventDataWithMetadataDetails, TenantEventDataDetails};
use common_eventbus::{EventBusError, EventHandler};
use common_logging::unwrapper::UnwrapLogger;
use meteroid_store::domain::DetailedInvoice;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_store::repositories::{
    CustomersInterface, InvoiceInterface, PlansInterface, SubscriptionInterface,
};
use meteroid_store::Store;
use crate::constants::OSS_API;

pub struct AnalyticsHandler {
    store: Store,
    client: segment::HttpClient,
    api_key: SecretString,
    context: Value,
}

impl AnalyticsHandler {
    pub fn new(config: AnalyticsConfig, store: Store, country: Option<String>) -> Self {
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
            "country": country.unwrap_or_else(|| "unknown".to_string()),
        });

        AnalyticsHandler {
            store,
            client: segment::HttpClient::default(),
            api_key: config.api_key,
            context,
        }
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
        self.client
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
            .await
            .unwrap_to_log_warn(|err| format!("Error sending event to segment. {:?}", err))
    }

    #[tracing::instrument(skip_all)]
    async fn api_token_created(
        &self,
        event: &Event,
        event_data_details: &EventDataDetails,
    ) -> Result<(), EventBusError> {
        let api_token = self
            .store
            .get_api_token_by_id(&event_data_details.entity_id)
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
        let billable_metric = self
            .store
            .find_billable_metric_by_id(event_data_details.entity_id, event_data_details.tenant_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "billable-metric-created".to_string(),
            event.actor,
            serde_json::json!({
                "billable_metric_id": event_data_details.entity_id,
                "tenant_id": event_data_details.tenant_id,
                "aggregation_type": crate::api::billablemetrics::mapping::aggregation_type::domain_to_server(billable_metric.aggregation_type).as_str_name()
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
        let customer = self
            .store
            .find_customer_by_id(event_data_details.entity_id, event_data_details.tenant_id)
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
    async fn customer_patched(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let customer = self
            .store
            .find_customer_by_id(event_data_details.entity_id, event_data_details.tenant_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "customer-patched".to_string(),
            event.actor,
            serde_json::json!({
                "customer_id": customer.id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn instance_inited(
        &self,
        event: &Event,
        event_data_details: &EventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "instance-inited".to_string(),
            event.actor,
            serde_json::json!({
                "organization_id": event_data_details.entity_id,
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
        let DetailedInvoice {
            invoice, customer, ..
        } = self
            .store
            .find_invoice_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "invoice-draft".to_string(),
            event.actor,
            serde_json::json!({
                "invoice_id": invoice.id,
                "customer_id": customer.id,
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
        let DetailedInvoice {
            invoice, customer, ..
        } = self
            .store
            .find_invoice_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "invoice-finalized".to_string(),
            event.actor,
            serde_json::json!({
                "invoice_id": invoice.id,
                "customer_id": customer.id,
                "subscription_id": invoice.subscription_id,
                "currency": invoice.currency,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn plan_created_draft(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let plan_version = self
            .store
            .get_plan_version_by_id(event_data_details.entity_id, event_data_details.tenant_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "plan-created-draft".to_string(),
            event.actor,
            serde_json::json!({
                "plan_version_id": plan_version.id,
                "plan_id": plan_version.plan_id,
                "version": plan_version.version,
                "tenant_id": plan_version.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn plan_published_version(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let plan_version = self
            .store
            .get_plan_version_by_id(event_data_details.entity_id, event_data_details.tenant_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "plan-published-version".to_string(),
            event.actor,
            serde_json::json!({
                "plan_version_id": plan_version.id,
                "plan_id": plan_version.plan_id,
                "version": plan_version.version,
                "tenant_id": plan_version.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn plan_discarded_version(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "plan-discarded-version".to_string(),
            event.actor,
            serde_json::json!({
                "plan_version_id": event_data_details.entity_id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn price_component_created(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let price_component = self
            .store
            .get_price_component_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "price-component-created".to_string(),
            event.actor,
            serde_json::json!({
                "price_component_id": price_component.id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn price_component_edited(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let price_component = self
            .store
            .get_price_component_by_id(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "price-component-edited".to_string(),
            event.actor,
            serde_json::json!({
                "price_component_id": price_component.id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn price_component_removed(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "price-component-removed".to_string(),
            event.actor,
            serde_json::json!({
                "price_component_id": event_data_details.entity_id,
                "tenant_id": event_data_details.tenant_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn product_family_created(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "product-family-created".to_string(),
            event.actor,
            serde_json::json!({
                "product_family_id": event_data_details.entity_id,
                "tenant_id": event_data_details.tenant_id,
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
        let subscription = self
            .store
            .get_subscription_details(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        self.send_track(
            "subscription-created".to_string(),
            event.actor,
            serde_json::json!({
                "subscription_id": subscription.id,
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
    async fn subscription_canceled(
        &self,
        event: &Event,
        event_data_details: &TenantEventDataDetails,
    ) -> Result<(), EventBusError> {
        let subscription = self
            .store
            .get_subscription_details(event_data_details.tenant_id, event_data_details.entity_id)
            .await
            .map_err(|e| EventBusError::EventHandlerFailed(e.to_string()))?;

        let canceled_at = subscription
            .canceled_at
            .map(|canceled_at| {
                format!(
                    "{}-{}-{}",
                    canceled_at.year(),
                    canceled_at.month(),
                    canceled_at.day()
                )
            })
            .unwrap_or("unknown".to_string());

        let billing_end_date = subscription
            .billing_end_date
            .map(|canceled_at| {
                format!(
                    "{}-{}-{}",
                    canceled_at.year(),
                    canceled_at.month(),
                    canceled_at.day()
                )
            })
            .unwrap_or("unknown".to_string());

        self.send_track(
            "subscription-canceled".to_string(),
            event.actor,
            serde_json::json!({
                "subscription_id": subscription.id,
                "tenant_id": subscription.tenant_id,
                "customer_id": subscription.customer_id,
                "currency": subscription.currency,
                "version": subscription.version,
                "canceled_at": canceled_at,
                "billing_end_date": billing_end_date,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn user_created(
        &self,
        event: &Event,
        event_data_details: &EventDataDetails,
    ) -> Result<(), EventBusError> {
        self.send_track(
            "user-created".to_string(),
            event.actor,
            serde_json::json!({
                "user_id": event_data_details.entity_id,
            }),
        )
            .await;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn user_updated(
        &self,
        event: &Event,
        event_data_details: &EventDataWithMetadataDetails,
    ) -> Result<(), EventBusError> {
        let mut properties = event_data_details.metadata_as_hashmap();
        properties.set("user_id", event_data_details.entity_id.to_string());

        self.send_track(
            "user-updated".to_string(),
            event.actor,
            serde_json::json!(properties),
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
            EventData::CustomerPatched(details) => self.customer_patched(&event, details).await?,
            EventData::OrganizationCreated(details) => self.instance_inited(&event, details).await?,
            EventData::InvoiceCreated(details) => self.invoice_draft(&event, details).await?,
            EventData::InvoiceFinalized(details) => self.invoice_finalized(&event, details).await?,
            EventData::PlanCreatedDraft(details) => {
                self.plan_created_draft(&event, details).await?
            }
            EventData::PlanPublishedVersion(details) => {
                self.plan_published_version(&event, details).await?
            }
            EventData::PlanDiscardedVersion(details) => {
                self.plan_discarded_version(&event, details).await?
            }
            EventData::PriceComponentCreated(details) => {
                self.price_component_created(&event, details).await?
            }
            EventData::PriceComponentEdited(details) => {
                self.price_component_edited(&event, details).await?
            }
            EventData::PriceComponentRemoved(details) => {
                self.price_component_removed(&event, details).await?
            }
            EventData::ProductFamilyCreated(details) => {
                self.product_family_created(&event, details).await?
            }
            EventData::SubscriptionCreated(details) => {
                self.subscription_created(&event, details).await?
            }
            EventData::SubscriptionCanceled(details) => {
                self.subscription_canceled(&event, details).await?
            }
            EventData::UserCreated(details) => self.user_created(&event, details).await?,
            _ => {
                log::debug!("Skipping event: {:?}", &event);
                return Ok(());
            }
        };

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct GeoIp {
    pub country: String,
}

pub async fn get_geoip() -> Result<GeoIp, String> {
    let response = reqwest::Client::new()
        .get(format!("{}/geoip", OSS_API))
        .send()
        .await
        .map_err(|e| e.to_string());

    match response {
        Ok(response) => response.json::<GeoIp>().await.map_err(|e| e.to_string()),
        Err(e) => Err(e),
    }
}

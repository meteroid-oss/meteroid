//! Demo tenants seeded with sample data.

use std::sync::Arc;

use common_domain::ids::OrganizationId;
use meteroid_grpc::meteroid::api::tenants::v1::SandboxPreset;
use meteroid_seeder::{presets, warmup};
use meteroid_store::domain::Tenant;
use meteroid_store::domain::enums::EmailDeliveryMode;
use meteroid_store::hooks::AnalyticsHook;
use meteroid_store::{Services, Store, StoreResult};

use crate::ApiContext;

#[async_trait::async_trait]
pub trait Sandboxes: Send + Sync {
    /// The demo tenant a new organization starts with, when the edition gives it one.
    async fn provision_for_signup(
        &self,
        _organization_id: OrganizationId,
    ) -> StoreResult<Option<Tenant>> {
        Ok(None)
    }

    async fn create(
        &self,
        organization_id: OrganizationId,
        name: String,
        email_delivery_mode: Option<EmailDeliveryMode>,
        preset: SandboxPreset,
    ) -> StoreResult<Tenant>;
}

/// Seeds a preset, then finalizes it once its history is billed so the data reads as live.
#[derive(Clone)]
pub struct SeededSandboxes {
    store: Arc<Store>,
    services: Arc<Services>,
    analytics: Arc<dyn AnalyticsHook>,
}

impl SeededSandboxes {
    pub fn new(ctx: &ApiContext) -> Self {
        Self {
            store: Arc::new(ctx.store.clone()),
            services: Arc::new(ctx.services.clone()),
            analytics: ctx.analytics.clone(),
        }
    }
}

#[async_trait::async_trait]
impl Sandboxes for SeededSandboxes {
    async fn create(
        &self,
        organization_id: OrganizationId,
        name: String,
        email_delivery_mode: Option<EmailDeliveryMode>,
        preset: SandboxPreset,
    ) -> StoreResult<Tenant> {
        let scenario = match preset {
            SandboxPreset::Default => presets::default_scenario(),
            SandboxPreset::AiCredits => presets::ai_credits_scenario(),
        };
        let tenant = presets::run_preset(
            &self.store,
            &self.services,
            scenario,
            organization_id,
            Some(name),
            email_delivery_mode,
            self.analytics.clone(),
        )
        .await?;
        warmup::spawn_sandbox_warmup(
            self.store.clone(),
            self.services.clone(),
            self.analytics.clone(),
            tenant.id,
        );
        Ok(tenant)
    }
}

//! The module traits and an edition's module list: what a module contributes to the APIs and the
//! workers, and the contexts it receives.

use std::sync::Arc;

pub mod eventbus;
pub mod process;
mod sandboxes;

pub use sandboxes::{Sandboxes, SeededSandboxes};

/// The edition's environment file, when there is one.
pub fn load_dotenv() -> Result<(), dotenvy::Error> {
    match dotenvy::dotenv() {
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }
}

use common_domain::ids::{CustomerId, LicenseId, TenantId};
use common_eventbus::{Event, EventHandler};
use meteroid_grpc::meteroid::portal::customer::v1::{
    GetLicenseActivationCodeResponse, ListLicensesResponse,
};
use meteroid_middleware::server::auth::SharedSupportGrantReader;
pub use meteroid_middleware::server::auth::rbac::{RouteAuth, RoutePolicy};
pub use meteroid_rest_common::spec::EditionSpec;
use meteroid_store::clients::pending_usage::PendingUsageSource;
use meteroid_store::hooks::{
    AnalyticsHook, CreditControlHooks, InstanceEdition, MemberHooks, OrganizationHooks,
    SignalHooks, StoreHooks, core_instance, no_analytics_hook, no_organization_hooks,
};
use meteroid_store::repositories::revenue_stats::{RevenueStats, postgres_revenue_stats};
use meteroid_store::{Services, Store};
use rate_limiter::{RateLimitBackend, RateLimitConfig, RateLimiter};
use tokio::task::JoinSet;
use utoipa_axum::router::OpenApiRouter;

use meteroid_app::config::Config;
use meteroid_app::features::{Entitlements, FeatureGate, OssEntitlements};
use meteroid_app::services::egress::{Egress, EgressGuard};
use meteroid_rest_common::delegated_auth::RestDelegatedAuth;

pub struct ApiContext {
    pub config: Config,
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
    /// The gates the edition's modules declare.
    pub feature_gates: Arc<[FeatureGate]>,
    pub rate_limiter: RateLimiter,
    /// Set in the gRPC API only.
    pub redis: Option<fred::prelude::Client>,
    pub analytics: Arc<dyn AnalyticsHook>,
    pub egress: Egress,
}

/// The licenses a customer holds, as the customer portal shows them.
#[async_trait::async_trait]
pub trait PortalLicenses: Send + Sync {
    async fn list(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<ListLicensesResponse, tonic::Status>;

    async fn activation_code(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        license_id: LicenseId,
    ) -> Result<GetLicenseActivationCodeResponse, tonic::Status>;
}

/// A module's routes, with their state already provided (see [`ModuleRestState`]).
#[derive(Default)]
pub struct RestRoutes {
    /// Part of the public REST API: authenticated and published in the edition's OpenAPI spec.
    pub api: OpenApiRouter,
    /// Part of the client API (`/api/client/v1`): authenticated with a customer portal token and
    /// published in the edition's client spec.
    pub client: OpenApiRouter,
    /// Outside the published API, e.g. protocol endpoints called by other installations.
    pub other: axum::Router,
}

/// What module REST handlers usually read; a module may use a state of its own instead.
#[derive(Clone)]
pub struct ModuleRestState {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
    /// `REST_API_EXTERNAL_URL`, without a trailing slash.
    pub api_url: String,
}

impl ModuleRestState {
    pub fn new(ctx: &ApiContext) -> Self {
        Self {
            store: ctx.store.clone(),
            services: ctx.services.clone(),
            feature_resolver: ctx.feature_resolver.clone(),
            api_url: ctx
                .config
                .rest_api_external_url
                .trim_end_matches('/')
                .to_string(),
        }
    }
}

#[async_trait::async_trait]
pub trait ApiModule: Send + Sync {
    fn id(&self) -> &'static str;

    /// The gates of this module's features, including those core checks on its behalf.
    fn feature_gates(&self) -> &'static [FeatureGate] {
        &[]
    }

    fn grpc(&self, _ctx: &ApiContext, _routes: &mut tonic::service::RoutesBuilder) {}

    async fn rest(&self, _ctx: &ApiContext) -> RestRoutes {
        RestRoutes::default()
    }

    /// The operations `rest` publishes, for generating the spec without building any state.
    fn openapi(&self) -> Option<utoipa::openapi::OpenApi> {
        None
    }

    /// The client API operations `rest` publishes.
    fn client_openapi(&self) -> Option<utoipa::openapi::OpenApi> {
        None
    }

    /// The webhook events this module emits among those core documents but publishes only when
    /// a module emits them (`WebhookOutEventTypeEnum::needs_module`), by name.
    fn webhook_events(&self) -> &'static [&'static str] {
        &[]
    }

    /// The access callers need on the gRPC services whose protos this module owns; core
    /// classifies every service of a core proto.
    fn route_policies(&self) -> &'static [RoutePolicy] {
        &[]
    }

    /// Lets platform admins with a live grant into organizations they are not members of.
    fn support_grants(&self, _ctx: &ApiContext) -> Option<SharedSupportGrantReader> {
        None
    }

    /// Replaces core's [`SeededSandboxes`], which gives a new organization no sandbox.
    fn sandboxes(&self, _ctx: &ApiContext) -> Option<Arc<dyn Sandboxes>> {
        None
    }

    fn rest_auth(&self, _ctx: &ApiContext) -> Option<Arc<dyn RestDelegatedAuth>> {
        None
    }

    fn portal_licenses(&self, _ctx: &ApiContext) -> Option<Arc<dyn PortalLicenses>> {
        None
    }

    /// Built with the store, before any context exists.
    fn signal_hooks(&self) -> Option<Arc<dyn SignalHooks>> {
        None
    }

    fn member_hooks(&self) -> Option<Arc<dyn MemberHooks>> {
        None
    }

    /// The live usage controls; without them, core never watches usage live.
    fn credit_control_hooks(&self) -> Option<Arc<dyn CreditControlHooks>> {
        None
    }

    /// Serves the home dashboard's MRR and revenue figures in place of Postgres.
    fn revenue_stats(&self, _ctx: &ApiContext) -> Option<Arc<dyn RevenueStats>> {
        None
    }

    /// Replaces core's hook, which ignores history written after the fact.
    fn analytics_hook(
        &self,
        _store: &Store,
        _config: &Config,
        _egress: &Egress,
    ) -> Option<Arc<dyn AnalyticsHook>> {
        None
    }

    /// Where the API's rate limits are counted, given the process's Redis; without a backend
    /// every request is allowed.
    async fn rate_limit_backend(
        &self,
        _redis: &fred::prelude::Client,
    ) -> Option<Arc<dyn RateLimitBackend>> {
        None
    }

    /// Restricts the destinations of outbound HTTP; without one, any destination is reachable.
    /// The process builds every outbound client with it, through its [`Egress`].
    fn destination_guard(&self) -> Option<Arc<dyn EgressGuard>> {
        None
    }

    /// Once per process (API, workers or standalone), after migrations ran.
    async fn on_startup(&self, _store: &Store, _config: &Config) {}

    /// Subscribed to the process's event bus, in the API and the workers.
    fn event_handlers(
        &self,
        _store: &Store,
        _config: &Config,
    ) -> Vec<Arc<dyn EventHandler<Event>>> {
        Vec::new()
    }
}

pub struct WorkerContext {
    pub config: Config,
    pub store: Arc<Store>,
    pub services: Arc<Services>,
    pub feature_resolver: Arc<dyn Entitlements>,
    pub analytics: Arc<dyn AnalyticsHook>,
    /// The metering reads that price usage not invoiced yet.
    pub pending_usage: Arc<dyn PendingUsageSource>,
    pub egress: Egress,
}

pub trait WorkerModule: Send + Sync {
    fn id(&self) -> &'static str;

    fn spawn(&self, ctx: &WorkerContext, join_set: &mut JoinSet<()>);

    /// Queues bound to a topic in SQL that this module consumes; core drains the others.
    fn consumed_queues(&self) -> &'static [meteroid_store::domain::pgmq::PgmqQueue] {
        &[]
    }
}

/// What an edition decides beyond its modules: how entitlements resolve, what the installation
/// is, and what happens around organizations. Built with the edition's settings.
pub trait Edition: Send + Sync {
    fn entitlements(&self, store: &Store) -> Arc<dyn Entitlements>;

    fn organization_hooks(&self, _config: &Config) -> Arc<dyn OrganizationHooks> {
        no_organization_hooks()
    }

    fn instance(&self) -> Arc<dyn InstanceEdition> {
        core_instance()
    }

    /// The SPDX identifier the edition is licensed under, when it is not core's.
    fn license(&self) -> Option<&'static str> {
        None
    }

    /// Settings found in the environment that the edition ignores, logged at startup.
    fn ignored_settings(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// The OSS edition: no license, every installation alike.
pub struct CoreEdition;

impl Edition for CoreEdition {
    fn entitlements(&self, _store: &Store) -> Arc<dyn Entitlements> {
        Arc::new(OssEntitlements)
    }
}

/// The modules an edition is composed of, and the edition itself.
#[derive(Clone)]
pub struct Modules {
    pub api: Vec<Arc<dyn ApiModule>>,
    pub workers: Vec<Arc<dyn WorkerModule>>,
    pub edition: Arc<dyn Edition>,
}

impl Modules {
    pub fn new(edition: Arc<dyn Edition>) -> Self {
        Self {
            api: Vec::new(),
            workers: Vec::new(),
            edition,
        }
    }

    /// No module, on the OSS edition.
    pub fn core() -> Self {
        Self::new(Arc::new(CoreEdition))
    }

    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .api
            .iter()
            .map(|m| m.id().to_string())
            .chain(self.workers.iter().map(|m| m.id().to_string()))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub async fn startup(&self, store: &Store, config: &Config) {
        for module in &self.api {
            module.on_startup(store, config).await;
        }
    }

    pub fn support_grants(&self, ctx: &ApiContext) -> Option<SharedSupportGrantReader> {
        self.api
            .iter()
            .find_map(|module| module.support_grants(ctx))
    }

    pub fn sandboxes(&self, ctx: &ApiContext) -> Arc<dyn Sandboxes> {
        self.api
            .iter()
            .find_map(|module| module.sandboxes(ctx))
            .unwrap_or_else(|| Arc::new(SeededSandboxes::new(ctx)))
    }

    pub fn rest_auth(&self, ctx: &ApiContext) -> Option<Arc<dyn RestDelegatedAuth>> {
        self.api.iter().find_map(|module| module.rest_auth(ctx))
    }

    pub fn portal_licenses(&self, ctx: &ApiContext) -> Option<Arc<dyn PortalLicenses>> {
        self.api
            .iter()
            .find_map(|module| module.portal_licenses(ctx))
    }

    pub fn store_hooks(&self) -> StoreHooks {
        let core = StoreHooks::default();
        StoreHooks {
            signals: self
                .api
                .iter()
                .find_map(|module| module.signal_hooks())
                .unwrap_or(core.signals),
            members: self
                .api
                .iter()
                .find_map(|module| module.member_hooks())
                .unwrap_or(core.members),
            credit: self
                .api
                .iter()
                .find_map(|module| module.credit_control_hooks())
                .unwrap_or(core.credit),
        }
    }

    pub fn revenue_stats(&self, ctx: &ApiContext) -> Arc<dyn RevenueStats> {
        self.api
            .iter()
            .find_map(|module| module.revenue_stats(ctx))
            .unwrap_or_else(|| postgres_revenue_stats(&ctx.store))
    }

    pub fn consumes(&self, queue: meteroid_store::domain::pgmq::PgmqQueue) -> bool {
        self.workers
            .iter()
            .any(|module| module.consumed_queues().contains(&queue))
    }

    pub fn entitlements(&self, store: &Store) -> Arc<dyn Entitlements> {
        self.edition.entitlements(store)
    }

    /// The gates the modules declare, each code once, in module order.
    pub fn feature_gates(&self) -> Vec<FeatureGate> {
        let mut gates: Vec<FeatureGate> = Vec::new();
        for gate in self.api.iter().flat_map(|module| module.feature_gates()) {
            if !gates.iter().any(|g| g.code == gate.code) {
                gates.push(*gate);
            }
        }
        gates
    }

    pub fn analytics_hook(
        &self,
        store: &Store,
        config: &Config,
        egress: &Egress,
    ) -> Arc<dyn AnalyticsHook> {
        self.api
            .iter()
            .find_map(|module| module.analytics_hook(store, config, egress))
            .unwrap_or_else(no_analytics_hook)
    }

    pub async fn rate_limiter(&self, redis: Option<&fred::prelude::Client>) -> RateLimiter {
        if let Some(redis) = redis {
            for module in &self.api {
                if let Some(backend) = module.rate_limit_backend(redis).await {
                    return RateLimiter::new(backend, RateLimitConfig::default());
                }
            }
        }
        RateLimiter::default()
    }

    pub fn destination_guard(&self) -> Option<Arc<dyn EgressGuard>> {
        self.api
            .iter()
            .find_map(|module| module.destination_guard())
    }

    pub fn route_policies(&self) -> Vec<RoutePolicy> {
        self.api
            .iter()
            .flat_map(|module| module.route_policies().iter().copied())
            .collect()
    }

    /// The edition's modules' share of the REST specs, under the edition's license when it has one.
    pub fn spec(&self) -> EditionSpec {
        let license = self.edition.license().map(|id| {
            utoipa::openapi::LicenseBuilder::new()
                .name(id)
                .identifier(Some(id))
                .build()
        });
        let merged = |docs: Vec<utoipa::openapi::OpenApi>| {
            let mut doc = docs.into_iter().fold(
                utoipa::openapi::OpenApiBuilder::new().build(),
                |mut merged, doc| {
                    merged.merge(doc);
                    merged
                },
            );
            doc.info.license = license.clone();
            doc
        };
        EditionSpec {
            api: merged(self.api.iter().filter_map(|m| m.openapi()).collect()),
            client: merged(self.api.iter().filter_map(|m| m.client_openapi()).collect()),
            webhook_events: self.webhook_events(),
        }
    }

    /// The webhook events the modules emit among those only a module emits.
    pub fn webhook_events(&self) -> Vec<&'static str> {
        self.api
            .iter()
            .flat_map(|module| module.webhook_events().iter().copied())
            .collect()
    }
}

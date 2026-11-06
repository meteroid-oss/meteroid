use common_domain::ids::{OrganizationId, TenantId};
use meteroid_store::Store;
use meteroid_store::domain::enums::TenantEnvironmentEnum;
use meteroid_store::domain::env_policy::environment_flag;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::sync::Arc;

/// A gate's code, as plans, licenses and subscriptions store it. Defined by the lowest crate
/// that checks it: core when core, or two modules, check it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeatureCode(&'static str);

impl FeatureCode {
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for FeatureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::fmt::Debug for FeatureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// A gate a module's feature sits behind, as the module declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureGate {
    pub code: FeatureCode,
    /// Part of the Intelligence add-on: the `intelligence_trial_environments` policy can open
    /// it in a non-production tenant.
    pub environment_trial: bool,
    /// Open wherever the module is in the edition, whatever the license says.
    pub bundled: bool,
}

impl FeatureGate {
    pub const fn new(code: FeatureCode) -> Self {
        Self {
            code,
            environment_trial: false,
            bundled: false,
        }
    }

    /// A gate of the Intelligence add-on.
    pub const fn trial(code: FeatureCode) -> Self {
        Self {
            code,
            environment_trial: true,
            bundled: false,
        }
    }

    pub const fn bundled(code: FeatureCode) -> Self {
        Self {
            code,
            environment_trial: false,
            bundled: true,
        }
    }
}

/// The gates core checks, or that more than one module checks.
pub mod gates {
    use super::FeatureCode;

    /// Hides "Powered by Meteroid" from customer emails, portals, checkout and dashboards.
    pub const WHITELABEL: FeatureCode = FeatureCode::new("whitelabel");
    pub const INVOICE_ANNEXES: FeatureCode = FeatureCode::new("invoice_annexes");
    /// Issuing software licenses to customers.
    pub const LICENSING: FeatureCode = FeatureCode::new("licensing");
    /// Exploratory sandbox presets (the AI credits example), on top of the default one.
    pub const ADMIN_SANDBOXES: FeatureCode = FeatureCode::new("admin_sandboxes");
    /// Custom roles: creating, editing and assigning organization-defined roles.
    pub const RBAC: FeatureCode = FeatureCode::new("rbac");
    /// Churn-risk and expansion rosters, per-customer signals.
    pub const CUSTOMER_SIGNALS: FeatureCode = FeatureCode::new("customer_signals");
    /// Organization Single Sign-On (OIDC).
    pub const SSO: FeatureCode = FeatureCode::new("sso");
    /// Enabling or changing a wallet's automatic top-up (the usage-controls module).
    pub const AUTO_TOP_UPS: FeatureCode = FeatureCode::new("auto_top_ups");
    /// Introducing or changing a minimum commitment on a plan, subscription or quote (the
    /// commitments module). Billing the minimums already stored stays open.
    pub const MINIMUM_COMMITMENTS: FeatureCode = FeatureCode::new("minimum_commitments");
    /// More than one signer on a quote.
    pub const QUOTE_MULTIPLE_SIGNERS: FeatureCode = FeatureCode::new("quote_multiple_signers");
    /// The customer entering a purchase order on a quote.
    pub const QUOTE_PURCHASE_ORDERS: FeatureCode = FeatureCode::new("quote_purchase_orders");
    /// Credit products, credit units and portal top-ups of credit units.
    pub const CREDIT_PRICING: FeatureCode = FeatureCode::new("credit_pricing");
    /// Credit granted to one subscription, to usage only or to some products.
    pub const SCOPED_CREDIT: FeatureCode = FeatureCode::new("scoped_credit");
    /// The usage cost reads and the running total of a subscription's current period.
    pub const USAGE_COSTS: FeatureCode = FeatureCode::new("usage_costs");
    /// Plan, add-on, seat and promo code changes from the customer portal.
    pub const PORTAL_SELF_SERVICE: FeatureCode = FeatureCode::new("portal_self_service");
    /// Embedding the customer portal.
    pub const PORTAL_EMBEDDING: FeatureCode = FeatureCode::new("portal_embedding");
    /// Changing the portal's theme mode and roundness.
    pub const PORTAL_THEMING: FeatureCode = FeatureCode::new("portal_theming");
    /// Per-kind settings, audiences, sender settings and test sends of customer emails, and the
    /// copy of payment reminder steps.
    pub const EMAIL_CUSTOMIZATION: FeatureCode = FeatureCode::new("email_customization");
}

/// How an enabled feature came to be enabled for the requesting tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitlementGrant {
    /// Owned: granted by the organization's plan / entitlements.
    Entitlement,
    /// Opened by the environment policy for a non-production tenant — not owned.
    Environment,
}

/// Resolved entitlement for a single gate.
///
/// `limit`/`remaining` are populated only for metered features (quota display);
/// `config` only for Config features, whose value drives policy rather than a gate.
#[derive(Clone, Debug)]
pub struct FeatureEntitlement {
    pub code: String,
    pub enabled: bool,
    pub limit: Option<Decimal>,
    pub remaining: Option<Decimal>,
    pub config: Option<serde_json::Value>,
    pub grant: Option<EntitlementGrant>,
}

impl FeatureEntitlement {
    fn on(code: FeatureCode) -> Self {
        Self {
            code: code.to_string(),
            enabled: true,
            limit: None,
            remaining: None,
            config: None,
            grant: Some(EntitlementGrant::Entitlement),
        }
    }
    fn off(code: FeatureCode) -> Self {
        Self {
            code: code.to_string(),
            enabled: false,
            limit: None,
            remaining: None,
            config: None,
            grant: None,
        }
    }
}

#[derive(Clone)]
enum Resolution {
    /// Every gate on: an operator organization, or a legacy license that predates entitlements.
    Open,
    /// Resolved from upstream — only codes matched from upstream feature names.
    Resolved(Arc<Vec<FeatureEntitlement>>),
    /// Upstream errored — fail closed (security gates must not open on outage).
    Closed,
}

#[derive(Clone)]
pub struct ResolvedEntitlements(Resolution);

impl ResolvedEntitlements {
    /// Every gate on, including gates added later.
    pub fn open() -> Self {
        Self(Resolution::Open)
    }

    /// The upstream could not be asked: every gate closed (security gates never open on outage).
    pub fn outage() -> Self {
        Self(Resolution::Closed)
    }

    /// Exactly the listed entitlements.
    pub fn from_entitlements(entitlements: Vec<FeatureEntitlement>) -> Self {
        Self(Resolution::Resolved(Arc::new(entitlements)))
    }

    /// The upstream could not be asked: every gate reads closed, but nothing is known.
    pub fn is_outage(&self) -> bool {
        matches!(self.0, Resolution::Closed)
    }

    /// `codes` read enabled; an outage stays closed.
    pub fn with_bundled(self, codes: &[FeatureCode]) -> Self {
        let Resolution::Resolved(listed) = &self.0 else {
            return self;
        };
        let mut v: Vec<FeatureEntitlement> = listed
            .iter()
            .filter(|e| !codes.iter().any(|c| c.as_str() == e.code))
            .cloned()
            .collect();
        v.extend(codes.iter().map(|c| FeatureEntitlement::on(*c)));
        Self(Resolution::Resolved(Arc::new(v)))
    }

    pub fn is_enabled(&self, code: FeatureCode) -> bool {
        match &self.0 {
            Resolution::Open => true,
            Resolution::Resolved(v) => v
                .iter()
                .find(|e| e.code == code.as_str())
                .is_some_and(|e| e.enabled),
            Resolution::Closed => false,
        }
    }

    pub fn get(&self, code: FeatureCode) -> FeatureEntitlement {
        match &self.0 {
            Resolution::Open => FeatureEntitlement::on(code),
            Resolution::Resolved(v) => v
                .iter()
                .find(|e| e.code == code.as_str())
                .cloned()
                .unwrap_or_else(|| FeatureEntitlement::off(code)),
            Resolution::Closed => FeatureEntitlement::off(code),
        }
    }

    /// Raw value of a Config feature, by upstream code. Config features carry
    /// policy (a map, a number, a flag), not a gate, so they are looked up by
    /// code rather than through `FeatureCode`.
    fn config(&self, code: &str) -> Option<&serde_json::Value> {
        match &self.0 {
            Resolution::Resolved(v) => v
                .iter()
                .find(|e| e.code == code)
                .and_then(|e| e.config.as_ref()),
            Resolution::Open | Resolution::Closed => None,
        }
    }

    /// Whether the Intelligence add-on opens in `environment` without the
    /// entitlement, per the `intelligence_trial_environments` policy map. Any
    /// environment the map doesn't name (directly or through `default`) falls back
    /// to the built-in policy: every non-production environment. An upstream outage
    /// (`Closed`) opens nothing.
    pub fn intelligence_opens_in(&self, environment: &TenantEnvironmentEnum) -> bool {
        match &self.0 {
            Resolution::Open => true,
            Resolution::Closed => false,
            Resolution::Resolved(_) => self
                .config(INTELLIGENCE_ENVIRONMENTS_CODE)
                .and_then(|map| environment_flag(map, environment))
                .unwrap_or(environment != &TenantEnvironmentEnum::Production),
        }
    }

    /// The entitlements of `gates` as they apply to a tenant in `environment`: a trial gate the
    /// org doesn't own, but that the environment policy opens, reports enabled with an
    /// `Environment` grant.
    pub fn all_entitlements(
        &self,
        environment: Option<&TenantEnvironmentEnum>,
        gates: &[FeatureGate],
    ) -> Vec<FeatureEntitlement> {
        let intelligence = environment.is_some_and(|env| self.intelligence_opens_in(env));
        gates
            .iter()
            .map(|gate| {
                let mut e = self.get(gate.code);
                if !e.enabled && intelligence && gate.environment_trial {
                    e.enabled = true;
                    e.grant = Some(EntitlementGrant::Environment);
                }
                e
            })
            .collect()
    }

    /// The explicit cap set under `code`: a Number config, or a metered feature's limit.
    pub fn limit(&self, code: &str) -> Option<u32> {
        let Resolution::Resolved(v) = &self.0 else {
            return None;
        };
        let e = v.iter().find(|e| e.code == code)?;
        e.config
            .as_ref()
            .and_then(config_decimal)
            .or(e.limit)?
            .trunc()
            .clamp(Decimal::ZERO, Decimal::from(u32::MAX))
            .to_u32()
    }

    /// Requests per minute a tenant of the org may make to the usage cost reads, when its plan
    /// sets a number; otherwise the instance's default applies.
    pub fn usage_costs_rate_limit(&self) -> Option<u64> {
        let per_minute: u64 = config_decimal(self.config(USAGE_COSTS_RATE_LIMIT_CODE)?)?
            .trunc()
            .try_into()
            .ok()?;
        (per_minute > 0).then_some(per_minute)
    }
}

#[doc(hidden)]
impl ResolvedEntitlements {
    pub fn with_enabled(codes: &[FeatureCode]) -> Self {
        let v = codes.iter().map(|c| FeatureEntitlement::on(*c)).collect();
        Self(Resolution::Resolved(Arc::new(v)))
    }
}

/// Config/NUMBER feature: the org's requests per minute on the usage cost reads.
pub const USAGE_COSTS_RATE_LIMIT_CODE: &str = "usage_costs_rate_limit";

/// Config/MAP feature whose truthy values open the Intelligence add-on in an
/// environment without the entitlement, e.g. `{"default": true, "production": false}`.
/// See `domain::env_policy` for the key and value rules.
pub const INTELLIGENCE_ENVIRONMENTS_CODE: &str = "intelligence_trial_environments";

/// A Number config value, which [`config_value_json`] carries as a decimal string.
fn config_decimal(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::Number(number) => number.to_string().parse().ok(),
        serde_json::Value::String(raw) => raw.trim().parse().ok(),
        _ => None,
    }
}

/// Whether an Intelligence gate is open for this request: owned by the org, or
/// opened by the environment policy for this tenant. A tenant lookup failure
/// falls back to the entitlement alone.
pub async fn intelligence_gate_open(
    store: &Store,
    resolved: &ResolvedEntitlements,
    code: FeatureCode,
    tenant_id: TenantId,
) -> bool {
    use meteroid_store::repositories::tenants::TenantInterface;

    if resolved.is_enabled(code) {
        return true;
    }
    match store.find_tenant_by_id(tenant_id).await {
        Ok(t) => intelligence_gate_open_for(resolved, code, &t.tenant.environment),
        Err(e) => {
            tracing::warn!(
                "intelligence policy check failed for {}: {:?}",
                tenant_id,
                e
            );
            false
        }
    }
}

/// [`intelligence_gate_open`] for a caller that already holds the tenant's environment.
/// Accounts and the paid built-in signals sit behind the Customer Signals gate.
pub async fn customer_signals_gate_open(
    resolver: &dyn Entitlements,
    store: &Store,
    org_id: OrganizationId,
    tenant_id: TenantId,
) -> bool {
    let resolved = resolver.resolve(org_id).await;
    intelligence_gate_open(store, &resolved, gates::CUSTOMER_SIGNALS, tenant_id).await
}

/// `resolve(..).is_enabled(code)` for a request that names a tenant, not an organization (the
/// customer portal). A failed tenant lookup is an error, never an open gate.
pub async fn tenant_gate_open(
    resolver: &dyn Entitlements,
    store: &Store,
    tenant_id: TenantId,
    code: FeatureCode,
) -> meteroid_store::StoreResult<bool> {
    use meteroid_store::repositories::tenants::TenantInterface;

    let tenant = store.find_tenant_by_id(tenant_id).await?;
    Ok(resolver
        .resolve(tenant.organization.id)
        .await
        .is_enabled(code))
}

pub fn intelligence_gate_open_for(
    resolved: &ResolvedEntitlements,
    code: FeatureCode,
    environment: &TenantEnvironmentEnum,
) -> bool {
    resolved.is_enabled(code) || resolved.intelligence_opens_in(environment)
}

/// Cap on alert definitions for organizations without a subscription.
pub const DEFAULT_FREE_TIER_ALERT_LIMIT: u32 = 3;

/// Cap on alert definitions per organization, when a license sets one.
pub const ALERT_DEFINITIONS_CODE: &str = "alert_definitions";
/// Cap on the Production tenants of an organization (Number config or metered limit).
pub const PRODUCTION_TENANTS_CODE: &str = "production_tenants";
/// Cap on the invoicing entities of a tenant (Number config or metered limit).
pub const INVOICING_ENTITIES_CODE: &str = "invoicing_entities";

/// What a plan or license unlocks inside the code an edition ships. The edition picks the
/// implementation; its module list decides what code ships at all.
#[async_trait::async_trait]
pub trait Entitlements: Send + Sync {
    async fn resolve(&self, org_id: OrganizationId) -> ResolvedEntitlements;

    /// The cap the caller enforces on alert definitions, or `None` when uncapped. Counting the
    /// tenant's existing definitions and rejecting the create is the caller's job.
    async fn alert_definition_limit(&self, org_id: OrganizationId) -> Option<u32>;

    /// Production tenants the org may hold, or `None` when uncapped.
    async fn production_tenant_limit(&self, org_id: OrganizationId) -> Option<u32>;

    /// Invoicing entities a tenant of the org may hold, or `None` when uncapped.
    async fn invoicing_entity_limit(&self, org_id: OrganizationId) -> Option<u32>;
}

pub fn with_bundled_gates(
    inner: Arc<dyn Entitlements>,
    gates: &[FeatureGate],
) -> Arc<dyn Entitlements> {
    let codes: Vec<FeatureCode> = gates.iter().filter(|g| g.bundled).map(|g| g.code).collect();
    if codes.is_empty() {
        return inner;
    }
    Arc::new(BundledEntitlements { inner, codes })
}

struct BundledEntitlements {
    inner: Arc<dyn Entitlements>,
    codes: Vec<FeatureCode>,
}

#[async_trait::async_trait]
impl Entitlements for BundledEntitlements {
    async fn resolve(&self, org_id: OrganizationId) -> ResolvedEntitlements {
        self.inner.resolve(org_id).await.with_bundled(&self.codes)
    }

    async fn alert_definition_limit(&self, org_id: OrganizationId) -> Option<u32> {
        self.inner.alert_definition_limit(org_id).await
    }

    async fn production_tenant_limit(&self, org_id: OrganizationId) -> Option<u32> {
        self.inner.production_tenant_limit(org_id).await
    }

    async fn invoicing_entity_limit(&self, org_id: OrganizationId) -> Option<u32> {
        self.inner.invoicing_entity_limit(org_id).await
    }
}

/// The OSS edition: no license and no gated feature, so every gate core checks is closed.
/// Like its single organization, its one Production tenant and one invoicing entity per tenant
/// are the edition's, not a cap.
pub struct OssEntitlements;

#[async_trait::async_trait]
impl Entitlements for OssEntitlements {
    async fn resolve(&self, _org_id: OrganizationId) -> ResolvedEntitlements {
        ResolvedEntitlements::from_entitlements(vec![])
    }

    async fn alert_definition_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        None
    }

    async fn production_tenant_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        Some(1)
    }

    async fn invoicing_entity_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        Some(1)
    }
}

/// Refuses a Production tenant past the org's `production_tenants` limit.
pub async fn ensure_production_tenant_allowed(
    resolver: &dyn Entitlements,
    store: &Store,
    org_id: OrganizationId,
) -> Result<(), tonic::Status> {
    use meteroid_store::repositories::OrganizationsInterface;

    let Some(limit) = resolver.production_tenant_limit(org_id).await else {
        return Ok(());
    };
    let count = store
        .get_organizations_with_tenants_by_id(org_id)
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to count tenants: {e}")))?
        .tenants
        .iter()
        .filter(|t| t.environment == TenantEnvironmentEnum::Production && t.archived_at.is_none())
        .count();
    if count >= limit as usize {
        return Err(tonic::Status::resource_exhausted(format!(
            "This organization can hold up to {limit} Production tenant(s)"
        )));
    }
    Ok(())
}

/// Refuses an invoicing entity past the `invoicing_entities` limit. Tenants already past it
/// keep the entities they have.
pub async fn ensure_invoicing_entity_allowed(
    resolver: &dyn Entitlements,
    store: &Store,
    org_id: OrganizationId,
    tenant_id: TenantId,
) -> Result<(), tonic::Status> {
    use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

    let Some(limit) = resolver.invoicing_entity_limit(org_id).await else {
        return Ok(());
    };
    let count = store
        .list_invoicing_entities(tenant_id)
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to count invoicing entities: {e}")))?
        .len();
    if count >= limit as usize {
        return Err(tonic::Status::resource_exhausted(format!(
            "This organization can hold up to {limit} invoicing entit{} per tenant",
            if limit == 1 { "y" } else { "ies" }
        )));
    }
    Ok(())
}

pub const MINIMUM_COMMITMENTS_NOT_IN_PLAN: &str =
    "Minimum commitments are not enabled for this organization.";

/// Whether the org may introduce or change minimum commitments. Without the gate a write may
/// still keep the stored minimum, so editing existing data survives a downgrade.
pub async fn minimum_commitments_open(resolver: &dyn Entitlements, org_id: OrganizationId) -> bool {
    resolver
        .resolve(org_id)
        .await
        .is_enabled(gates::MINIMUM_COMMITMENTS)
}

pub fn minimum_commitments_refused() -> tonic::Status {
    tonic::Status::permission_denied(MINIMUM_COMMITMENTS_NOT_IN_PLAN)
}

pub const QUOTE_MULTIPLE_SIGNERS_NOT_IN_PLAN: &str =
    "Quotes with more than one signer are not enabled for this organization.";
pub const SCOPED_CREDIT_NOT_IN_PLAN: &str = "Scoped credit is not enabled for this organization.";
pub const CREDIT_PRICING_NOT_IN_PLAN: &str =
    "Credit packs, credit units and scoped credit are not enabled for this organization.";
pub const PORTAL_SELF_SERVICE_NOT_IN_PLAN: &str =
    "Changing a subscription from the portal is not available; please contact your provider";
pub const PORTAL_THEMING_NOT_IN_PLAN: &str = "Portal theming is not enabled for this organization.";
pub const PORTAL_EMBEDDING_NOT_IN_PLAN: &str =
    "Embedding the customer portal is not enabled for this organization.";
pub const EMAIL_CUSTOMIZATION_NOT_IN_PLAN: &str =
    "Customizing customer emails is not enabled for this organization.";
pub const QUOTE_PURCHASE_ORDERS_NOT_IN_PLAN: &str =
    "Purchase orders on quotes are not enabled for this organization.";

pub async fn ensure_credit_pricing_allowed(
    resolver: &dyn Entitlements,
    org_id: OrganizationId,
    introduces: bool,
) -> Result<(), tonic::Status> {
    ensure_change_allowed(
        resolver,
        org_id,
        gates::CREDIT_PRICING,
        introduces,
        CREDIT_PRICING_NOT_IN_PLAN,
    )
    .await
}

pub async fn ensure_portal_self_service(
    resolver: &dyn Entitlements,
    store: &Store,
    tenant_id: TenantId,
) -> Result<(), tonic::Status> {
    let open = tenant_gate_open(resolver, store, tenant_id, gates::PORTAL_SELF_SERVICE)
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to resolve features: {e}")))?;
    if open {
        Ok(())
    } else {
        Err(tonic::Status::permission_denied(
            PORTAL_SELF_SERVICE_NOT_IN_PLAN,
        ))
    }
}

/// Refuses a change behind `code` unless the org holds it.
pub async fn ensure_change_allowed(
    resolver: &dyn Entitlements,
    org_id: OrganizationId,
    code: FeatureCode,
    changes: bool,
    refusal: &'static str,
) -> Result<(), tonic::Status> {
    if !changes || resolver.resolve(org_id).await.is_enabled(code) {
        return Ok(());
    }
    Err(tonic::Status::permission_denied(refusal))
}

/// [`gates::WHITELABEL`] hides "Powered by Meteroid" from customer emails, portals, checkout and
/// dashboards.
#[async_trait::async_trait]
pub trait WhitelabelResolver: Send + Sync {
    async fn is_whitelabel(&self, tenant_id: TenantId) -> bool;
}

/// Tenant → organization (cached lookup), then the org-keyed `resolve`; a lookup error fails
/// closed (footer shown).
pub struct EntitlementsWhitelabelResolver {
    store: Arc<Store>,
    features: Arc<dyn Entitlements>,
}

impl EntitlementsWhitelabelResolver {
    pub fn new(store: Arc<Store>, features: Arc<dyn Entitlements>) -> Self {
        Self { store, features }
    }
}

#[async_trait::async_trait]
impl WhitelabelResolver for EntitlementsWhitelabelResolver {
    async fn is_whitelabel(&self, tenant_id: TenantId) -> bool {
        is_whitelabel(self.features.as_ref(), &self.store, tenant_id).await
    }
}

/// Whether the tenant's organization hides Meteroid's branding; a lookup error shows it.
pub async fn is_whitelabel(
    features: &dyn Entitlements,
    store: &Store,
    tenant_id: TenantId,
) -> bool {
    use meteroid_store::repositories::tenants::TenantInterface;

    match store.find_tenant_by_id(tenant_id).await {
        Ok(t) => features
            .resolve(t.organization.id)
            .await
            .is_enabled(gates::WHITELABEL),
        Err(e) => {
            tracing::warn!("whitelabel check failed for {}: {:?}", tenant_id, e);
            false
        }
    }
}

/// Tests and `run_once_send_email`: never whitelabel.
pub struct NoWhitelabel;

#[async_trait::async_trait]
impl WhitelabelResolver for NoWhitelabel {
    async fn is_whitelabel(&self, _tenant_id: TenantId) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    const ANALYTICS: FeatureCode = FeatureCode::new("analytics");
    const ACTIVATION: FeatureCode = FeatureCode::new("activation");
    const AUDIT_LOG: FeatureCode = FeatureCode::new("audit_log");
    const MFA_POLICY: FeatureCode = FeatureCode::new("mfa_policy");

    const GATES: [FeatureGate; 5] = [
        FeatureGate::trial(ANALYTICS),
        FeatureGate::trial(ACTIVATION),
        FeatureGate::trial(gates::CUSTOMER_SIGNALS),
        FeatureGate::new(AUDIT_LOG),
        FeatureGate::new(MFA_POLICY),
    ];

    const CORE_GATES: [FeatureCode; 18] = [
        gates::CREDIT_PRICING,
        gates::SCOPED_CREDIT,
        gates::USAGE_COSTS,
        gates::EMAIL_CUSTOMIZATION,
        gates::PORTAL_SELF_SERVICE,
        gates::PORTAL_EMBEDDING,
        gates::PORTAL_THEMING,
        gates::QUOTE_MULTIPLE_SIGNERS,
        gates::QUOTE_PURCHASE_ORDERS,
        gates::WHITELABEL,
        gates::INVOICE_ANNEXES,
        gates::LICENSING,
        gates::ADMIN_SANDBOXES,
        gates::RBAC,
        gates::CUSTOMER_SIGNALS,
        gates::SSO,
        gates::AUTO_TOP_UPS,
        gates::MINIMUM_COMMITMENTS,
    ];

    fn map(pairs: &[(FeatureCode, bool)]) -> ResolvedEntitlements {
        let v = pairs
            .iter()
            .map(|(code, enabled)| FeatureEntitlement {
                code: code.to_string(),
                enabled: *enabled,
                limit: None,
                remaining: None,
                config: None,
                grant: enabled.then_some(EntitlementGrant::Entitlement),
            })
            .collect();
        ResolvedEntitlements(Resolution::Resolved(Arc::new(v)))
    }

    /// Entitlements carrying the environment policy config, with no gate enabled.
    fn with_policy(policy: serde_json::Value) -> ResolvedEntitlements {
        ResolvedEntitlements(Resolution::Resolved(Arc::new(vec![FeatureEntitlement {
            code: INTELLIGENCE_ENVIRONMENTS_CODE.to_string(),
            enabled: false,
            limit: None,
            remaining: None,
            config: Some(policy),
            grant: None,
        }])))
    }

    #[test]
    fn a_code_reads_as_the_string_it_is_stored_as() {
        assert_eq!(gates::WHITELABEL.to_string(), "whitelabel");
        assert_eq!(gates::CUSTOMER_SIGNALS.as_str(), "customer_signals");
        assert_eq!(format!("{:?}", gates::ADMIN_SANDBOXES), "admin_sandboxes");
        assert_eq!(FeatureCode::new("rbac"), gates::RBAC);
    }

    #[test]
    fn without_policy_every_non_production_environment_opens() {
        let r = map(&[(ANALYTICS, false)]);
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Sandbox));
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Development));
        assert!(!r.intelligence_opens_in(&TenantEnvironmentEnum::Production));
    }

    #[test]
    fn policy_map_overrides_the_built_in_default() {
        let r = with_policy(serde_json::json!({
            "sandbox": false,
            "production": true,
        }));
        assert!(!r.intelligence_opens_in(&TenantEnvironmentEnum::Sandbox));
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Production));
        // An environment the map names neither directly nor by `default` keeps
        // the built-in policy.
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Qa));
    }

    #[test]
    fn a_default_key_covers_the_environments_the_policy_does_not_name() {
        let r = with_policy(serde_json::json!({ "default": true, "production": false }));
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Sandbox));
        assert!(r.intelligence_opens_in(&TenantEnvironmentEnum::Qa));
        assert!(!r.intelligence_opens_in(&TenantEnvironmentEnum::Production));
    }

    #[test]
    fn the_listed_gates_are_exactly_the_given_ones() {
        let out =
            map(&[(AUDIT_LOG, true), (gates::WHITELABEL, true)]).all_entitlements(None, &GATES);
        let codes: Vec<&str> = out.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "analytics",
                "activation",
                "customer_signals",
                "audit_log",
                "mfa_policy"
            ]
        );
    }

    #[test]
    fn a_non_production_tenant_gets_every_trial_gate_opened() {
        let r = map(&[(ACTIVATION, false)]);
        let opened = r.all_entitlements(Some(&TenantEnvironmentEnum::Sandbox), &GATES);
        for gate in GATES.iter().filter(|g| g.environment_trial) {
            let code = gate.code;
            let e = opened
                .iter()
                .find(|e| e.code == code.as_str())
                .unwrap_or_else(|| panic!("{code:?} missing from the entitlement list"));
            assert!(e.enabled, "{code:?} stayed closed in Sandbox");
            assert_eq!(e.grant, Some(EntitlementGrant::Environment));
        }
    }

    #[test]
    fn an_upstream_outage_opens_nothing() {
        let r = ResolvedEntitlements(Resolution::Closed);
        assert!(!r.intelligence_opens_in(&TenantEnvironmentEnum::Sandbox));
        assert!(r.is_outage());
        assert!(!ResolvedEntitlements(Resolution::Open).is_outage());
        assert!(!map(&[(gates::CUSTOMER_SIGNALS, false)]).is_outage());
    }

    #[test]
    fn environment_grant_marks_a_gate_the_org_does_not_own() {
        let r = map(&[(ANALYTICS, false), (AUDIT_LOG, true)]);
        let out = r.all_entitlements(Some(&TenantEnvironmentEnum::Sandbox), &GATES);
        let by = |code: FeatureCode| {
            out.iter()
                .find(|e| e.code == code.as_str())
                .cloned()
                .unwrap()
        };

        let analytics = by(ANALYTICS);
        assert!(analytics.enabled);
        assert_eq!(analytics.grant, Some(EntitlementGrant::Environment));

        // A non-trial gate is untouched by the policy.
        let audit = by(AUDIT_LOG);
        assert_eq!(audit.grant, Some(EntitlementGrant::Entitlement));
        assert!(!by(MFA_POLICY).enabled);
    }

    #[test]
    fn production_reports_intelligence_as_locked() {
        let r = map(&[(ANALYTICS, false)]);
        let out = r.all_entitlements(Some(&TenantEnvironmentEnum::Production), &GATES);
        let analytics = out.iter().find(|e| e.code == ANALYTICS.as_str()).unwrap();
        assert!(!analytics.enabled);
        assert_eq!(analytics.grant, None);
    }

    #[test]
    fn intelligence_gate_open_for_matches_async_variant() {
        let closed = map(&[(gates::CUSTOMER_SIGNALS, false)]);
        let gate = |r: &ResolvedEntitlements, env| {
            intelligence_gate_open_for(r, gates::CUSTOMER_SIGNALS, &env)
        };
        assert!(!gate(&closed, TenantEnvironmentEnum::Production));
        assert!(gate(&closed, TenantEnvironmentEnum::Sandbox));

        let owned = map(&[(gates::CUSTOMER_SIGNALS, true)]);
        assert!(gate(&owned, TenantEnvironmentEnum::Production));

        let outage = ResolvedEntitlements(Resolution::Closed);
        assert!(!gate(&outage, TenantEnvironmentEnum::Sandbox));
        assert!(gate(
            &ResolvedEntitlements(Resolution::Open),
            TenantEnvironmentEnum::Production
        ));
    }

    #[test]
    fn map_lookup_respects_enabled() {
        let r = map(&[(AUDIT_LOG, true), (MFA_POLICY, false)]);
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(!r.is_enabled(MFA_POLICY));
        assert!(
            !ResolvedEntitlements(Resolution::Resolved(Arc::new(vec![]))).is_enabled(AUDIT_LOG)
        );
    }

    #[tokio::test]
    async fn oss_closes_every_gate_and_holds_one_production_tenant_and_entity() {
        let org = OrganizationId::new();
        let r = OssEntitlements.resolve(org).await;
        for code in CORE_GATES {
            assert!(!r.is_enabled(code), "{code:?}");
        }
        assert!(!r.is_outage());
        assert_eq!(OssEntitlements.alert_definition_limit(org).await, None);
        assert_eq!(OssEntitlements.production_tenant_limit(org).await, Some(1));
        assert_eq!(OssEntitlements.invoicing_entity_limit(org).await, Some(1));
    }

    #[test]
    fn open_enables_everything() {
        let r = ResolvedEntitlements(Resolution::Open);
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(r.is_enabled(FeatureCode::new("a_gate_added_later")));
    }

    #[test]
    fn closed_disables_everything() {
        let r = ResolvedEntitlements(Resolution::Closed);
        assert!(!r.is_enabled(AUDIT_LOG));
        assert!(!r.is_enabled(MFA_POLICY));
    }

    fn with_config(code: &str, value: serde_json::Value) -> ResolvedEntitlements {
        ResolvedEntitlements(Resolution::Resolved(Arc::new(vec![FeatureEntitlement {
            code: code.to_string(),
            enabled: false,
            limit: None,
            remaining: None,
            config: Some(value),
            grant: None,
        }])))
    }

    #[test]
    fn a_creation_cap_reads_a_number_config_or_a_metered_limit() {
        let number =
            |value| with_config(INVOICING_ENTITIES_CODE, value).limit(INVOICING_ENTITIES_CODE);
        assert_eq!(number(serde_json::json!("3")), Some(3));
        assert_eq!(number(serde_json::json!(2)), Some(2));
        assert_eq!(number(serde_json::json!("2.9")), Some(2));
        assert_eq!(number(serde_json::json!("-1")), Some(0));
        assert_eq!(number(serde_json::json!({"production": 1})), None);

        let metered =
            ResolvedEntitlements(Resolution::Resolved(Arc::new(vec![FeatureEntitlement {
                code: PRODUCTION_TENANTS_CODE.to_string(),
                enabled: true,
                limit: Some(Decimal::from(4)),
                remaining: None,
                config: None,
                grant: Some(EntitlementGrant::Entitlement),
            }])));
        assert_eq!(metered.limit(PRODUCTION_TENANTS_CODE), Some(4));
        assert_eq!(metered.limit(INVOICING_ENTITIES_CODE), None);
    }

    #[test]
    fn usage_costs_rate_limit_reads_a_positive_number_and_defaults_otherwise() {
        let limit =
            |value| with_config(USAGE_COSTS_RATE_LIMIT_CODE, value).usage_costs_rate_limit();
        assert_eq!(limit(serde_json::json!("1200")), Some(1200));
        assert_eq!(limit(serde_json::json!(" 90.7 ")), Some(90));
        assert_eq!(limit(serde_json::json!(300)), Some(300));
        assert_eq!(limit(serde_json::json!(90.7)), Some(90));
        assert_eq!(limit(serde_json::json!("0")), None);
        assert_eq!(limit(serde_json::json!("-5")), None);
        assert_eq!(limit(serde_json::json!({"production": 10})), None);
        assert_eq!(
            with_config("other", serde_json::json!("10")).usage_costs_rate_limit(),
            None
        );
        assert_eq!(
            ResolvedEntitlements(Resolution::Open).usage_costs_rate_limit(),
            None
        );
        assert_eq!(
            ResolvedEntitlements::outage().usage_costs_rate_limit(),
            None
        );
    }

    #[test]
    fn a_bundled_gate_opens_whatever_the_license_lists() {
        let r = map(&[(AUDIT_LOG, false), (MFA_POLICY, true)]).with_bundled(&[AUDIT_LOG]);
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(r.is_enabled(MFA_POLICY));
        assert!(!r.is_enabled(ANALYTICS));
        assert_eq!(r.get(AUDIT_LOG).grant, Some(EntitlementGrant::Entitlement));
        assert!(
            ResolvedEntitlements::from_entitlements(vec![])
                .with_bundled(&[AUDIT_LOG])
                .is_enabled(AUDIT_LOG)
        );
        assert!(
            !ResolvedEntitlements::outage()
                .with_bundled(&[AUDIT_LOG])
                .is_enabled(AUDIT_LOG)
        );
    }

    #[tokio::test]
    async fn only_bundled_gates_are_opened_by_the_wrapper() {
        let gates = [
            FeatureGate::bundled(AUDIT_LOG),
            FeatureGate::new(MFA_POLICY),
        ];
        let resolver = with_bundled_gates(Arc::new(OssEntitlements), &gates);
        let r = resolver.resolve(OrganizationId::new()).await;
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(!r.is_enabled(MFA_POLICY));
        assert_eq!(
            resolver
                .production_tenant_limit(OrganizationId::new())
                .await,
            Some(1)
        );
    }
}

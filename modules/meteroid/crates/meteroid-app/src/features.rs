use common_domain::ids::{OrganizationId, TenantId};
use meteroid_store::Store;
use meteroid_store::domain::billable_metrics::normalize_usage_breakdown_key;
use meteroid_store::domain::enums::TenantEnvironmentEnum;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateKind {
    Boolean,
    Number,
    Limit,
}

/// Where a gate's value is read: for the organization (our customer), or for one of its
/// tenants, which our catalog's conditions can tell apart (`tenant.environment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateLevel {
    Organization,
    Tenant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateValue {
    Off,
    On,
    Count(u64),
    Unlimited,
}

/// A gate a module's feature sits behind, as the module declares it (ADR 0011 §3.13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureGate {
    pub code: FeatureCode,
    pub kind: GateKind,
    pub unit: Option<&'static str>,
    pub level: GateLevel,
    /// Read while our billing can't be asked.
    pub outage: GateValue,
    /// Read when our catalog doesn't know the code, so code can ship before it is configured.
    pub default: GateValue,
    /// Open wherever the module is in the edition, whatever the license says.
    pub bundled: bool,
}

impl FeatureGate {
    /// A boolean gate of the organization, closed by default and on outage.
    pub const fn new(code: FeatureCode) -> Self {
        Self {
            code,
            kind: GateKind::Boolean,
            unit: None,
            level: GateLevel::Organization,
            outage: GateValue::Off,
            default: GateValue::Off,
            bundled: false,
        }
    }

    /// Opened by the module being in the edition (`with_bundled_gates`).
    pub const fn bundled(code: FeatureCode) -> Self {
        Self {
            bundled: true,
            ..Self::new(code)
        }
    }

    /// A cap the caller counts, unlimited by default and on outage.
    pub const fn limit(code: FeatureCode) -> Self {
        Self {
            kind: GateKind::Limit,
            outage: GateValue::Unlimited,
            default: GateValue::Unlimited,
            ..Self::new(code)
        }
    }

    /// A number the caller applies; unset (`Off`) leaves the caller's own default.
    pub const fn number(code: FeatureCode, unit: &'static str) -> Self {
        Self {
            kind: GateKind::Number,
            unit: Some(unit),
            ..Self::new(code)
        }
    }

    pub const fn per_tenant(self) -> Self {
        Self {
            level: GateLevel::Tenant,
            ..self
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
    /// Advanced quoting: several signers, purchase orders, revisions, line editing and
    /// discounts, templates, approvals, countersignatures, verified signatures, CRM deal links
    /// and quote analytics.
    pub const QUOTES_FULL: FeatureCode = FeatureCode::new("quotes_full");
    /// Credit products, credit units and portal top-ups of credit units.
    pub const CREDIT_PRICING: FeatureCode = FeatureCode::new("credit_pricing");
    /// Credit granted to one subscription, to usage only or to some products.
    pub const SCOPED_CREDIT: FeatureCode = FeatureCode::new("scoped_credit");
    /// The usage cost reads and the running total of a subscription's current period.
    pub const USAGE_COSTS: FeatureCode = FeatureCode::new("usage_costs");
    /// Setting a billable metric's usage breakdown key (the usage-breakdown module). Invoices
    /// keep breaking down by a key already stored.
    pub const USAGE_BREAKDOWN: FeatureCode = FeatureCode::new("usage_breakdown");
    /// Plan, add-on, seat and promo code changes from the customer portal.
    pub const PORTAL_SELF_SERVICE: FeatureCode = FeatureCode::new("portal_self_service");
    /// Embedding the customer portal.
    pub const PORTAL_EMBEDDING: FeatureCode = FeatureCode::new("portal_embedding");
    /// Changing the portal's theme mode and roundness.
    pub const PORTAL_THEMING: FeatureCode = FeatureCode::new("portal_theming");
    /// Per-kind settings, audiences, sender settings and test sends of customer emails, and the
    /// copy of payment reminder steps.
    pub const EMAIL_CUSTOMIZATION: FeatureCode = FeatureCode::new("email_customization");
    /// The AI module (cost book, AI margin, gateway integrations): the license lets a tenant turn
    /// it on; core refuses cost cards unless it is on.
    pub const AI: FeatureCode = FeatureCode::new("ai");
    /// The portal cancel flow, and renewal notices of periods, end dates and cancellations
    /// (term renewals are noticed without it).
    pub const RETENTION: FeatureCode = FeatureCode::new("retention");
}

pub const ALERT_DEFINITIONS_CODE: &str = "alert_definitions";
pub const PRODUCTION_TENANTS_CODE: &str = "production_tenants";
pub const INVOICING_ENTITIES_CODE: &str = "invoicing_entities";
pub const USAGE_COSTS_RATE_LIMIT_CODE: &str = "usage_costs_rate_limit";
/// The self-hosted free tier's cap on alert definitions.
pub const DEFAULT_FREE_TIER_ALERT_LIMIT: u32 = 3;

/// The caps and numbers core reads; limits are counted by the caller (`current_count`).
pub mod quotas {
    use super::{
        ALERT_DEFINITIONS_CODE, FeatureCode, FeatureGate, INVOICING_ENTITIES_CODE,
        PRODUCTION_TENANTS_CODE, USAGE_COSTS_RATE_LIMIT_CODE,
    };

    pub const ALERT_DEFINITIONS: FeatureGate =
        FeatureGate::limit(FeatureCode::new(ALERT_DEFINITIONS_CODE)).per_tenant();
    pub const PRODUCTION_TENANTS: FeatureGate =
        FeatureGate::limit(FeatureCode::new(PRODUCTION_TENANTS_CODE));
    pub const INVOICING_ENTITIES: FeatureGate =
        FeatureGate::limit(FeatureCode::new(INVOICING_ENTITIES_CODE)).per_tenant();
    /// The usage cost reads' rate; the limiter stays in code.
    pub const USAGE_COSTS_RATE_LIMIT: FeatureGate = FeatureGate::number(
        FeatureCode::new(USAGE_COSTS_RATE_LIMIT_CODE),
        "requests per minute",
    );
}

/// Resolved entitlement for a single gate.
///
/// `limit`/`remaining` are populated only for limits (quota display); `config` only for
/// values, which drive policy rather than a gate.
#[derive(Clone, Debug)]
pub struct FeatureEntitlement {
    pub code: String,
    pub enabled: bool,
    pub limit: Option<Decimal>,
    pub remaining: Option<Decimal>,
    pub config: Option<serde_json::Value>,
}

impl FeatureEntitlement {
    fn on(code: FeatureCode) -> Self {
        Self {
            enabled: true,
            ..Self::off(code)
        }
    }
    fn off(code: FeatureCode) -> Self {
        Self {
            code: code.to_string(),
            enabled: false,
            limit: None,
            remaining: None,
            config: None,
        }
    }

    fn value(&self, kind: GateKind) -> GateValue {
        let count = |n: Decimal| match n.trunc().to_u64() {
            Some(n) => GateValue::Count(n),
            None if n.is_sign_negative() => GateValue::Off,
            None => GateValue::Unlimited,
        };
        let number = self.config.as_ref().and_then(config_decimal);
        match kind {
            GateKind::Boolean if self.enabled => GateValue::On,
            GateKind::Boolean => GateValue::Off,
            GateKind::Number => number.map_or(GateValue::Off, count),
            GateKind::Limit => match number.or(self.limit) {
                Some(n) => count(n),
                None if self.enabled => GateValue::Unlimited,
                None => GateValue::Off,
            },
        }
    }
}

#[derive(Clone)]
enum Resolution {
    /// Every gate on: an operator organization, or a legacy license that predates entitlements.
    Open,
    /// Resolved from upstream — only codes matched from upstream feature names.
    Resolved(Arc<Vec<FeatureEntitlement>>),
    /// Upstream errored: declared gates read their outage value, others are closed.
    Closed,
}

#[derive(Clone)]
pub struct ResolvedEntitlements(Resolution);

impl ResolvedEntitlements {
    /// Every gate on, including gates added later.
    pub fn open() -> Self {
        Self(Resolution::Open)
    }

    /// The upstream could not be asked.
    pub fn outage() -> Self {
        Self(Resolution::Closed)
    }

    /// Exactly the listed entitlements.
    pub fn from_entitlements(entitlements: Vec<FeatureEntitlement>) -> Self {
        Self(Resolution::Resolved(Arc::new(entitlements)))
    }

    /// The upstream could not be asked: nothing is known.
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

    fn find(&self, code: &str) -> Option<&FeatureEntitlement> {
        match &self.0 {
            Resolution::Resolved(v) => v.iter().find(|e| e.code == code),
            Resolution::Open | Resolution::Closed => None,
        }
    }

    pub fn is_enabled(&self, code: FeatureCode) -> bool {
        self.enabled(&FeatureGate::new(code))
    }

    pub fn get(&self, code: FeatureCode) -> FeatureEntitlement {
        match &self.0 {
            Resolution::Open => FeatureEntitlement::on(code),
            Resolution::Resolved(_) => self
                .find(code.as_str())
                .cloned()
                .unwrap_or_else(|| FeatureEntitlement::off(code)),
            Resolution::Closed => FeatureEntitlement::off(code),
        }
    }

    /// A declared gate's value: its default when upstream doesn't know it, its outage value
    /// when upstream can't be asked.
    pub fn value(&self, gate: &FeatureGate) -> GateValue {
        match &self.0 {
            Resolution::Open => match gate.kind {
                GateKind::Boolean => GateValue::On,
                GateKind::Limit => GateValue::Unlimited,
                GateKind::Number => gate.default,
            },
            Resolution::Resolved(_) => self
                .find(gate.code.as_str())
                .map_or(gate.default, |e| e.value(gate.kind)),
            Resolution::Closed => gate.outage,
        }
    }

    pub fn enabled(&self, gate: &FeatureGate) -> bool {
        self.value(gate) != GateValue::Off
    }

    /// A limit gate's cap, `None` when uncapped.
    pub fn limit_of(&self, gate: &FeatureGate) -> Option<u32> {
        match self.value(gate) {
            GateValue::Off => Some(0),
            GateValue::Count(n) => Some(n.min(u32::MAX as u64) as u32),
            GateValue::On | GateValue::Unlimited => None,
        }
    }

    /// A number gate's value, `None` when unset.
    pub fn number(&self, gate: &FeatureGate) -> Option<u64> {
        match self.value(gate) {
            GateValue::Count(n) => Some(n),
            _ => None,
        }
    }

    /// The entitlements of `gates`, as the gate list shows them.
    pub fn all_entitlements(&self, gates: &[FeatureGate]) -> Vec<FeatureEntitlement> {
        gates
            .iter()
            .map(|gate| FeatureEntitlement {
                enabled: self.enabled(gate),
                ..self.get(gate.code)
            })
            .collect()
    }

    /// The explicit cap set under `code`: a Number config, or a metered feature's limit.
    pub fn limit(&self, code: &str) -> Option<u32> {
        let e = self.find(code)?;
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
        self.number(&quotas::USAGE_COSTS_RATE_LIMIT)
            .filter(|n| *n > 0)
    }
}

#[doc(hidden)]
impl ResolvedEntitlements {
    pub fn with_enabled(codes: &[FeatureCode]) -> Self {
        let v = codes.iter().map(|c| FeatureEntitlement::on(*c)).collect();
        Self(Resolution::Resolved(Arc::new(v)))
    }
}

/// A Number config value, which upstream carries as a decimal string.
fn config_decimal(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::Number(number) => number.to_string().parse().ok(),
        serde_json::Value::String(raw) => raw.trim().parse().ok(),
        _ => None,
    }
}

/// `resolve(..).is_enabled(code)` for a request that names a tenant, not an organization (the
/// customer portal). A failed tenant lookup is an error, never an open gate.
pub async fn portal_gate_open(
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

/// What a plan or license unlocks inside the code an edition ships. The edition picks the
/// implementation; its module list decides what code ships at all.
#[async_trait::async_trait]
pub trait Entitlements: Send + Sync {
    async fn resolve(&self, org_id: OrganizationId) -> ResolvedEntitlements;

    /// `resolve` for one tenant, which per-tenant gates and their conditions need. Editions
    /// without per-tenant grants answer for the organization.
    async fn resolve_tenant(
        &self,
        org_id: OrganizationId,
        _tenant_id: TenantId,
    ) -> ResolvedEntitlements {
        self.resolve(org_id).await
    }

    /// The cap of a limit gate (the tenant's, for a per-tenant gate), or `None` when uncapped.
    async fn limit(
        &self,
        org_id: OrganizationId,
        tenant_id: Option<TenantId>,
        gate: &FeatureGate,
    ) -> Option<u32> {
        match (gate.level, tenant_id) {
            (GateLevel::Tenant, Some(tenant_id)) => self.resolve_tenant(org_id, tenant_id).await,
            _ => self.resolve(org_id).await,
        }
        .limit_of(gate)
    }
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

    async fn resolve_tenant(
        &self,
        org_id: OrganizationId,
        tenant_id: TenantId,
    ) -> ResolvedEntitlements {
        self.inner
            .resolve_tenant(org_id, tenant_id)
            .await
            .with_bundled(&self.codes)
    }

    async fn limit(
        &self,
        org_id: OrganizationId,
        tenant_id: Option<TenantId>,
        gate: &FeatureGate,
    ) -> Option<u32> {
        self.inner.limit(org_id, tenant_id, gate).await
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

    async fn limit(
        &self,
        _org_id: OrganizationId,
        _tenant_id: Option<TenantId>,
        gate: &FeatureGate,
    ) -> Option<u32> {
        [
            quotas::PRODUCTION_TENANTS.code,
            quotas::INVOICING_ENTITIES.code,
        ]
        .contains(&gate.code)
        .then_some(1)
    }
}

/// Whether a gate is open for the tenant: per-tenant gates are decided by their conditions on it.
pub async fn tenant_gate_open(
    resolver: &dyn Entitlements,
    org_id: OrganizationId,
    tenant_id: TenantId,
    code: FeatureCode,
) -> bool {
    resolver
        .resolve_tenant(org_id, tenant_id)
        .await
        .is_enabled(code)
}

/// Refuses one more of a limit gate when `current_count`, counted by the caller, reaches the
/// cap; `Err` carries the cap.
pub async fn check_count(
    resolver: &dyn Entitlements,
    org_id: OrganizationId,
    tenant_id: Option<TenantId>,
    gate: &FeatureGate,
    current_count: usize,
) -> Result<(), u32> {
    match resolver.limit(org_id, tenant_id, gate).await {
        Some(limit) if current_count >= limit as usize => Err(limit),
        _ => Ok(()),
    }
}

/// Refuses a Production tenant past the org's `production_tenants` limit.
pub async fn ensure_production_tenant_allowed(
    resolver: &dyn Entitlements,
    store: &Store,
    org_id: OrganizationId,
) -> Result<(), tonic::Status> {
    use meteroid_store::repositories::OrganizationsInterface;

    let count = store
        .get_organizations_with_tenants_by_id(org_id)
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to count tenants: {e}")))?
        .tenants
        .iter()
        .filter(|t| t.environment == TenantEnvironmentEnum::Production && t.archived_at.is_none())
        .count();
    check_count(resolver, org_id, None, &quotas::PRODUCTION_TENANTS, count)
        .await
        .map_err(|limit| {
            tonic::Status::resource_exhausted(if store.instance.is_non_production() {
                "This instance's license is for non-production use: it allows no Production tenant"
                    .to_string()
            } else {
                format!("This organization can hold up to {limit} Production tenant(s)")
            })
        })
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

    let count = store
        .list_invoicing_entities(tenant_id)
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to count invoicing entities: {e}")))?
        .len();
    check_count(
        resolver,
        org_id,
        Some(tenant_id),
        &quotas::INVOICING_ENTITIES,
        count,
    )
    .await
    .map_err(|limit| {
        tonic::Status::resource_exhausted(format!(
            "This organization can hold up to {limit} invoicing entit{} per tenant",
            if limit == 1 { "y" } else { "ies" }
        ))
    })
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
pub const USAGE_BREAKDOWN_NOT_IN_PLAN: &str =
    "Usage breakdowns are not enabled for this organization.";

/// Refuses setting a metric's breakdown key without the gate, unless it is the key `stored`.
/// Keeping or removing the key stays open, so a metric stays editable after a downgrade.
pub async fn ensure_usage_breakdown_allowed(
    resolver: &dyn Entitlements,
    org_id: OrganizationId,
    requested: Option<&str>,
    stored: Option<&str>,
) -> Result<(), tonic::Status> {
    let requested = normalize_usage_breakdown_key(requested.map(str::to_owned));
    let changes = requested.is_some() && requested.as_deref() != stored;
    ensure_change_allowed(
        resolver,
        org_id,
        gates::USAGE_BREAKDOWN,
        changes,
        USAGE_BREAKDOWN_NOT_IN_PLAN,
    )
    .await
}

pub const QUOTES_FULL_NOT_IN_PLAN: &str = "Advanced quoting is not enabled for this organization.";

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
    let open = portal_gate_open(resolver, store, tenant_id, gates::PORTAL_SELF_SERVICE)
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

pub const AI_MODULE_OFF: &str =
    "The AI module is off for this tenant: turn it on in Settings, under AI.";

/// Whether the AI module runs for the tenant: its organization holds the gate and the tenant
/// turned the module on.
pub async fn ai_module_open(
    resolver: &dyn Entitlements,
    store: &Store,
    tenant_id: TenantId,
) -> meteroid_store::StoreResult<bool> {
    use meteroid_store::repositories::tenant_modules::{AI_MODULE, TenantModulesInterface};
    use meteroid_store::repositories::tenants::TenantInterface;

    let tenant = store.find_tenant_by_id(tenant_id).await?;
    Ok(
        tenant_gate_open(resolver, tenant.organization.id, tenant_id, gates::AI).await
            && store.is_tenant_module_enabled(tenant_id, AI_MODULE).await?,
    )
}

pub async fn ensure_ai_module_open(
    resolver: &dyn Entitlements,
    store: &Store,
    tenant_id: TenantId,
) -> Result<(), tonic::Status> {
    match ai_module_open(resolver, store, tenant_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(tonic::Status::failed_precondition(AI_MODULE_OFF)),
        Err(e) => Err(tonic::Status::internal(format!(
            "Failed to resolve the AI module: {e}"
        ))),
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
    use serde_json::json;

    const AUDIT_LOG: FeatureCode = FeatureCode::new("audit_log");
    const MFA_POLICY: FeatureCode = FeatureCode::new("mfa_policy");
    const ANALYTICS: FeatureGate = FeatureGate::new(FeatureCode::new("analytics")).per_tenant();

    const CORE_GATES: [FeatureCode; 17] = [
        gates::QUOTES_FULL,
        gates::CREDIT_PRICING,
        gates::SCOPED_CREDIT,
        gates::USAGE_COSTS,
        gates::EMAIL_CUSTOMIZATION,
        gates::PORTAL_SELF_SERVICE,
        gates::PORTAL_EMBEDDING,
        gates::PORTAL_THEMING,
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

    fn entry(code: &str) -> FeatureEntitlement {
        FeatureEntitlement {
            code: code.to_string(),
            enabled: true,
            limit: None,
            remaining: None,
            config: None,
        }
    }

    fn resolved(entries: Vec<FeatureEntitlement>) -> ResolvedEntitlements {
        ResolvedEntitlements::from_entitlements(entries)
    }

    fn with_config(code: &str, value: serde_json::Value) -> ResolvedEntitlements {
        resolved(vec![FeatureEntitlement {
            enabled: false,
            config: Some(value),
            ..entry(code)
        }])
    }

    #[test]
    fn a_code_reads_as_the_string_it_is_stored_as() {
        assert_eq!(gates::WHITELABEL.to_string(), "whitelabel");
        assert_eq!(gates::CUSTOMER_SIGNALS.as_str(), "customer_signals");
        assert_eq!(format!("{:?}", gates::ADMIN_SANDBOXES), "admin_sandboxes");
        assert_eq!(FeatureCode::new("rbac"), gates::RBAC);
    }

    #[test]
    fn declarations_carry_kind_level_unit_and_their_outage_and_default_values() {
        assert_eq!(
            FeatureGate::new(AUDIT_LOG),
            FeatureGate {
                code: AUDIT_LOG,
                kind: GateKind::Boolean,
                unit: None,
                level: GateLevel::Organization,
                outage: GateValue::Off,
                default: GateValue::Off,
                bundled: false,
            }
        );
        assert_eq!(ANALYTICS.level, GateLevel::Tenant);
        assert_eq!(ANALYTICS.kind, GateKind::Boolean);

        for cap in [
            quotas::ALERT_DEFINITIONS,
            quotas::PRODUCTION_TENANTS,
            quotas::INVOICING_ENTITIES,
        ] {
            assert_eq!(cap.kind, GateKind::Limit, "{:?}", cap.code);
            assert_eq!(cap.outage, GateValue::Unlimited, "{:?}", cap.code);
            assert_eq!(cap.default, GateValue::Unlimited, "{:?}", cap.code);
        }
        assert_eq!(quotas::ALERT_DEFINITIONS.level, GateLevel::Tenant);
        assert_eq!(quotas::PRODUCTION_TENANTS.level, GateLevel::Organization);
        assert_eq!(quotas::INVOICING_ENTITIES.level, GateLevel::Tenant);

        let rate = quotas::USAGE_COSTS_RATE_LIMIT;
        assert_eq!(rate.kind, GateKind::Number);
        assert_eq!(rate.unit, Some("requests per minute"));
        assert_eq!(rate.outage, GateValue::Off);
    }

    #[test]
    fn an_outage_reads_the_declared_outage_values() {
        let outage = ResolvedEntitlements::outage();
        assert!(outage.is_outage());
        assert!(!outage.enabled(&ANALYTICS));
        assert_eq!(outage.limit_of(&quotas::PRODUCTION_TENANTS), None);
        assert_eq!(outage.number(&quotas::USAGE_COSTS_RATE_LIMIT), None);
        assert!(!outage.is_enabled(AUDIT_LOG));
        assert!(!ResolvedEntitlements::open().is_outage());
    }

    #[test]
    fn a_gate_unknown_upstream_reads_its_default() {
        let empty = resolved(vec![]);
        assert!(!empty.enabled(&ANALYTICS));
        assert_eq!(empty.limit_of(&quotas::INVOICING_ENTITIES), None);
        assert_eq!(empty.value(&quotas::USAGE_COSTS_RATE_LIMIT), GateValue::Off);
    }

    #[test]
    fn limits_read_a_number_an_amount_unlimited_or_off() {
        let cap = &quotas::INVOICING_ENTITIES;
        let read = |e: FeatureEntitlement| resolved(vec![e]).limit_of(cap);
        let code = INVOICING_ENTITIES_CODE;
        assert_eq!(
            read(FeatureEntitlement {
                limit: Some(Decimal::from(4)),
                ..entry(code)
            }),
            Some(4)
        );
        assert_eq!(read(entry(code)), None);
        assert_eq!(
            read(FeatureEntitlement {
                enabled: false,
                ..entry(code)
            }),
            Some(0)
        );
        assert_eq!(
            read(FeatureEntitlement {
                config: Some(json!("2.9")),
                ..entry(code)
            }),
            Some(2)
        );
        assert_eq!(
            read(FeatureEntitlement {
                config: Some(json!("-1")),
                ..entry(code)
            }),
            Some(0)
        );
        assert_eq!(ResolvedEntitlements::open().limit_of(cap), None);
    }

    #[test]
    fn a_creation_cap_reads_a_number_config_or_a_metered_limit() {
        let number =
            |value| with_config(INVOICING_ENTITIES_CODE, value).limit(INVOICING_ENTITIES_CODE);
        assert_eq!(number(json!("3")), Some(3));
        assert_eq!(number(json!(2)), Some(2));
        assert_eq!(number(json!("2.9")), Some(2));
        assert_eq!(number(json!("-1")), Some(0));
        assert_eq!(number(json!({"production": 1})), None);

        let metered = resolved(vec![FeatureEntitlement {
            limit: Some(Decimal::from(4)),
            ..entry(PRODUCTION_TENANTS_CODE)
        }]);
        assert_eq!(metered.limit(PRODUCTION_TENANTS_CODE), Some(4));
        assert_eq!(metered.limit(INVOICING_ENTITIES_CODE), None);
    }

    #[test]
    fn usage_costs_rate_limit_reads_a_positive_number_and_defaults_otherwise() {
        let limit =
            |value| with_config(USAGE_COSTS_RATE_LIMIT_CODE, value).usage_costs_rate_limit();
        assert_eq!(limit(json!("1200")), Some(1200));
        assert_eq!(limit(json!(" 90.7 ")), Some(90));
        assert_eq!(limit(json!(300)), Some(300));
        assert_eq!(limit(json!(90.7)), Some(90));
        assert_eq!(limit(json!("0")), None);
        assert_eq!(limit(json!("-5")), None);
        assert_eq!(limit(json!({"production": 10})), None);
        assert_eq!(
            with_config("other", json!("10")).usage_costs_rate_limit(),
            None
        );
        assert_eq!(ResolvedEntitlements::open().usage_costs_rate_limit(), None);
        assert_eq!(
            ResolvedEntitlements::outage().usage_costs_rate_limit(),
            None
        );
    }

    #[test]
    fn the_listed_gates_are_exactly_the_declared_ones() {
        let gates = [
            ANALYTICS,
            FeatureGate::new(AUDIT_LOG),
            FeatureGate::new(MFA_POLICY),
        ];
        let out = resolved(vec![entry("audit_log"), entry("whitelabel")]).all_entitlements(&gates);
        let listed: Vec<(&str, bool)> = out.iter().map(|e| (e.code.as_str(), e.enabled)).collect();
        assert_eq!(
            listed,
            [
                ("analytics", false),
                ("audit_log", true),
                ("mfa_policy", false)
            ]
        );
    }

    #[test]
    fn map_lookup_respects_enabled_open_and_closed() {
        let r = resolved(vec![
            entry("audit_log"),
            FeatureEntitlement {
                enabled: false,
                ..entry("mfa_policy")
            },
        ]);
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(!r.is_enabled(MFA_POLICY));
        assert!(!resolved(vec![]).is_enabled(AUDIT_LOG));
        assert!(ResolvedEntitlements::open().is_enabled(FeatureCode::new("a_gate_added_later")));
        assert!(!ResolvedEntitlements::outage().is_enabled(AUDIT_LOG));
    }

    struct PerTenant;

    #[async_trait::async_trait]
    impl Entitlements for PerTenant {
        async fn resolve(&self, _: OrganizationId) -> ResolvedEntitlements {
            resolved(vec![FeatureEntitlement {
                limit: Some(Decimal::from(5)),
                ..entry(INVOICING_ENTITIES_CODE)
            }])
        }

        async fn resolve_tenant(&self, _: OrganizationId, _: TenantId) -> ResolvedEntitlements {
            resolved(vec![
                FeatureEntitlement {
                    limit: Some(Decimal::from(2)),
                    ..entry(INVOICING_ENTITIES_CODE)
                },
                entry("analytics"),
            ])
        }
    }

    #[tokio::test]
    async fn a_per_tenant_gate_reads_the_tenant_and_the_caller_passes_its_count() {
        let (org, tenant) = (OrganizationId::new(), TenantId::new());
        let cap = &quotas::INVOICING_ENTITIES;
        assert_eq!(PerTenant.limit(org, Some(tenant), cap).await, Some(2));
        assert_eq!(PerTenant.limit(org, None, cap).await, Some(5));
        assert_eq!(
            PerTenant
                .limit(org, Some(tenant), &quotas::PRODUCTION_TENANTS)
                .await,
            None
        );
        assert_eq!(
            check_count(&PerTenant, org, Some(tenant), cap, 1).await,
            Ok(())
        );
        assert_eq!(
            check_count(&PerTenant, org, Some(tenant), cap, 2).await,
            Err(2)
        );
        assert!(tenant_gate_open(&PerTenant, org, tenant, ANALYTICS.code).await);
        assert!(!PerTenant.resolve(org).await.enabled(&ANALYTICS));
    }

    #[tokio::test]
    async fn oss_closes_every_gate_and_holds_one_production_tenant_and_entity() {
        let (org, tenant) = (OrganizationId::new(), TenantId::new());
        let r = OssEntitlements.resolve(org).await;
        for code in CORE_GATES {
            assert!(!r.is_enabled(code), "{code:?}");
        }
        assert!(!r.is_outage());
        assert_eq!(
            OssEntitlements
                .limit(org, Some(tenant), &quotas::ALERT_DEFINITIONS)
                .await,
            None
        );
        assert_eq!(
            OssEntitlements
                .limit(org, Some(tenant), &quotas::INVOICING_ENTITIES)
                .await,
            Some(1)
        );
        assert_eq!(
            OssEntitlements
                .limit(org, None, &quotas::PRODUCTION_TENANTS)
                .await,
            Some(1)
        );
    }

    #[test]
    fn a_bundled_gate_opens_whatever_the_license_lists() {
        let r = resolved(vec![
            FeatureEntitlement {
                enabled: false,
                ..entry("audit_log")
            },
            entry("mfa_policy"),
        ])
        .with_bundled(&[AUDIT_LOG]);
        assert!(r.is_enabled(AUDIT_LOG));
        assert!(r.is_enabled(MFA_POLICY));
        assert!(!r.enabled(&ANALYTICS));
        assert!(
            resolved(vec![])
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
        let (org, tenant) = (OrganizationId::new(), TenantId::new());
        let gates = [
            FeatureGate::bundled(AUDIT_LOG),
            FeatureGate::new(MFA_POLICY),
        ];
        let resolver = with_bundled_gates(Arc::new(PerTenant), &gates);
        assert!(resolver.resolve(org).await.is_enabled(AUDIT_LOG));
        assert!(!resolver.resolve(org).await.is_enabled(MFA_POLICY));
        let t = resolver.resolve_tenant(org, tenant).await;
        assert!(t.is_enabled(AUDIT_LOG) && t.enabled(&ANALYTICS));
        assert_eq!(
            resolver
                .limit(org, Some(tenant), &quotas::INVOICING_ENTITIES)
                .await,
            Some(2)
        );
        let oss = with_bundled_gates(Arc::new(OssEntitlements), &gates);
        assert_eq!(
            oss.limit(org, None, &quotas::PRODUCTION_TENANTS).await,
            Some(1)
        );
    }
}

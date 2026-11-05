//! Editions for tests, deciding what the modules under test are entitled to.
use std::sync::Arc;

use common_domain::ids::OrganizationId;
use meteroid::module::{ApiModule, Edition, Modules};
use meteroid_app::features::{Entitlements, FeatureGate, ResolvedEntitlements, gates};
use meteroid_store::Store;

/// Every gate open and nothing capped, as a legacy license grants.
pub struct OpenEdition;

impl Edition for OpenEdition {
    fn entitlements(&self, _store: &Store) -> Arc<dyn Entitlements> {
        Arc::new(OpenEntitlements)
    }
}

pub struct OpenEntitlements;

#[async_trait::async_trait]
impl Entitlements for OpenEntitlements {
    async fn resolve(&self, _org_id: OrganizationId) -> ResolvedEntitlements {
        ResolvedEntitlements::open()
    }

    async fn alert_definition_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        None
    }

    async fn production_tenant_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        None
    }

    async fn invoicing_entity_limit(&self, _org_id: OrganizationId) -> Option<u32> {
        None
    }
}

static CORE_TESTED: [FeatureGate; 7] = [
    FeatureGate::bundled(gates::CREDIT_PRICING),
    FeatureGate::bundled(gates::SCOPED_CREDIT),
    FeatureGate::bundled(gates::USAGE_COSTS),
    FeatureGate::bundled(gates::EINVOICING),
    FeatureGate::bundled(gates::EMAIL_CUSTOMIZATION),
    FeatureGate::bundled(gates::PORTAL_SELF_SERVICE),
    FeatureGate::bundled(gates::QUOTE_MULTIPLE_SIGNERS),
];

struct CoreTestedGates;

#[async_trait::async_trait]
impl ApiModule for CoreTestedGates {
    fn id(&self) -> &'static str {
        "test-core-tested-gates"
    }

    fn feature_gates(&self) -> &'static [FeatureGate] {
        &CORE_TESTED
    }
}

/// The default test edition: OSS with the gates of the engines core tests cover open.
pub fn core_test_modules() -> Modules {
    Modules {
        api: vec![Arc::new(CoreTestedGates)],
        ..Modules::core()
    }
}

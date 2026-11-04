//! Editions for tests, deciding what the modules under test are entitled to.
use std::sync::Arc;

use common_domain::ids::OrganizationId;
use meteroid::module::Edition;
use meteroid_app::features::{Entitlements, ResolvedEntitlements};
use meteroid_store::Store;

/// Every gate open and nothing capped, as a legacy license grants. The default `Modules::core()`
/// edition closes every gate, which is what tests of a closed gate use.
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

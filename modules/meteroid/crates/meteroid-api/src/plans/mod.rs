use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

use meteroid_app::features::Entitlements;

mod error;
pub(crate) mod mapping;
mod service;

pub struct PlanServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> PlansServiceServer<PlanServiceComponents> {
    let inner = PlanServiceComponents {
        store,
        feature_resolver,
    };
    PlansServiceServer::new(inner)
}

impl PlanServiceComponents {
    /// The plan as the editor reads it, with its trial.
    pub(crate) async fn with_effective_trial(
        &self,
        tenant_id: common_domain::ids::TenantId,
        plan: meteroid_store::domain::PlanWithVersion,
    ) -> Result<meteroid_grpc::meteroid::api::plans::v1::PlanWithVersion, tonic::Status> {
        use meteroid_store::repositories::PlansInterface;
        let trial = self
            .store
            .plan_trial(tenant_id, plan.plan.id)
            .await
            .map_err(Into::<crate::plans::error::PlanApiError>::into)?;
        let mut out = mapping::plans::PlanWithVersionWrapper::from(plan).0;
        out.trial = trial.map(mapping::plans::plan_trial_to_proto);
        Ok(out)
    }
}

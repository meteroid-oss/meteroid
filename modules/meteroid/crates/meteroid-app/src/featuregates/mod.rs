use std::sync::Arc;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::features::v1::EntitlementGrant as ProtoGrant;
use meteroid_grpc::meteroid::api::features::v1::features_service_server::{
    FeaturesService, FeaturesServiceServer,
};
use meteroid_grpc::meteroid::api::features::v1::{
    FeatureEntitlement, GetFeaturesRequest, GetFeaturesResponse,
};
use meteroid_store::Store;
use meteroid_store::repositories::TenantInterface;
use tonic::{Request, Response, Status};

use crate::features::{EntitlementGrant, Entitlements, FeatureGate};

/// Lists the entitlements of `gates`, the gates the edition's modules declare.
pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
    gates: Arc<[FeatureGate]>,
) -> FeaturesServiceServer<FeaturesServiceComponents> {
    FeaturesServiceServer::new(FeaturesServiceComponents {
        store,
        feature_resolver,
        gates,
    })
}

pub struct FeaturesServiceComponents {
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
    gates: Arc<[FeatureGate]>,
}

#[tonic::async_trait]
impl FeaturesService for FeaturesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_features(
        &self,
        request: Request<GetFeaturesRequest>,
    ) -> Result<Response<GetFeaturesResponse>, Status> {
        let organization = request.organization()?;
        let resolved = self.feature_resolver.resolve(organization).await;

        // Resolved per tenant: a gate the org doesn't own can still be open in a
        // non-production tenant, and the client needs to tell that apart from owning it.
        let environment = match request.tenant() {
            Ok(tenant_id) => self
                .store
                .find_tenant_by_id(tenant_id)
                .await
                .ok()
                .map(|t| t.tenant.environment),
            Err(_) => None,
        };

        let entitlements = resolved
            .all_entitlements(environment.as_ref(), &self.gates)
            .into_iter()
            .map(|e| FeatureEntitlement {
                code: e.code.to_string(),
                enabled: e.enabled,
                limit: e.limit.map(|x| x.to_string()),
                remaining: e.remaining.map(|x| x.to_string()),
                granted_by: match e.grant {
                    Some(EntitlementGrant::Entitlement) => ProtoGrant::Entitlement,
                    Some(EntitlementGrant::Environment) => ProtoGrant::Environment,
                    None => ProtoGrant::Unspecified,
                } as i32,
            })
            .collect();

        let invoicing_entity_limit = self
            .feature_resolver
            .invoicing_entity_limit(organization)
            .await;

        let production_tenant_limit = self
            .feature_resolver
            .production_tenant_limit(organization)
            .await;

        Ok(Response::new(GetFeaturesResponse {
            entitlements,
            invoicing_entity_limit,
            production_tenant_limit,
        }))
    }
}

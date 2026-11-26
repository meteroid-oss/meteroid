use std::sync::Arc;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::features::v1::EntitlementGrant as ProtoGrant;
use meteroid_grpc::meteroid::api::features::v1::features_service_server::{
    FeaturesService, FeaturesServiceServer,
};
use meteroid_grpc::meteroid::api::features::v1::{
    FeatureEntitlement, GetFeaturesRequest, GetFeaturesResponse,
};
use tonic::{Request, Response, Status};

use crate::features::{Entitlements, FeatureGate, quotas};

/// Lists the entitlements of `gates`, the gates the edition's modules declare.
pub fn service(
    feature_resolver: Arc<dyn Entitlements>,
    gates: Arc<[FeatureGate]>,
) -> FeaturesServiceServer<FeaturesServiceComponents> {
    FeaturesServiceServer::new(FeaturesServiceComponents {
        feature_resolver,
        gates,
    })
}

pub struct FeaturesServiceComponents {
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
        // Per tenant when the request names one: per-tenant gates follow their conditions on it.
        let tenant = request.tenant().ok();
        let resolved = match tenant {
            Some(tenant_id) => {
                self.feature_resolver
                    .resolve_tenant(organization, tenant_id)
                    .await
            }
            None => self.feature_resolver.resolve(organization).await,
        };

        let entitlements = resolved
            .all_entitlements(&self.gates)
            .into_iter()
            .map(|e| FeatureEntitlement {
                code: e.code.to_string(),
                enabled: e.enabled,
                limit: e.limit.map(|x| x.to_string()),
                remaining: e.remaining.map(|x| x.to_string()),
                granted_by: if e.enabled {
                    ProtoGrant::Entitlement
                } else {
                    ProtoGrant::Unspecified
                } as i32,
            })
            .collect();

        let invoicing_entity_limit = self
            .feature_resolver
            .limit(organization, tenant, &quotas::INVOICING_ENTITIES)
            .await;

        let production_tenant_limit = self
            .feature_resolver
            .limit(organization, None, &quotas::PRODUCTION_TENANTS)
            .await;

        Ok(Response::new(GetFeaturesResponse {
            entitlements,
            invoicing_entity_limit,
            production_tenant_limit,
        }))
    }
}

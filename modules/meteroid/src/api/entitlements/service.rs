use crate::api::entitlements::error::EntitlementApiError;
use crate::api::entitlements::mapping;
use crate::api::utils::PaginationExt;
use common_domain::ids::{
    AddOnId, CustomerId, EntitlementId, FeatureId, PlanVersionId, ProductId, QuoteId,
    SubscriptionId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::entitlements::v1::{
    BatchCreateEntitlementsRequest, BatchCreateEntitlementsResponse, CreateEntitlementRequest,
    CreateEntitlementResponse, CreateFeatureRequest, CreateFeatureResponse,
    DeleteEntitlementRequest, DeleteEntitlementResponse, GetEffectiveEntitlementsRequest,
    GetEffectiveEntitlementsResponse, GetEntitlementRequest, GetEntitlementResponse,
    GetFeatureRequest, GetFeatureResponse, GetResolvedForAddOnRequest,
    GetResolvedForPlanVersionRequest, GetResolvedForProductRequest, GetResolvedForQuoteRequest,
    GetResolvedForSelectionRequest, GetResolvedForSubscriptionRequest, GetResolvedResponse,
    ListEntitlementsByEntityRequest, ListEntitlementsByEntityResponse,
    ListEntitlementsByFeatureRequest, ListEntitlementsByFeatureResponse, ListFeaturesRequest,
    ListFeaturesResponse, SetFeatureStatusRequest, SetFeatureStatusResponse,
    UpdateEntitlementRequest, UpdateEntitlementResponse, UpdateFeatureRequest,
    UpdateFeatureResponse, entitlements_service_server::EntitlementsService,
};
use meteroid_store::domain::entitlements::FeatureEntitlementSpec;
use meteroid_store::domain::{EntitlementNew, EntitlementUpdate, FeatureNew, FeatureUpdate};
use meteroid_store::repositories::EntitlementsInterface;
use meteroid_store::repositories::entitlements::ResolveTarget;
use tonic::{Request, Response, Status};

use super::EntitlementsComponents;

#[tonic::async_trait]
impl EntitlementsService for EntitlementsComponents {
    #[tracing::instrument(skip_all)]
    async fn create_feature(
        &self,
        request: Request<CreateFeatureRequest>,
    ) -> Result<Response<CreateFeatureResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let inner = request.into_inner();

        let feature_type = mapping::feature_type_from_proto(inner.feature_type)?;
        let product_id = mapping::product_id_from_proto(inner.product_id)?;

        let entitlement = inner
            .entitlement
            .map(|spec| -> Result<FeatureEntitlementSpec, Status> {
                Ok(FeatureEntitlementSpec {
                    entity: mapping::entity_from_proto(spec.entity.as_ref())?,
                    value: mapping::entitlement_value_from_proto(spec.value)?,
                })
            })
            .transpose()?;

        let mut feature = self
            .store
            .create_feature(FeatureNew {
                tenant_id,
                product_id,
                name: inner.name,
                description: inner.description,
                feature_type,
                created_by: actor,
                entitlement,
            })
            .await
            .map_err(EntitlementApiError::from)?;

        let entitlement_proto = feature
            .entitlement
            .take()
            .map(mapping::entitlement_to_proto);

        Ok(Response::new(CreateFeatureResponse {
            feature: Some(mapping::feature_to_proto(feature)),
            entitlement: entitlement_proto,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_feature(
        &self,
        request: Request<GetFeatureRequest>,
    ) -> Result<Response<GetFeatureResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = FeatureId::from_proto(&inner.id)?;

        let feature = self
            .store
            .get_feature(id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetFeatureResponse {
            feature: Some(mapping::feature_to_proto(feature)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_features(
        &self,
        request: Request<ListFeaturesRequest>,
    ) -> Result<Response<ListFeaturesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let pagination = inner.pagination.into_domain();

        // proto `repeated` defaults to an empty Vec when absent; map [] → None so
        // an empty selection is treated as "no filter" (same as the old "All" tab).
        let statuses: Option<Vec<meteroid_store::domain::enums::FeatureStatusEnum>> =
            if inner.statuses.is_empty() {
                None
            } else {
                Some(
                    inner
                        .statuses
                        .into_iter()
                        .map(mapping::feature_status_from_proto)
                        .collect::<Result<Vec<_>, _>>()?,
                )
            };

        let product_id = mapping::product_id_from_proto(inner.product_id)?;

        let result = self
            .store
            .list_features(tenant_id, pagination, statuses, product_id, inner.search)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListFeaturesResponse {
            features: result
                .items
                .into_iter()
                .map(mapping::feature_to_proto)
                .collect(),
            pagination_meta: inner
                .pagination
                .into_response(result.total_pages, result.total_results),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_feature(
        &self,
        request: Request<UpdateFeatureRequest>,
    ) -> Result<Response<UpdateFeatureResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = FeatureId::from_proto(&inner.id)?;

        let description = inner
            .description
            .map(|d| if d.is_empty() { None } else { Some(d) });

        // clear_product_id wins over product_id — caller can detach with one flag.
        let product_id = if inner.clear_product_id {
            Some(None)
        } else {
            inner
                .product_id
                .map(|p| common_domain::ids::ProductId::from_proto(&p))
                .transpose()?
                .map(Some)
        };

        let feature = self
            .store
            .update_feature(
                id,
                tenant_id,
                FeatureUpdate {
                    name: inner.name,
                    description,
                    product_id,
                },
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(UpdateFeatureResponse {
            feature: Some(mapping::feature_to_proto(feature)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_feature_status(
        &self,
        request: Request<SetFeatureStatusRequest>,
    ) -> Result<Response<SetFeatureStatusResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = FeatureId::from_proto(&inner.id)?;
        let status = mapping::feature_status_from_proto(inner.status)?;

        self.store
            .set_feature_status(id, tenant_id, status)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(SetFeatureStatusResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn create_entitlement(
        &self,
        request: Request<CreateEntitlementRequest>,
    ) -> Result<Response<CreateEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let inner = request.into_inner();

        let feature_id = FeatureId::from_proto(&inner.feature_id)?;
        let entity = mapping::entity_from_proto(inner.entity.as_ref())?;
        let value = mapping::entitlement_value_from_proto(inner.value)?;

        let entitlement = self
            .store
            .create_entitlement(EntitlementNew {
                tenant_id,
                feature_id,
                entity,
                value,
                created_by: actor,
            })
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(CreateEntitlementResponse {
            entitlement: Some(mapping::entitlement_to_proto(entitlement)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_entitlement(
        &self,
        request: Request<GetEntitlementRequest>,
    ) -> Result<Response<GetEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = EntitlementId::from_proto(&inner.id)?;

        let entitlement = self
            .store
            .get_entitlement(id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetEntitlementResponse {
            entitlement: Some(mapping::entitlement_to_proto(entitlement)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_entitlements_by_entity(
        &self,
        request: Request<ListEntitlementsByEntityRequest>,
    ) -> Result<Response<ListEntitlementsByEntityResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let entity = mapping::entity_from_proto(inner.entity.as_ref())?;

        let entitlements = self
            .store
            .list_entitlements_by_entity(entity, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListEntitlementsByEntityResponse {
            entitlements: entitlements
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_entitlements_by_feature(
        &self,
        request: Request<ListEntitlementsByFeatureRequest>,
    ) -> Result<Response<ListEntitlementsByFeatureResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let feature_id = FeatureId::from_proto(&inner.feature_id)?;

        let entitlements = self
            .store
            .list_entitlements_by_feature(feature_id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListEntitlementsByFeatureResponse {
            entitlements: entitlements
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_entitlement(
        &self,
        request: Request<UpdateEntitlementRequest>,
    ) -> Result<Response<UpdateEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = EntitlementId::from_proto(&inner.id)?;

        let value = inner
            .value
            .map(|v| mapping::entitlement_value_from_proto(Some(v)))
            .transpose()?;

        let entitlement = self
            .store
            .update_entitlement(id, tenant_id, EntitlementUpdate { value })
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(UpdateEntitlementResponse {
            entitlement: Some(mapping::entitlement_to_proto(entitlement)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_entitlement(
        &self,
        request: Request<DeleteEntitlementRequest>,
    ) -> Result<Response<DeleteEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = EntitlementId::from_proto(&inner.id)?;

        self.store
            .delete_entitlement(id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(DeleteEntitlementResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn get_effective_entitlements(
        &self,
        request: Request<GetEffectiveEntitlementsRequest>,
    ) -> Result<Response<GetEffectiveEntitlementsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto(&inner.customer_id)?;

        let resolved = self
            .services
            .get_effective_entitlements(customer_id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetEffectiveEntitlementsResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::effective_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_product(
        &self,
        request: Request<GetResolvedForProductRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let product_id = ProductId::from_proto(&inner.product_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(&mut conn, tenant_id, ResolveTarget::Product(product_id))
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_add_on(
        &self,
        request: Request<GetResolvedForAddOnRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let add_on_id = AddOnId::from_proto(&inner.add_on_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(&mut conn, tenant_id, ResolveTarget::AddOn(add_on_id))
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_plan_version(
        &self,
        request: Request<GetResolvedForPlanVersionRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&inner.plan_version_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(
                &mut conn,
                tenant_id,
                ResolveTarget::PlanVersion(plan_version_id),
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_subscription(
        &self,
        request: Request<GetResolvedForSubscriptionRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(
                &mut conn,
                tenant_id,
                ResolveTarget::Subscription(subscription_id),
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_quote(
        &self,
        request: Request<GetResolvedForQuoteRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(&mut conn, tenant_id, ResolveTarget::Quote(quote_id))
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resolved_entitlements_for_selection(
        &self,
        request: Request<GetResolvedForSelectionRequest>,
    ) -> Result<Response<GetResolvedResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&inner.plan_version_id)?;
        let add_on_ids = inner
            .add_on_ids
            .iter()
            .map(AddOnId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;
        let extra_product_ids = inner
            .extra_product_ids
            .iter()
            .map(ProductId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(EntitlementApiError::from)?;
        let resolved = self
            .store
            .resolve_for_entity(
                &mut conn,
                tenant_id,
                ResolveTarget::Selection {
                    plan_version_id,
                    add_on_ids,
                    extra_product_ids,
                },
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(mapping::resolved_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn batch_create_entitlements(
        &self,
        request: Request<BatchCreateEntitlementsRequest>,
    ) -> Result<Response<BatchCreateEntitlementsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let inner = request.into_inner();

        let entity = mapping::entity_from_proto(inner.entity.as_ref())?;
        let specs = inner
            .specs
            .into_iter()
            .map(mapping::entitlement_spec_from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let created = self
            .store
            .batch_create_entitlements(tenant_id, entity, specs, actor)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(BatchCreateEntitlementsResponse {
            created: created
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
        }))
    }
}

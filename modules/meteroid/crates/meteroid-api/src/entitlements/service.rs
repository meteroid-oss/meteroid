use crate::entitlements::error::EntitlementApiError;
use crate::entitlements::mapping;
use common_domain::ids::{
    AddOnId, CustomerId, EntitlementId, FeatureId, PlanVersionId, ProductId, QuoteId, ResourceId,
    ResourceTypeId, SubscriptionId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::entitlements::v1::{
    BatchCreateEntitlementsRequest, BatchCreateEntitlementsResponse, CheckEntitlementRequest,
    CheckEntitlementResponse, CreateEntitlementRequest, CreateEntitlementResponse,
    CreateFeatureRequest, CreateFeatureResponse, DeleteEntitlementRequest,
    DeleteEntitlementResponse, FeatureOrder, GetEffectiveEntitlementsRequest,
    GetEffectiveEntitlementsResponse, GetEntitlementRequest, GetEntitlementResponse,
    GetEntitlementSnapshotRequest, GetEntitlementSnapshotResponse, GetEntitlementSnapshotsRequest,
    GetEntitlementSnapshotsResponse, GetFeatureCodeHolderRequest, GetFeatureCodeHolderResponse,
    GetFeatureRequest, GetFeatureResponse, GetResolvedForAddOnRequest,
    GetResolvedForPlanVersionRequest, GetResolvedForProductRequest, GetResolvedForQuoteRequest,
    GetResolvedForSelectionRequest, GetResolvedForSubscriptionRequest, GetResolvedResponse,
    ListCreditPaidFeaturesRequest, ListCreditPaidFeaturesResponse, ListEntitlementHistoryRequest,
    ListEntitlementHistoryResponse, ListEntitlementsByEntityRequest,
    ListEntitlementsByEntityResponse, ListEntitlementsByFeatureRequest,
    ListEntitlementsByFeatureResponse, ListFeaturesRequest, ListFeaturesResponse,
    ReportEntitlementUsageRequest, ReportEntitlementUsageResponse, SetFeatureStatusRequest,
    SetFeatureStatusResponse, UpdateEntitlementRequest, UpdateEntitlementResponse,
    UpdateFeatureRequest, UpdateFeatureResponse, entitlements_service_server::EntitlementsService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain::entitlement_checks::effective_usage_warnings;
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::resources::ResourceInput;
use meteroid_store::domain::{EntitlementNew, FeatureListOrder, FeatureNew, FeatureUpdate};
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let kind = mapping::kind_from_proto(inner.kind, inner.value, inner.limit)?;
        let product_id = mapping::product_id_from_proto(inner.product_id)?;

        let code = validate_feature_code(&inner.code)?;

        let default = inner
            .entitlement
            .map(|v| mapping::grant_value_from_proto(Some(v)))
            .transpose()?;

        let feature = self
            .store
            .create_feature(
                actor,
                FeatureNew {
                    tenant_id,
                    product_id,
                    name: inner.name,
                    code,
                    description: inner.description,
                    kind,
                    unit: inner.unit,
                    level_type_id: ResourceTypeId::from_proto_opt(inner.level_resource_type_id)?,
                    policy: inner.policy.map(mapping::policy_from_proto).transpose()?,
                    default,
                },
            )
            .await
            .map_err(EntitlementApiError::from)?;

        let feature = mapping::feature_to_proto(feature);
        Ok(Response::new(CreateFeatureResponse {
            entitlement: feature.entitlement.clone(),
            feature: Some(feature),
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
        let order = match inner.order() {
            FeatureOrder::Unspecified => FeatureListOrder::Name,
            FeatureOrder::Product => FeatureListOrder::Product,
        };

        // An absent `repeated` is an empty Vec; archived features are only listed when asked for.
        let statuses = if inner.statuses.is_empty() {
            vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled]
        } else {
            inner
                .statuses
                .into_iter()
                .map(mapping::feature_status_from_proto)
                .collect::<Result<Vec<_>, _>>()?
        };

        let product_id = mapping::product_id_from_proto(inner.product_id)?;
        let level = inner
            .level_resource_type_id
            .map(|id| {
                (!id.is_empty())
                    .then(|| ResourceTypeId::from_proto(&id))
                    .transpose()
            })
            .transpose()?;

        let result = self
            .store
            .list_features(
                tenant_id,
                pagination,
                Some(statuses),
                product_id,
                inner.search,
                level,
                order,
            )
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

        // Only touch options when the caller opts in, so a normal metadata edit leaves them intact.
        let select_options = inner.update_config_options.then_some(inner.config_options);

        let level_type_id = if inner.clear_level_resource_type_id {
            Some(None)
        } else {
            ResourceTypeId::from_proto_opt(inner.level_resource_type_id)?.map(Some)
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
                    unit: if inner.clear_unit {
                        Some(None)
                    } else {
                        inner.unit.map(Some)
                    },
                    number_higher_is_better: inner.higher_is_better,
                    select_options,
                    select_ordered: inner.select_ordered,
                    level_type_id,
                    policy: inner.policy.map(mapping::policy_from_proto).transpose()?,
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let id = FeatureId::from_proto(&inner.id)?;
        let status = mapping::feature_status_from_proto(inner.status)?;

        self.store
            .set_feature_status(actor, id, tenant_id, status)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(SetFeatureStatusResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn get_feature_code_holder(
        &self,
        request: Request<GetFeatureCodeHolderRequest>,
    ) -> Result<Response<GetFeatureCodeHolderResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let id = FeatureId::from_proto(&inner.id)?;

        let holder = self
            .store
            .feature_code_holder(id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetFeatureCodeHolderResponse {
            holder: holder.map(mapping::feature_ref_to_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_entitlement(
        &self,
        request: Request<CreateEntitlementRequest>,
    ) -> Result<Response<CreateEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let entity = mapping::entity_from_proto(inner.entity.as_ref())?;
        let spec = mapping::entitlement_spec_from_proto(
            inner
                .spec
                .ok_or_else(|| Status::invalid_argument("spec is required"))?,
        )?;

        let entitlement = self
            .store
            .create_entitlement(
                &actor,
                EntitlementNew {
                    tenant_id,
                    entity,
                    resource_id: ResourceId::from_proto_opt(inner.resource_id)?,
                    spec,
                },
            )
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
        let feature_ids: Vec<FeatureId> = entitlements
            .iter()
            .map(|e| e.feature_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let features = self
            .store
            .list_features_by_ids(&feature_ids, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListEntitlementsByEntityResponse {
            entitlements: entitlements
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
            features: features
                .into_iter()
                .map(mapping::feature_to_proto)
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let id = EntitlementId::from_proto(&inner.id)?;
        let update = mapping::entitlement_update_from_proto(inner)?;

        let entitlement = self
            .store
            .update_entitlement(&actor, id, tenant_id, update)
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let id = EntitlementId::from_proto(&inner.id)?;

        self.store
            .delete_entitlement(&actor, id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(DeleteEntitlementResponse {}))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
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
            warnings: effective_usage_warnings(&resolved),
            entitlements: resolved
                .into_iter()
                .map(mapping::effective_entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn list_credit_paid_features(
        &self,
        request: Request<ListCreditPaidFeaturesRequest>,
    ) -> Result<Response<ListCreditPaidFeaturesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = CustomerId::from_proto(&request.into_inner().customer_id)?;

        let features = self
            .services
            .list_credit_paid_features(tenant_id, customer_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListCreditPaidFeaturesResponse {
            features: features
                .into_iter()
                .map(mapping::credit_paid_feature_to_proto)
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
            .resolve_entitlements_for_entity(
                &mut conn,
                tenant_id,
                ResolveTarget::Product(product_id),
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
            .resolve_entitlements_for_entity(&mut conn, tenant_id, ResolveTarget::AddOn(add_on_id))
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
            .resolve_entitlements_for_entity(
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

        let resolved = self
            .store
            .subscription_entitlements(tenant_id, subscription_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(GetResolvedResponse {
            entitlements: resolved
                .into_iter()
                .map(|(entitlement, own_grant)| {
                    meteroid_grpc::meteroid::api::entitlements::v1::ResolvedEntitlement {
                        own_grant: own_grant.map(mapping::subscription_grant_to_proto),
                        ..mapping::resolved_entitlement_to_proto(entitlement)
                    }
                })
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
            .resolve_entitlements_for_entity(&mut conn, tenant_id, ResolveTarget::Quote(quote_id))
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
            .resolve_entitlements_for_entity(
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let entity = mapping::entity_from_proto(inner.entity.as_ref())?;
        let specs = inner
            .specs
            .into_iter()
            .map(mapping::entitlement_spec_from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let created = self
            .store
            .batch_create_entitlements(&actor, tenant_id, entity, specs)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(BatchCreateEntitlementsResponse {
            created: created
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_entitlement_history(
        &self,
        request: Request<ListEntitlementHistoryRequest>,
    ) -> Result<Response<ListEntitlementHistoryResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = EntitlementId::from_proto(&request.into_inner().id)?;

        let rows = self
            .store
            .get_entitlement_history(id, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(ListEntitlementHistoryResponse {
            entitlements: rows
                .into_iter()
                .map(mapping::entitlement_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn check_entitlement(
        &self,
        request: Request<CheckEntitlementRequest>,
    ) -> Result<Response<CheckEntitlementResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let (customer_id, feature, check) =
            mapping::check_request_from_proto(request.into_inner(), &actor)?;

        let answer = self
            .store
            .check_entitlement(tenant_id, customer_id, feature, check)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(mapping::check_response_to_proto(answer)))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn get_entitlement_snapshot(
        &self,
        request: Request<GetEntitlementSnapshotRequest>,
    ) -> Result<Response<GetEntitlementSnapshotResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let customer_id = CustomerId::from_proto(&inner.customer_id)?;
        let resources = ResourceInput {
            resources: inner.resources.into_iter().collect(),
            attributes: inner.attributes.into_iter().collect(),
        };

        let snapshot = self
            .store
            .entitlement_snapshot(
                tenant_id,
                customer_id,
                &resources,
                inner.if_none_match.as_deref(),
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(mapping::snapshot_to_proto(snapshot)))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn get_entitlement_snapshots(
        &self,
        request: Request<GetEntitlementSnapshotsRequest>,
    ) -> Result<Response<GetEntitlementSnapshotsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let customer_id = CustomerId::from_proto(&inner.customer_id)?;
        let resource_ids = inner
            .resource_ids
            .iter()
            .map(ResourceId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;
        let feature_ids = inner
            .feature_ids
            .iter()
            .map(FeatureId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let snapshots = self
            .store
            .entitlement_snapshots(
                tenant_id,
                customer_id,
                &resource_ids,
                (!feature_ids.is_empty()).then_some(feature_ids.as_slice()),
            )
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(mapping::snapshots_to_proto(snapshots)))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn report_entitlement_usage(
        &self,
        request: Request<ReportEntitlementUsageRequest>,
    ) -> Result<Response<ReportEntitlementUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let (customer_id, feature, write) =
            mapping::app_count_write_from_proto(request.into_inner())?;

        let feature = self
            .store
            .get_feature_by_id_or_code(feature, tenant_id)
            .await
            .map_err(EntitlementApiError::from)?;
        let result = self
            .store
            .report_app_count(tenant_id, customer_id, feature.id, write)
            .await
            .map_err(EntitlementApiError::from)?;

        Ok(Response::new(mapping::app_count_result_to_proto(result)))
    }
}

/// Mirrors FE regex `^[a-z0-9][a-z0-9_-]*$` with an additional 128-char cap.
fn validate_feature_code(code: &str) -> Result<String, Status> {
    let code = code.trim();
    if code.len() > 128 {
        return Err(Status::invalid_argument(format!(
            "invalid feature code: too long ({} > 128 chars)",
            code.len()
        )));
    }
    if !lazy_regex::regex_is_match!(r"^[a-z0-9][a-z0-9_-]*$", code) {
        return Err(Status::invalid_argument(
            "invalid feature code: must start with a lowercase letter or digit, then only lowercase letters, digits, '-', '_'",
        ));
    }
    Ok(code.to_string())
}

#[cfg(test)]
mod tests {
    use super::validate_feature_code;

    #[test]
    fn accepts_valid_slugs() {
        for c in ["audit_log", "mfa_policy", "sso", "a1-b2_c3"] {
            assert!(validate_feature_code(c).is_ok(), "{c} should be valid");
        }
    }

    #[test]
    fn trims_then_validates() {
        assert_eq!(validate_feature_code("  audit_log  ").unwrap(), "audit_log");
    }

    #[test]
    fn rejects_invalid_slugs() {
        for c in [
            "",
            "  ",
            "Audit_Log",
            "audit log",
            "audit'log",
            "feature.v2",
            "-leading",
            "_leading",
            "über",
        ] {
            assert!(validate_feature_code(c).is_err(), "{c} should be invalid");
        }
    }
}

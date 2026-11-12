use super::PlanServiceComponents;
use crate::entitlements::mapping::entitlement_spec_from_proto;
use crate::plans::error::PlanApiError;
use crate::plans::mapping::plans::{
    ListPlanVersionWrapper, PlanOverviewWrapper, PlanStatusWrapper, PlanTypeWrapper,
    PlanVersionWrapper, PlanWithVersionWrapper,
};
use common_domain::ids::{PlanId, PlanVersionId, ProductFamilyId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::{gates, minimum_commitments_open, minimum_commitments_refused};
use meteroid_grpc::meteroid::api::plans::v1::get_plan_with_version_request::Filter;
use meteroid_grpc::meteroid::api::plans::v1::{
    ArchivePlanRequest, ArchivePlanResponse, ClearPlanMinimumCommitmentRequest,
    ClearPlanMinimumCommitmentResponse, CopyVersionToDraftRequest, CopyVersionToDraftResponse,
    CreateDraftPlanRequest, CreateDraftPlanResponse, DiscardDraftVersionRequest,
    DiscardDraftVersionResponse, GetPlanOverviewRequest, GetPlanOverviewResponse,
    GetPlanParametersRequest, GetPlanParametersResponse, GetPlanWithVersionByVersionIdRequest,
    GetPlanWithVersionByVersionIdResponse, GetPlanWithVersionRequest, GetPlanWithVersionResponse,
    ListPlanVersionByIdRequest, ListPlanVersionByIdResponse, ListPlansRequest, ListPlansResponse,
    PublishPlanVersionRequest, PublishPlanVersionResponse, SetPlanMinimumCommitmentRequest,
    SetPlanMinimumCommitmentResponse, UnarchivePlanRequest, UnarchivePlanResponse,
    UpdateDraftPlanOverviewRequest, UpdateDraftPlanOverviewResponse, UpdatePlanTrialRequest,
    UpdatePlanTrialResponse, UpdatePublishedPlanOverviewRequest,
    UpdatePublishedPlanOverviewResponse, plans_service_server::PlansService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain;
use meteroid_store::domain::{
    PlanAndVersionPatch, PlanFilters, PlanPatch, PlanVersionFilter, PlanVersionPatch, TrialPatch,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::{PlansInterface, ProductFamilyInterface};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PlansService for PlanServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_draft_plan(
        &self,
        request: Request<CreateDraftPlanRequest>,
    ) -> Result<Response<CreateDraftPlanResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let plan_type: domain::enums::PlanTypeEnum = PlanTypeWrapper(req.plan_type()).into();

        // hack, remove when default product family is revisited
        let pf_id = if req.product_family_local_id.to_lowercase().as_str() == "default" {
            self.store
                .find_default_product_family(tenant_id)
                .await
                .map_err(Into::<PlanApiError>::into)?
                .id
        } else {
            ProductFamilyId::from_proto(req.product_family_local_id)?
        };

        let entitlements = req
            .entitlements
            .into_iter()
            .map(entitlement_spec_from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let plan_new = domain::FullPlanNew {
            plan: domain::PlanNew {
                name: req.name,
                description: req.description,
                tenant_id,
                product_family_id: pf_id,
                status: domain::enums::PlanStatusEnum::Draft,
                plan_type,
            },
            version: domain::PlanVersionNewInternal {
                is_draft_version: true,
                trial: None,
                period_start_day: None,
                net_terms: 0,
                currency: Some(req.currency),
                billing_cycles: None,
                entitlements,
                tax_inclusive: req.tax_inclusive,
                credit_exhausted_rule: None,
            },
            price_components: vec![],
        };

        let plan_details = self
            .store
            .insert_plan(actor, plan_new)
            .await
            .map(|x| PlanWithVersionWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(CreateDraftPlanResponse {
            plan: Some(plan_details),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_plans(
        &self,
        request: Request<ListPlansRequest>,
    ) -> Result<Response<ListPlansResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let pagination_req = req.pagination.into_domain();

        let plan_filters = match req.filters {
            None => PlanFilters {
                search: None,
                filter_status: vec![],
                filter_type: vec![],
                filter_currency: None,
            },
            Some(filter) => PlanFilters {
                search: filter.search.clone(),
                filter_status: filter
                    .statuses()
                    .map(|status| PlanStatusWrapper(status).into())
                    .collect(),
                filter_type: filter
                    .types()
                    .map(|plan_type| PlanTypeWrapper(plan_type).into())
                    .collect(),
                filter_currency: filter.currency,
            },
        };

        let res = self
            .store
            .list_plans(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.product_family_local_id)?,
                plan_filters,
                pagination_req,
                req.order_by,
            )
            .await
            .map_err(Into::<PlanApiError>::into)?;

        let response = ListPlansResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
            plans: res
                .items
                .into_iter()
                .map(|l| PlanOverviewWrapper::from(l).0)
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn list_plan_version_by_id(
        &self,
        request: Request<ListPlanVersionByIdRequest>,
    ) -> Result<Response<ListPlanVersionByIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let plan_id = PlanId::from_proto(&req.plan_id)?;

        let pagination_req = req.pagination.into_domain();

        let res = self
            .store
            .list_plan_versions(plan_id, tenant_id, pagination_req)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        let response = ListPlanVersionByIdResponse {
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
            plan_versions: res
                .items
                .into_iter()
                .map(|l| ListPlanVersionWrapper::from(l).0)
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn copy_version_to_draft(
        &self,
        request: Request<CopyVersionToDraftRequest>,
    ) -> Result<Response<CopyVersionToDraftResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .copy_plan_version_to_draft(plan_version_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanVersionWrapper::from(x).0)?;

        Ok(Response::new(CopyVersionToDraftResponse {
            plan_version: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn publish_plan_version(
        &self,
        request: Request<PublishPlanVersionRequest>,
    ) -> Result<Response<PublishPlanVersionResponse>, Status> {
        let actor_typed = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .publish_plan_version(actor_typed, plan_version_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanVersionWrapper::from(x).0)?;

        Ok(Response::new(PublishPlanVersionResponse {
            plan_version: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn discard_draft_version(
        &self,
        request: Request<DiscardDraftVersionRequest>,
    ) -> Result<Response<DiscardDraftVersionResponse>, Status> {
        let actor_typed = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        self.store
            .discard_draft_plan_version(actor_typed, plan_version_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(DiscardDraftVersionResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn update_draft_plan_overview(
        &self,
        request: Request<UpdateDraftPlanOverviewRequest>,
    ) -> Result<Response<UpdateDraftPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let credit_exhausted_rule = req
            .credit_exhausted_rule
            .map(crate::plans::mapping::plans::credit_exhausted_rule_from_proto)
            .transpose()?;
        self.store
            .ensure_credit_rule_enforceable(credit_exhausted_rule.as_ref())
            .map_err(Into::<PlanApiError>::into)?;

        let res = self
            .store
            .patch_draft_plan(
                actor,
                PlanAndVersionPatch {
                    version: PlanVersionPatch {
                        id: plan_version_id,
                        tenant_id,
                        currency: Some(req.currency),
                        net_terms: Some(req.net_terms as i32),
                        tax_inclusive: Some(req.tax_inclusive),
                        credit_exhausted_rule,
                    },
                    name: Some(req.name),
                    description: Some(req.description),
                },
            )
            .await
            .map(|x| PlanWithVersionWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(UpdateDraftPlanOverviewResponse {
            plan: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_published_plan_overview(
        &self,
        request: Request<UpdatePublishedPlanOverviewRequest>,
    ) -> Result<Response<UpdatePublishedPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let organization_id = request.organization();
        let req = request.into_inner();
        let plan_id = PlanId::from_proto(&req.plan_id)?;

        if req.issues_license == Some(true) {
            let current = self
                .store
                .get_plan_overview(plan_id, tenant_id)
                .await
                .map_err(Into::<PlanApiError>::into)?;
            if !current.issues_license
                && !self
                    .feature_resolver
                    .resolve(organization_id?)
                    .await
                    .is_enabled(gates::LICENSING)
            {
                return Err(Status::permission_denied(
                    "Licensing is not enabled for this organization.",
                ));
            }
        }

        let res = self
            .store
            .patch_published_plan(
                actor,
                PlanPatch {
                    id: plan_id,
                    tenant_id,
                    name: Some(req.name),
                    description: Some(req.description),
                    active_version_id: None,
                    self_service_rank: req
                        .self_service_rank
                        .map(|v| if v > 0 { Some(v) } else { None }),
                    issues_license: req.issues_license,
                },
            )
            .await
            .map(|x| PlanOverviewWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(UpdatePublishedPlanOverviewResponse {
            plan_overview: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_parameters(
        &self,
        _request: Request<GetPlanParametersRequest>,
    ) -> Result<Response<GetPlanParametersResponse>, Status> {
        todo!()
    }

    #[tracing::instrument(skip_all)]
    async fn update_plan_trial(
        &self,
        request: Request<UpdatePlanTrialRequest>,
    ) -> Result<Response<UpdatePlanTrialResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .patch_trial(
                actor,
                TrialPatch {
                    tenant_id,
                    plan_version_id,
                    trial: req
                        .trial
                        .map(|t| {
                            Ok::<domain::PlanTrial, Status>(domain::PlanTrial {
                                duration_days: t.duration_days,
                                trialing_plan_id: PlanId::from_proto_opt(t.trialing_plan_id)?,
                                trial_is_free: t.trial_is_free,
                            })
                        })
                        .transpose()?,
                },
            )
            .await
            .map(|x| PlanWithVersionWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(UpdatePlanTrialResponse { plan: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_overview(
        &self,
        request: Request<GetPlanOverviewRequest>,
    ) -> Result<Response<GetPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let res = self
            .store
            .get_plan_overview(PlanId::from_proto(&req.local_id)?, tenant_id)
            .await
            .map(|x| PlanOverviewWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(GetPlanOverviewResponse {
            plan_overview: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_with_version(
        &self,
        request: Request<GetPlanWithVersionRequest>,
    ) -> Result<Response<GetPlanWithVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_id = PlanId::from_proto(&req.local_id)?;
        let (filter, fallback_to_draft) = match req.filter {
            None => (PlanVersionFilter::Active, false),
            Some(c) => match c {
                Filter::Version(v) => (PlanVersionFilter::Version(v as i32), false),
                Filter::Draft(()) => (PlanVersionFilter::Draft, false),
                Filter::Active(()) => (PlanVersionFilter::Active, false),
                Filter::ActiveOrDraft(()) => (PlanVersionFilter::Active, true),
            },
        };

        let res = match self.store.get_plan(plan_id, tenant_id, filter).await {
            Err(err)
                if fallback_to_draft
                    && matches!(err.current_context(), StoreError::ValueNotFound(_)) =>
            {
                self.store
                    .get_plan(plan_id, tenant_id, PlanVersionFilter::Draft)
                    .await
            }
            other => other,
        }
        .map(|x| PlanWithVersionWrapper::from(x).0)
        .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(GetPlanWithVersionResponse {
            plan: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_with_version_by_version_id(
        &self,
        request: Request<GetPlanWithVersionByVersionIdRequest>,
    ) -> Result<Response<GetPlanWithVersionByVersionIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let res = self
            .store
            .get_plan_by_version_id(PlanVersionId::from_proto(&req.local_id)?, tenant_id)
            .await
            .map(|x| PlanWithVersionWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(GetPlanWithVersionByVersionIdResponse {
            plan: Some(res),
        }))
    }

    //
    // #[tracing::instrument(skip_all)]
    // async fn get_plan_parameters(
    //     &self,
    //     request: Request<GetPlanParametersRequest>,
    // ) -> Result<Response<GetPlanParametersResponse>, Status> {
    //     let tenant_id = request.tenant()?;
    //     let req = request.into_inner();
    //     let connection = self.get_connection().await?;
    //
    //     let components = pricecomponents::ext::list_price_components(
    //         parse_uuid!(&req.plan_version_id)?,
    //         tenant_id,
    //         &connection,
    //     )
    //     .await?;
    //     let plan_parameters = pricecomponents::ext::components_to_params(components)
    //         .into_iter()
    //         .map(mapping::plans::parameters::to_grpc)
    //         .collect();
    //
    //     Ok(Response::new(GetPlanParametersResponse {
    //         parameters: plan_parameters,
    //     }))
    // }

    #[tracing::instrument(skip_all)]
    async fn archive_plan(
        &self,
        request: Request<ArchivePlanRequest>,
    ) -> Result<Response<ArchivePlanResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let plan_id = PlanId::from_proto(&req.id)?;

        self.store
            .archive_plan(actor, plan_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(ArchivePlanResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn unarchive_plan(
        &self,
        request: Request<UnarchivePlanRequest>,
    ) -> Result<Response<UnarchivePlanResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_id = PlanId::from_proto(&req.id)?;

        self.store
            .unarchive_plan(plan_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(UnarchivePlanResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn set_plan_minimum_commitment(
        &self,
        request: Request<SetPlanMinimumCommitmentRequest>,
    ) -> Result<Response<SetPlanMinimumCommitmentResponse>, Status> {
        use crate::plans::mapping::plans::{
            minimum_commitment_from_proto, minimum_commitment_to_proto,
        };

        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        let proto_min = req
            .minimum_commitment
            .ok_or_else(|| Status::invalid_argument("minimum_commitment is required"))?;
        let (amount_subunit, scope) =
            minimum_commitment_from_proto(proto_min).map_err(Status::invalid_argument)?;

        if !minimum_commitments_open(self.feature_resolver.as_ref(), organization_id).await {
            let kept = self
                .store
                .get_plan_minimum(plan_version_id, tenant_id)
                .await
                .map_err(Into::<PlanApiError>::into)?
                .is_some_and(|stored| {
                    stored.amount_subunit == amount_subunit && stored.scope.same_as(&scope)
                });
            if !kept {
                return Err(minimum_commitments_refused());
            }
        }

        let saved = self
            .store
            .set_plan_minimum(plan_version_id, tenant_id, amount_subunit, scope)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(SetPlanMinimumCommitmentResponse {
            minimum_commitment: Some(minimum_commitment_to_proto(&saved)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn clear_plan_minimum_commitment(
        &self,
        request: Request<ClearPlanMinimumCommitmentRequest>,
    ) -> Result<Response<ClearPlanMinimumCommitmentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;
        self.store
            .clear_plan_minimum(plan_version_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)?;
        Ok(Response::new(ClearPlanMinimumCommitmentResponse {}))
    }
}

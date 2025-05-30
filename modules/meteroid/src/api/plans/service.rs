use super::PlanServiceComponents;
use crate::api::plans::error::PlanApiError;
use crate::api::plans::mapping::plans::{
    ActionAfterTrialWrapper, ListPlanVersionWrapper, PlanOverviewWrapper, PlanStatusWrapper,
    PlanTypeWrapper, PlanVersionWrapper, PlanWithVersionWrapper,
};
use crate::api::utils::PaginationExt;
use common_domain::ids::{PlanId, PlanVersionId, ProductFamilyId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::plans::v1::get_plan_with_version_request::Filter;
use meteroid_grpc::meteroid::api::plans::v1::{
    CopyVersionToDraftRequest, CopyVersionToDraftResponse, CreateDraftPlanRequest,
    CreateDraftPlanResponse, DiscardDraftVersionRequest, DiscardDraftVersionResponse,
    GetPlanOverviewRequest, GetPlanOverviewResponse, GetPlanParametersRequest,
    GetPlanParametersResponse, GetPlanWithVersionRequest, GetPlanWithVersionResponse,
    ListPlanVersionByIdRequest, ListPlanVersionByIdResponse, ListPlansRequest, ListPlansResponse,
    PublishPlanVersionRequest, PublishPlanVersionResponse, UpdateDraftPlanOverviewRequest,
    UpdateDraftPlanOverviewResponse, UpdatePlanTrialRequest, UpdatePlanTrialResponse,
    UpdatePublishedPlanOverviewRequest, UpdatePublishedPlanOverviewResponse,
    list_plans_request::SortBy, plans_service_server::PlansService,
};
use meteroid_store::domain;
use meteroid_store::domain::{
    OrderByRequest, PlanAndVersionPatch, PlanFilters, PlanPatch, PlanVersionFilter,
    PlanVersionPatch, TrialPatch,
};
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
        let created_by = request.actor()?;

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

        let plan_new = domain::FullPlanNew {
            plan: domain::PlanNew {
                name: req.name,
                description: req.description,
                created_by,
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
                currency: None,
                billing_cycles: None,
            },
            price_components: vec![],
        };

        let plan_details = self
            .store
            .insert_plan(plan_new)
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

        let order_by = match req.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::NameAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::NameDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let plan_filters = match req.filters {
            None => PlanFilters {
                search: None,
                filter_status: vec![],
                filter_type: vec![],
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
            },
        };

        let res = self
            .store
            .list_plans(
                tenant_id,
                ProductFamilyId::from_proto_opt(req.product_family_local_id)?,
                plan_filters,
                pagination_req,
                order_by,
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .copy_plan_version_to_draft(plan_version_id, tenant_id, actor)
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .publish_plan_version(plan_version_id, tenant_id, actor)
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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        self.store
            .discard_draft_plan_version(plan_version_id, tenant_id, actor)
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
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .patch_draft_plan(PlanAndVersionPatch {
                version: PlanVersionPatch {
                    id: plan_version_id,
                    tenant_id,
                    currency: Some(req.currency),
                    net_terms: Some(req.net_terms as i32),
                },
                name: Some(req.name),
                description: Some(req.description),
            })
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
        let req = request.into_inner();
        let plan_id = PlanId::from_proto(&req.plan_id)?;

        let res = self
            .store
            .patch_published_plan(PlanPatch {
                id: plan_id,
                tenant_id,
                name: Some(req.name),
                description: Some(req.description),
                active_version_id: None,
            })
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
        let req = request.into_inner();

        let plan_version_id = PlanVersionId::from_proto(&req.plan_version_id)?;

        let res = self
            .store
            .patch_trial(TrialPatch {
                tenant_id,
                plan_version_id,
                trial: req
                    .trial
                    .map(|t| {
                        Ok::<domain::PlanTrial, Status>(domain::PlanTrial {
                            action_after_trial: Some(
                                ActionAfterTrialWrapper(t.action_after_trial()).into(),
                            ),
                            duration_days: t.duration_days,
                            trialing_plan_id: PlanId::from_proto_opt(t.trialing_plan_id)?,
                            downgrade_plan_id: PlanId::from_proto_opt(t.downgrade_plan_id)?,
                            require_pre_authorization: t.trial_is_free,
                        })
                    })
                    .transpose()?,
            })
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

        let filter = match req.filter {
            None => PlanVersionFilter::Active,
            Some(c) => match c {
                Filter::Version(v) => PlanVersionFilter::Version(v as i32),
                Filter::Draft(_) => PlanVersionFilter::Draft,
                Filter::Active(_) => PlanVersionFilter::Active,
            },
        };

        let res = self
            .store
            .get_plan(PlanId::from_proto(&req.local_id)?, tenant_id, filter)
            .await
            .map(|x| PlanWithVersionWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(GetPlanWithVersionResponse {
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
}

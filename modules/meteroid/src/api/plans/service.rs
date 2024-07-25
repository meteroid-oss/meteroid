use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::plans::v1::{
    list_plans_request::SortBy, plans_service_server::PlansService, CopyVersionToDraftRequest,
    CopyVersionToDraftResponse, CreateDraftPlanRequest, CreateDraftPlanResponse,
    DiscardDraftVersionRequest, DiscardDraftVersionResponse, GetLastPublishedPlanVersionRequest,
    GetLastPublishedPlanVersionResponse, GetPlanByExternalIdRequest, GetPlanByExternalIdResponse,
    GetPlanOverviewByExternalIdRequest, GetPlanOverviewByExternalIdResponse,
    GetPlanParametersRequest, GetPlanParametersResponse, GetPlanVersionByIdRequest,
    GetPlanVersionByIdResponse, ListPlanVersionByIdRequest, ListPlanVersionByIdResponse,
    ListPlansRequest, ListPlansResponse, ListSubscribablePlanVersionRequest,
    ListSubscribablePlanVersionResponse, PublishPlanVersionRequest, PublishPlanVersionResponse,
    UpdateDraftPlanOverviewRequest, UpdateDraftPlanOverviewResponse,
    UpdatePublishedPlanOverviewRequest, UpdatePublishedPlanOverviewResponse,
};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use crate::api::plans::error::PlanApiError;

use crate::api::domain_mapping::billing_period;
use crate::api::plans::mapping::plans::{
    ListPlanVersionWrapper, ListPlanWrapper, ListSubscribablePlanVersionWrapper,
    PlanDetailsWrapper, PlanOverviewWrapper, PlanTypeWrapper, PlanVersionWrapper,
};
use crate::api::utils::PaginationExt;
use crate::{api::utils::parse_uuid, parse_uuid};
use meteroid_store::domain;
use meteroid_store::domain::{OrderByRequest, PlanAndVersionPatch, PlanPatch, PlanVersionPatch};
use meteroid_store::repositories::PlansInterface;

use super::PlanServiceComponents;

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

        let plan_type: domain::enums::PlanTypeEnum =
            PlanTypeWrapper(req.plan_type().clone()).into();

        let plan_new = domain::FullPlanNew {
            plan: domain::PlanNew {
                name: req.name,
                description: req.description,
                created_by,
                tenant_id,
                external_id: req.external_id,
                product_family_external_id: req.product_family_external_id,
                status: domain::enums::PlanStatusEnum::Draft,
                plan_type,
            },
            version: domain::PlanVersionNewInternal {
                is_draft_version: true,
                trial_duration_days: None,
                trial_fallback_plan_id: None,
                period_start_day: None,
                net_terms: 0,
                currency: None,
                billing_cycles: None,
                billing_periods: vec![],
            },
            price_components: vec![],
        };

        let plan_details = self
            .store
            .insert_plan(plan_new)
            .await
            .map(|x| PlanDetailsWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(CreateDraftPlanResponse {
            plan: Some(plan_details),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_by_external_id(
        &self,
        request: Request<GetPlanByExternalIdRequest>,
    ) -> Result<Response<GetPlanByExternalIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let plan_details = self
            .store
            .get_plan_by_external_id(req.external_id.as_str(), tenant_id)
            .await
            .map(|x| PlanDetailsWrapper::from(x).0)
            .map_err(Into::<PlanApiError>::into)?;

        Ok(Response::new(GetPlanByExternalIdResponse {
            plan_details: Some(plan_details),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_plans(
        &self,
        request: Request<ListPlansRequest>,
    ) -> Result<Response<ListPlansResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = match req.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::NameAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::NameDesc) => OrderByRequest::NameDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_plans(
                tenant_id,
                req.search,
                req.product_family_external_id,
                pagination_req,
                order_by,
            )
            .await
            .map_err(Into::<PlanApiError>::into)?;

        let response = ListPlansResponse {
            pagination_meta: req.pagination.into_response(res.total_results as u32),
            plans: res
                .items
                .into_iter()
                .map(|l| ListPlanWrapper::from(l).0)
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn list_subscribable_plan_version(
        &self,
        request: Request<ListSubscribablePlanVersionRequest>,
    ) -> Result<Response<ListSubscribablePlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;

        let plan_versions = self
            .store
            .list_latest_published_plan_versions(tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        let response = ListSubscribablePlanVersionResponse {
            plan_versions: plan_versions
                .into_iter()
                .map(|x| ListSubscribablePlanVersionWrapper::from(x).0)
                .collect(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_version_by_id(
        &self,
        request: Request<GetPlanVersionByIdRequest>,
    ) -> Result<Response<GetPlanVersionByIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let id = parse_uuid!(&req.plan_version_id)?;

        let version = self
            .store
            .get_plan_version_by_id(id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanVersionWrapper::from(x).0)?;

        let response = GetPlanVersionByIdResponse {
            plan_version: Some(version),
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
        let plan_id = parse_uuid!(&req.plan_id)?;

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let res = self
            .store
            .list_plan_versions(plan_id, tenant_id, pagination_req)
            .await
            .map_err(Into::<PlanApiError>::into)?;

        let response = ListPlanVersionByIdResponse {
            pagination_meta: req.pagination.into_response(res.total_results as u32),
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

        let plan_version_id = parse_uuid!(&req.plan_version_id)?;

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

        let plan_version_id = parse_uuid!(&req.plan_version_id)?;

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
    async fn get_last_published_plan_version(
        &self,
        request: Request<GetLastPublishedPlanVersionRequest>,
    ) -> Result<Response<GetLastPublishedPlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let plan_id = parse_uuid!(&req.plan_id)?;

        let res = self
            .store
            .get_last_published_plan_version(plan_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| x.map(|x| PlanVersionWrapper::from(x).0))?;

        Ok(Response::new(GetLastPublishedPlanVersionResponse {
            version: res,
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

        let plan_version_id = parse_uuid!(&req.plan_version_id)?;

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

        let plan_version_id = parse_uuid!(&req.plan_version_id)?;

        let frequencies = req
            .billing_periods
            .iter()
            .map(|f| {
                BillingPeriod::try_from(*f)
                    .map_err(|_| PlanApiError::InvalidArgument("billing period".to_string()))
                    .map(billing_period::from_proto)
            })
            .collect::<Result<Vec<domain::enums::BillingPeriodEnum>, PlanApiError>>()?;

        let res = self
            .store
            .patch_draft_plan(PlanAndVersionPatch {
                version: PlanVersionPatch {
                    id: plan_version_id,
                    tenant_id,
                    currency: Some(req.currency),
                    net_terms: Some(req.net_terms as i32),
                    billing_periods: Some(frequencies),
                },
                name: Some(req.name),
                description: Some(req.description),
            })
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanOverviewWrapper::from(x).0)?;

        Ok(Response::new(UpdateDraftPlanOverviewResponse {
            plan_overview: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_published_plan_overview(
        &self,
        request: Request<UpdatePublishedPlanOverviewRequest>,
    ) -> Result<Response<UpdatePublishedPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let plan_id = parse_uuid!(&req.plan_id)?;

        let res = self
            .store
            .patch_published_plan(PlanPatch {
                id: plan_id,
                tenant_id,
                name: Some(req.name),
                description: Some(req.description),
            })
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanOverviewWrapper::from(x).0)?;

        Ok(Response::new(UpdatePublishedPlanOverviewResponse {
            plan_overview: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_overview_by_external_id(
        &self,
        request: Request<GetPlanOverviewByExternalIdRequest>,
    ) -> Result<Response<GetPlanOverviewByExternalIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let res = self
            .store
            .get_plan_with_version_by_external_id(&req.external_id, tenant_id)
            .await
            .map_err(Into::<PlanApiError>::into)
            .map(|x| PlanOverviewWrapper::from(x).0)?;

        let response = GetPlanOverviewByExternalIdResponse {
            plan_overview: Some(res),
        };

        Ok(Response::new(response))
    }

    async fn get_plan_parameters(
        &self,
        _request: Request<GetPlanParametersRequest>,
    ) -> Result<Response<GetPlanParametersResponse>, Status> {
        todo!()
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

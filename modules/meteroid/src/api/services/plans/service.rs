use super::mapping;
use crate::api::services::pricecomponents;
use crate::api::services::shared::mapping::period::billing_period_to_db;
use crate::api::services::utils::PaginationExt;
use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    db::DbService,
    parse_uuid,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::plans::v1::{
    list_plans_request::SortBy, plans_service_server::PlansService, CopyVersionToDraftRequest,
    CopyVersionToDraftResponse, CreateDraftPlanRequest, CreateDraftPlanResponse,
    DiscardDraftVersionRequest, DiscardDraftVersionResponse, GetLastPublishedPlanVersionRequest,
    GetLastPublishedPlanVersionResponse, GetPlanByExternalIdRequest, GetPlanByExternalIdResponse,
    GetPlanOverviewByExternalIdRequest, GetPlanOverviewByExternalIdResponse,
    GetPlanParametersRequest, GetPlanParametersResponse, GetPlanVersionByIdRequest,
    GetPlanVersionByIdResponse, ListPlanVersionByIdRequest, ListPlanVersionByIdResponse,
    ListPlansRequest, ListPlansResponse, PlanDetails, PublishPlanVersionRequest,
    PublishPlanVersionResponse, UpdateDraftPlanOverviewRequest, UpdateDraftPlanOverviewResponse,
    UpdatePublishedPlanOverviewRequest, UpdatePublishedPlanOverviewResponse,
};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use meteroid_repository as db;
use meteroid_repository::Params;
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PlansService for DbService {
    #[tracing::instrument(skip(self))]
    async fn create_draft_plan(
        &self,
        request: Request<CreateDraftPlanRequest>,
    ) -> Result<Response<CreateDraftPlanResponse>, Status> {
        let tenant_id = request.tenant()?;
        let created_by = request.actor()?;

        let req = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let plan_type = req.plan_type();

        let params = db::plans::CreatePlanParams {
            id: uuid_gen::v7(),
            name: req.name,
            external_id: req.external_id,
            description: req.description,
            tenant_id,
            product_family_external_id: req.product_family_external_id,
            created_by,
            status: db::PlanStatusEnum::DRAFT,
            plan_type: match plan_type {
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Standard => {
                    db::PlanTypeEnum::STANDARD
                }
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Free => db::PlanTypeEnum::FREE,
                meteroid_grpc::meteroid::api::plans::v1::PlanType::Custom => {
                    db::PlanTypeEnum::CUSTOM
                }
            },
        };

        let plan = db::plans::create_plan()
            .params(&transaction, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to create plan")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let plan_version_params = db::plans::CreatePlanVersionParams {
            id: uuid_gen::v7(),
            plan_id: plan.id,
            version: 1,
            created_by,
            trial_duration_days: None,
            trial_fallback_plan_id: None,
            tenant_id,
            period_start_day: None,
            net_terms: None,
            currency: None::<String>,
            billing_cycles: None,
            billing_periods: vec![],
        };

        let version = db::plans::create_plan_version()
            .params(&transaction, &plan_version_params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to create plan version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let mapped_version = mapping::plans::version::db_to_server(version);
        let mapped_plan = mapping::plans::db_to_server(plan);

        Ok(Response::new(CreateDraftPlanResponse {
            plan: Some(PlanDetails {
                plan: Some(mapped_plan),
                current_version: Some(mapped_version),
                metadata: vec![],
            }),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn get_plan_by_external_id(
        &self,
        request: Request<GetPlanByExternalIdRequest>,
    ) -> Result<Response<GetPlanByExternalIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let plan = db::plans::find_plan_by_external_id()
            .bind(&connection, &tenant_id, &req.external_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get plan by external_id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let plan_version = db::plans::last_plan_version()
            .bind(&connection, &tenant_id, &plan.id, &None)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get plan by external_id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res_plan = mapping::plans::db_to_server(plan);
        let res_plan_version = mapping::plans::version::db_to_server(plan_version);

        Ok(Response::new(GetPlanByExternalIdResponse {
            plan_details: Some(PlanDetails {
                current_version: Some(res_plan_version),
                plan: Some(res_plan),
                metadata: vec![],
            }),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn list_plans(
        &self,
        request: Request<ListPlansRequest>,
    ) -> Result<Response<ListPlansResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::plans::ListPlansParams {
            tenant_id,
            product_family_external_id: req.product_family_external_id,
            limit: req.pagination.limit(),
            offset: req.pagination.offset(),
            order_by: match req.order_by.try_into() {
                Ok(SortBy::DateAsc) => "DATE_ASC",
                Ok(SortBy::DateDesc) => "DATE_DESC",
                Ok(SortBy::NameAsc) => "NAME_ASC",
                Ok(SortBy::NameDesc) => "NAME_DESC",
                Err(_) => "DATE_DESC",
            },
            search: req.search,
        };

        let plans = db::plans::list_plans()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to list plans")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total = plans.first().map(|p| p.total_count).unwrap_or(0);

        let response = ListPlansResponse {
            plans: plans
                .into_iter()
                .map(mapping::plans::list_db_to_server)
                .collect(),
            pagination_meta: req.pagination.into_response(total as u32),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn get_plan_version_by_id(
        &self,
        request: Request<GetPlanVersionByIdRequest>,
    ) -> Result<Response<GetPlanVersionByIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::plans::get_plan_version_by_id()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get version by id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = GetPlanVersionByIdResponse {
            plan_version: Some(mapping::plans::version::db_to_server(res)),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn list_plan_version_by_id(
        &self,
        request: Request<ListPlanVersionByIdRequest>,
    ) -> Result<Response<ListPlanVersionByIdResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::plans::ListPlansVersionsParams {
            tenant_id,
            plan_id: parse_uuid!(&inner.plan_id)?,
            limit: inner.pagination.limit(),
            offset: inner.pagination.offset(),
        };

        let plans = db::plans::list_plans_versions()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to list plans")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total = plans.first().map(|p| p.total_count).unwrap_or(0);

        let response = ListPlanVersionByIdResponse {
            plan_versions: plans
                .into_iter()
                .map(mapping::plans::version::list_db_to_server)
                .collect(),
            pagination_meta: inner.pagination.into_response(total as u32),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn copy_version_to_draft(
        &self,
        request: Request<CopyVersionToDraftRequest>,
    ) -> Result<Response<CopyVersionToDraftResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        db::plans::delete_all_draft_versions_of_same_plan()
            .bind(
                &transaction,
                &parse_uuid!(&req.plan_version_id)?,
                &tenant_id,
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to discard drafts")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let params = db::plans::CopyVersionToDraftParams {
            new_plan_version_id: uuid_gen::v7(),
            created_by: actor,
            original_plan_version_id: parse_uuid!(&req.plan_version_id)?,
            tenant_id,
        };

        let new_version = db::plans::copy_version_to_draft()
            .params(&transaction, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to copy plan version to draft")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let response = mapping::plans::version::db_to_server(new_version);

        Ok(Response::new(CopyVersionToDraftResponse {
            plan_version: Some(response),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn publish_plan_version(
        &self,
        request: Request<PublishPlanVersionRequest>,
    ) -> Result<Response<PublishPlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        // TODO validations
        // - all components on committed must have values for all periods
        let res = db::plans::publish_plan_version()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to publish plan version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        db::plans::activate_plan()
            .bind(&connection, &parse_uuid!(&req.plan_id)?, &tenant_id)
            .await
            .map_err(|e| {
                Status::internal("Unable to activate plan")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = mapping::plans::version::db_to_server(res);

        Ok(Response::new(PublishPlanVersionResponse {
            plan_version: Some(response),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn get_last_published_plan_version(
        &self,
        request: Request<GetLastPublishedPlanVersionRequest>,
    ) -> Result<Response<GetLastPublishedPlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::plans::last_plan_version()
            .bind(
                &connection,
                &tenant_id,
                &parse_uuid!(&req.plan_id)?,
                &Some(false),
            )
            .opt()
            .await
            .map_err(|e| {
                Status::internal("Unable to get last published plan version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = GetLastPublishedPlanVersionResponse {
            version: res.map(mapping::plans::version::db_to_server),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn discard_draft_version(
        &self,
        request: Request<DiscardDraftVersionRequest>,
    ) -> Result<Response<DiscardDraftVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        db::plans::delete_draft_plan_version()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .await
            .map_err(|e| {
                Status::internal("Unable to discard draft version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        db::plans::delete_plan_if_no_versions()
            .bind(&connection, &parse_uuid!(&req.plan_id)?, &tenant_id)
            .await
            .map_err(|e| {
                Status::internal("Unable to discard_draft_plan")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(DiscardDraftVersionResponse {}))
    }

    #[tracing::instrument(skip(self))]
    async fn update_draft_plan_overview(
        &self,
        request: Request<UpdateDraftPlanOverviewRequest>,
    ) -> Result<Response<UpdateDraftPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let frequencies = req
            .billing_periods
            .iter()
            .map(|f| {
                BillingPeriod::try_from(*f)
                    .map_err(|_| Status::invalid_argument("Invalid billing period"))
                    .map(|period| billing_period_to_db(&period))
            })
            .collect::<Result<Vec<db::BillingPeriodEnum>, Status>>()?;

        db::plans::update_plan_version_overview()
            .params(
                &transaction,
                &db::plans::UpdatePlanVersionOverviewParams {
                    plan_version_id: parse_uuid!(&req.plan_version_id)?,
                    tenant_id: tenant_id.clone(),
                    currency: req.currency,
                    billing_periods: frequencies,
                    net_terms: req.net_terms as i32,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to update draft plan version overview")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        db::plans::update_plan_overview()
            .params(
                &transaction,
                &db::plans::UpdatePlanOverviewParams {
                    plan_id: parse_uuid!(&req.plan_id)?,
                    tenant_id: tenant_id.clone(),
                    name: req.name,
                    description: req.description,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to update plan overview")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res = db::plans::get_plan_overview_by_id()
            .bind(
                &transaction,
                &parse_uuid!(&req.plan_version_id)?,
                &tenant_id,
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get plan overview")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let response = mapping::plans::overview::db_to_server(res);

        Ok(Response::new(UpdateDraftPlanOverviewResponse {
            plan_overview: Some(response),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn update_published_plan_overview(
        &self,
        request: Request<UpdatePublishedPlanOverviewRequest>,
    ) -> Result<Response<UpdatePublishedPlanOverviewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        db::plans::update_plan_overview()
            .params(
                &connection,
                &db::plans::UpdatePlanOverviewParams {
                    plan_id: parse_uuid!(&req.plan_id)?,
                    tenant_id,
                    name: req.name,
                    description: req.description,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to update plan overview")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res = db::plans::get_plan_overview_by_id()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get plan overview")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = mapping::plans::overview::db_to_server(res);

        Ok(Response::new(UpdatePublishedPlanOverviewResponse {
            plan_overview: Some(response),
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn get_plan_overview_by_external_id(
        &self,
        request: Request<GetPlanOverviewByExternalIdRequest>,
    ) -> Result<Response<GetPlanOverviewByExternalIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let plan = db::plans::get_plan_overview_by_external_id()
            .bind(&connection, &req.external_id, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get plan overview by external_id")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let res_plan_version = mapping::plans::overview::db_to_server(plan);

        let response = GetPlanOverviewByExternalIdResponse {
            plan_overview: Some(res_plan_version),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn get_plan_parameters(
        &self,
        request: Request<GetPlanParametersRequest>,
    ) -> Result<Response<GetPlanParametersResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let components = pricecomponents::ext::list_price_components(
            parse_uuid!(&req.plan_version_id)?,
            tenant_id,
            &connection,
        )
        .await?;
        let plan_parameters = pricecomponents::ext::components_to_params(components)
            .into_iter()
            .map(mapping::plans::parameters::to_grpc)
            .collect();

        Ok(Response::new(GetPlanParametersResponse {
            parameters: plan_parameters,
        }))
    }
}

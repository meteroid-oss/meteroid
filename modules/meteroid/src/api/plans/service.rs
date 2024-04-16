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
    ListSubscribablePlanVersionResponse, PlanDetails, PublishPlanVersionRequest,
    PublishPlanVersionResponse, UpdateDraftPlanOverviewRequest, UpdateDraftPlanOverviewResponse,
    UpdatePublishedPlanOverviewRequest, UpdatePublishedPlanOverviewResponse,
};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use meteroid_repository as db;
use meteroid_repository::Params;

use crate::api::plans::error::PlanApiError;
use crate::api::pricecomponents;
use crate::api::shared::mapping::period::billing_period_to_db;
use crate::api::utils::PaginationExt;
use crate::eventbus::Event;
use crate::{
    api::utils::{parse_uuid, uuid_gen},
    parse_uuid,
};

use super::{mapping, PlanServiceComponents};

#[tonic::async_trait]
impl PlansService for PlanServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_draft_plan(
        &self,
        request: Request<CreateDraftPlanRequest>,
    ) -> Result<Response<CreateDraftPlanResponse>, Status> {
        let actor = request.actor()?;
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
            .map_err(|e| PlanApiError::DatabaseError("Unable to create plan".to_string(), e))?;

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
                PlanApiError::DatabaseError("unable to create plan version".to_string(), e)
            })?;

        transaction.commit().await.map_err(|e| {
            PlanApiError::DatabaseError("failed to commit transaction".to_string(), e)
        })?;

        let mapped_version = mapping::plans::version::db_to_server(version.clone());
        let mapped_plan = mapping::plans::db_to_server(plan);

        let _ = self
            .eventbus
            .publish(Event::plan_created_draft(actor, version.id, tenant_id))
            .await;

        Ok(Response::new(CreateDraftPlanResponse {
            plan: Some(PlanDetails {
                plan: Some(mapped_plan),
                current_version: Some(mapped_version),
                metadata: vec![],
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
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
                PlanApiError::DatabaseError("unable to get plan by external_id".to_string(), e)
            })?;

        let plan_version = db::plans::last_plan_version()
            .bind(&connection, &tenant_id, &plan.id, &None)
            .one()
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to get plan by external_id".to_string(), e)
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

    #[tracing::instrument(skip_all)]
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
            .map_err(|e| PlanApiError::DatabaseError("unable to list plans".to_string(), e))?;

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

    #[tracing::instrument(skip_all)]
    async fn list_subscribable_plan_version(
        &self,
        request: Request<ListSubscribablePlanVersionRequest>,
    ) -> Result<Response<ListSubscribablePlanVersionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let connection = self.get_connection().await?;

        let plan_versions = db::plans::list_subscribable_plan_version()
            .bind(&connection, &tenant_id)
            .all()
            .await
            .map_err(|e| PlanApiError::DatabaseError("unable to list plans".to_string(), e))?;

        let response = ListSubscribablePlanVersionResponse {
            plan_versions: plan_versions
                .into_iter()
                .map(mapping::plans::list_subscribable_db_to_server)
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
        let connection = self.get_connection().await?;

        let res = db::plans::get_plan_version_by_id()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to get version by id".to_string(), e)
            })?;

        let response = GetPlanVersionByIdResponse {
            plan_version: Some(mapping::plans::version::db_to_server(res)),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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
            .map_err(|e| PlanApiError::DatabaseError("unable to list plans".to_string(), e))?;

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

    #[tracing::instrument(skip_all)]
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
            .map_err(|e| PlanApiError::DatabaseError("unable to discard drafts".to_string(), e))?;

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
                PlanApiError::DatabaseError("unable to copy plan version to draft".to_string(), e)
            })?;

        transaction.commit().await.map_err(|e| {
            PlanApiError::DatabaseError("failed to commit transaction".to_string(), e)
        })?;

        let response = mapping::plans::version::db_to_server(new_version);

        Ok(Response::new(CopyVersionToDraftResponse {
            plan_version: Some(response),
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
        let connection = self.get_connection().await?;

        // TODO validations
        // - all components on committed must have values for all periods
        let plan_version_row = db::plans::publish_plan_version()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to publish plan version".to_string(), e)
            })?;

        db::plans::activate_plan()
            .bind(&connection, &parse_uuid!(&req.plan_id)?, &tenant_id)
            .await
            .map_err(|e| PlanApiError::DatabaseError("unable to activate plan".to_string(), e))?;

        let response = mapping::plans::version::db_to_server(plan_version_row.clone());

        let _ = self
            .eventbus
            .publish(Event::plan_published_version(
                actor,
                plan_version_row.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(PublishPlanVersionResponse {
            plan_version: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
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
                PlanApiError::DatabaseError(
                    "unable to get last published plan version".to_string(),
                    e,
                )
            })?;

        let response = GetLastPublishedPlanVersionResponse {
            version: res.map(mapping::plans::version::db_to_server),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn discard_draft_version(
        &self,
        request: Request<DiscardDraftVersionRequest>,
    ) -> Result<Response<DiscardDraftVersionResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let plan_version_id = parse_uuid!(&req.plan_version_id)?;
        let plan_id = parse_uuid!(&req.plan_id)?;

        db::plans::delete_draft_plan_version()
            .bind(&connection, &plan_version_id, &tenant_id)
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to discard draft version".to_string(), e)
            })?;

        db::plans::delete_plan_if_no_versions()
            .bind(&connection, &plan_id, &tenant_id)
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to discard_draft_plan".to_string(), e)
            })?;

        let _ = self
            .eventbus
            .publish(Event::plan_discarded_version(
                actor,
                plan_version_id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(DiscardDraftVersionResponse {}))
    }

    #[tracing::instrument(skip_all)]
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
                    .map_err(|_| PlanApiError::InvalidArgument("billing period".to_string()))
                    .map(|period| billing_period_to_db(&period))
            })
            .collect::<Result<Vec<db::BillingPeriodEnum>, PlanApiError>>()?;

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
                PlanApiError::DatabaseError(
                    "unable to update draft plan version overview".to_string(),
                    e,
                )
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
                PlanApiError::DatabaseError("unable to update plan overview".to_string(), e)
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
                PlanApiError::DatabaseError("unable to get plan overview".to_string(), e)
            })?;

        transaction.commit().await.map_err(|e| {
            PlanApiError::DatabaseError("Failed to commit transaction".to_string(), e)
        })?;

        let response = mapping::plans::overview::db_to_server(res);

        Ok(Response::new(UpdateDraftPlanOverviewResponse {
            plan_overview: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
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
                PlanApiError::DatabaseError("unable to update plan overview".to_string(), e)
            })?;

        let res = db::plans::get_plan_overview_by_id()
            .bind(&connection, &parse_uuid!(&req.plan_version_id)?, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                PlanApiError::DatabaseError("unable to get plan overview".to_string(), e)
            })?;

        let response = mapping::plans::overview::db_to_server(res);

        Ok(Response::new(UpdatePublishedPlanOverviewResponse {
            plan_overview: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
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
                PlanApiError::DatabaseError(
                    "unable to get plan overview by external_id".to_string(),
                    e,
                )
            })?;

        let res_plan_version = mapping::plans::overview::db_to_server(plan);

        let response = GetPlanOverviewByExternalIdResponse {
            plan_overview: Some(res_plan_version),
        };

        Ok(Response::new(response))
    }

    async fn get_plan_parameters(&self, request: Request<GetPlanParametersRequest>) -> Result<Response<GetPlanParametersResponse>, Status> {
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

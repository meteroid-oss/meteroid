use cornucopia_async::Params;
use meteroid_repository as db;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    db::DbService,
    parse_uuid,
};

use crate::api::services::shared::mapping::period::billing_period_to_db;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::schedules::v1::{
    schedules_service_server::SchedulesService, CreateScheduleRequest, CreateScheduleResponse,
    EditScheduleRequest, EditScheduleResponse, EmptyResponse, ListSchedulesRequests,
    ListSchedulesResponse, RemoveScheduleRequest, Schedule,
};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use super::mapping;

#[tonic::async_trait]
impl SchedulesService for DbService {
    #[tracing::instrument(skip_all)]
    async fn list_schedules(
        &self,
        request: Request<ListSchedulesRequests>,
    ) -> Result<Response<ListSchedulesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::schedules::list_schedules()
            .params(
                &connection,
                &db::schedules::ListSchedulesParams {
                    plan_version_id: parse_uuid!(&req.plan_version_id)?,
                    tenant_id,
                },
            )
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to publish plan version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let schedules = res
            .into_iter()
            .map(mapping::schedules::db_to_server)
            .collect::<Result<Vec<Schedule>, Status>>()?;

        let response = ListSchedulesResponse { schedules };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_schedule(
        &self,
        request: Request<CreateScheduleRequest>,
    ) -> Result<Response<CreateScheduleResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let ramps = &req
            .ramps
            .ok_or_else(|| Status::invalid_argument("Missing ramps"))
            .and_then(|ramps| {
                serde_json::to_value(ramps).map_err(|e| {
                    Status::invalid_argument(format!("Failed to serialize ramps: {}", e))
                })
            })?;

        let billing_period: BillingPeriod = req
            .billing_period
            .try_into()
            .map_err(|e| Status::invalid_argument(format!("Invalid billing period: {}", e)))?;

        let schedule = db::schedules::create_schedule()
            .params(
                &connection,
                &db::schedules::CreateScheduleParams {
                    id: uuid_gen::v7(),
                    plan_version_id: parse_uuid!(&req.plan_version_id)?,
                    tenant_id,
                    billing_period: billing_period_to_db(&billing_period),
                    ramps,
                },
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to create price schedule")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = mapping::schedules::db_to_server(schedule)?;

        Ok(Response::new(CreateScheduleResponse {
            schedule: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_schedule(
        &self,
        request: Request<EditScheduleRequest>,
    ) -> Result<Response<EditScheduleResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let schedule = req
            .schedule
            .ok_or_else(|| Status::invalid_argument("Missing schedule"))?;

        let ramps = &schedule
            .ramps
            .ok_or_else(|| Status::invalid_argument("Missing ramps"))
            .and_then(|ramps| {
                serde_json::to_value(ramps).map_err(|e| {
                    Status::invalid_argument(format!("Failed to serialize ramps: {}", e))
                })
            })?;

        let schedule = db::schedules::update_schedule()
            .params(
                &connection,
                &db::schedules::UpdateScheduleParams {
                    id: parse_uuid!(&schedule.id)?,
                    tenant_id,
                    ramps,
                },
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to edit price schedule")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = mapping::schedules::db_to_server(schedule)?;

        Ok(Response::new(EditScheduleResponse {
            schedule: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_schedule(
        &self,
        request: Request<RemoveScheduleRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        db::schedules::delete_schedule()
            .params(
                &connection,
                &db::schedules::DeleteScheduleParams {
                    id: parse_uuid(&req.schedule_id, "schedule_id")?,
                    tenant_id,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to remove price schedule")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(EmptyResponse {}))
    }
}

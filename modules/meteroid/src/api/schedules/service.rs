use cornucopia_async::Params;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::schedules::v1::{
    schedules_service_server::SchedulesService, CreateScheduleRequest, CreateScheduleResponse,
    EditScheduleRequest, EditScheduleResponse, EmptyResponse, ListSchedulesRequests,
    ListSchedulesResponse, RemoveScheduleRequest, Schedule,
};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use meteroid_repository as db;

use crate::api::schedules::error::ScheduleApiError;
use crate::api::shared::mapping::period::billing_period_to_db;
use crate::{
    api::utils::{parse_uuid, uuid_gen},
    db::DbService,
    parse_uuid,
};

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
                ScheduleApiError::DatabaseError("unable to publish plan version".to_string(), e)
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
            .ok_or_else(|| ScheduleApiError::MissingArgument("ramps".to_string()))
            .and_then(|ramps| {
                serde_json::to_value(ramps).map_err(|e| {
                    ScheduleApiError::SerializationError("failed to serialize ramps".to_string(), e)
                })
            })?;

        let billing_period: BillingPeriod = req
            .billing_period
            .try_into()
            .map_err(|e| ScheduleApiError::InvalidArgument("billing period".to_string(), e))?;

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
                ScheduleApiError::DatabaseError("unable to create price schedule".to_string(), e)
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
            .ok_or_else(|| ScheduleApiError::MissingArgument("schedule".to_string()))?;

        let ramps = &schedule
            .ramps
            .ok_or_else(|| ScheduleApiError::MissingArgument("ramps".to_string()))
            .and_then(|ramps| {
                serde_json::to_value(ramps).map_err(|e| {
                    ScheduleApiError::SerializationError("failed to serialize ramps".to_string(), e)
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
                ScheduleApiError::DatabaseError("unable to edit price schedule".to_string(), e)
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
                ScheduleApiError::DatabaseError("unable to remove price schedule".to_string(), e)
            })?;

        Ok(Response::new(EmptyResponse {}))
    }
}

use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::schedules::v1::{
    CreateScheduleRequest, CreateScheduleResponse, EditScheduleRequest, EditScheduleResponse,
    EmptyResponse, ListSchedulesRequests, ListSchedulesResponse, RemoveScheduleRequest,
    schedules_service_server::SchedulesService,
};
use meteroid_store::domain;
use meteroid_store::repositories::schedules::ScheduleInterface;

use crate::api::domain_mapping::billing_period;
use crate::api::schedules::error::ScheduleApiError;
use crate::api::schedules::mapping::schedules::{PlanRampsWrapper, ScheduleWrapper};
use crate::{api::utils::parse_uuid, parse_uuid};

use super::ScheduleServiceComponents;

#[tonic::async_trait]
impl SchedulesService for ScheduleServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_schedules(
        &self,
        request: Request<ListSchedulesRequests>,
    ) -> Result<Response<ListSchedulesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let schedules = self
            .store
            .list_schedules(parse_uuid!(&req.plan_version_id)?, tenant_id)
            .await
            .map_err(Into::<ScheduleApiError>::into)?
            .into_iter()
            .map(|x| ScheduleWrapper::from(x).0)
            .collect();

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

        let schedule_new = domain::ScheduleNew {
            plan_version_id: parse_uuid!(&req.plan_version_id)?,
            billing_period: billing_period::from_proto(req.billing_period()),
            ramps: PlanRampsWrapper(
                req.ramps
                    .ok_or_else(|| ScheduleApiError::MissingArgument("ramps".to_string()))?,
            )
            .try_into()
            .map_err(Into::<ScheduleApiError>::into)?,
        };

        let response = self
            .store
            .insert_schedule(schedule_new, tenant_id)
            .await
            .map_err(Into::<ScheduleApiError>::into)
            .map(|x| ScheduleWrapper::from(x).0)?;

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

        let schedule = req
            .schedule
            .ok_or_else(|| ScheduleApiError::MissingArgument("schedule".to_string()))?;

        let ramps: domain::PlanRamps = PlanRampsWrapper(
            schedule
                .ramps
                .ok_or_else(|| ScheduleApiError::MissingArgument("ramps".to_string()))?,
        )
        .try_into()
        .map_err(Into::<ScheduleApiError>::into)?;

        let schedule_patch = domain::SchedulePatch {
            id: parse_uuid!(&schedule.id)?,
            ramps: Some(ramps),
        };

        let response = self
            .store
            .patch_schedule(schedule_patch, tenant_id)
            .await
            .map_err(Into::<ScheduleApiError>::into)
            .map(|x| ScheduleWrapper::from(x).0)?;

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
        let id = parse_uuid(&req.schedule_id, "schedule_id")?;

        self.store
            .delete_schedule(id, tenant_id)
            .await
            .map_err(Into::<ScheduleApiError>::into)?;

        Ok(Response::new(EmptyResponse {}))
    }
}

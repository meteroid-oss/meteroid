use common_domain::ids::SubscriptionId;
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::renewals::v1::{
    GetRenewalSettingsRequest, GetRenewalSettingsResponse, GetSubscriptionRenewalRequest,
    GetSubscriptionRenewalResponse, UpdateRenewalSettingsRequest, UpdateRenewalSettingsResponse,
    renewals_service_server::RenewalsService,
};
use meteroid_grpc_mapping::renewals::{notice_to_proto, renewal_to_proto, settings_to_proto};
use meteroid_store::errors::StoreError;
use meteroid_store::services::renewals;
use tonic::{Request, Response, Status};

use super::RenewalsServiceComponents;

fn store_status(report: Report<StoreError>) -> Status {
    match report.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::ValueNotFound(message) => Status::not_found(message.clone()),
        _ => {
            log::error!("renewals: {report:?}");
            Status::internal("Error in renewals service")
        }
    }
}

#[tonic::async_trait]
impl RenewalsService for RenewalsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_renewal_settings(
        &self,
        request: Request<GetRenewalSettingsRequest>,
    ) -> Result<Response<GetRenewalSettingsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let settings = renewals::get_settings(&self.store, tenant_id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(GetRenewalSettingsResponse {
            settings: Some(settings_to_proto(settings)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_renewal_settings(
        &self,
        request: Request<UpdateRenewalSettingsRequest>,
    ) -> Result<Response<UpdateRenewalSettingsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let settings = renewals::update_settings(
            &self.store,
            tenant_id,
            req.checkpoints.map(|c| c.days),
            req.customer_reminder.map(|r| r.days),
        )
        .await
        .map_err(store_status)?;
        Ok(Response::new(UpdateRenewalSettingsResponse {
            settings: Some(settings_to_proto(settings)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_subscription_renewal(
        &self,
        request: Request<GetSubscriptionRenewalRequest>,
    ) -> Result<Response<GetSubscriptionRenewalResponse>, Status> {
        let tenant_id = request.tenant()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let today = common_clock::now().date_naive();
        let (renewal, notices) =
            renewals::subscription_renewal(&self.store, tenant_id, subscription_id, today)
                .await
                .map_err(store_status)?;
        Ok(Response::new(GetSubscriptionRenewalResponse {
            renewal: renewal.map(|renewal| renewal_to_proto(renewal, today)),
            notices: notices.into_iter().map(notice_to_proto).collect(),
        }))
    }
}

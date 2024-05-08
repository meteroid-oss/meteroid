use tonic::{Request, Response, Status};
use uuid::Uuid;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceService;
use meteroid_grpc::meteroid::api::instance::v1::{
    GetInstanceRequest, GetInstanceResponse, GetInviteRequest, GetInviteResponse,
    InitInstanceRequest, InitInstanceResponse, Instance,
};
use meteroid_repository as db;
use meteroid_repository::organizations::CreateOrganizationParams;
use meteroid_repository::Params;

use crate::api::instance::error::InstanceApiError;
use crate::api::instance::InstanceServiceComponents;
use crate::api::utils::uuid_gen;
use common_eventbus::Event;

#[tonic::async_trait]
impl InstanceService for InstanceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_instance(
        &self,
        _request: Request<GetInstanceRequest>,
    ) -> Result<Response<GetInstanceResponse>, Status> {
        let connection = self.get_connection().await?;
        let instance = db::organizations::instance()
            .bind(&connection)
            .opt()
            .await
            .map_err(|e| {
                InstanceApiError::DatabaseError("unable to get instance".to_string(), e)
            })?;

        Ok(Response::new(GetInstanceResponse {
            instance: instance.map(|org| Instance {
                company_name: org.name,
                organization_id: org.id.to_string(),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn init_instance(
        &self,
        request: Request<InitInstanceRequest>,
    ) -> Result<Response<InitInstanceResponse>, Status> {
        let actor = request.actor()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let org = db::organizations::create_organization()
            .params(
                &connection,
                &CreateOrganizationParams {
                    id: uuid_gen::v7(),
                    name: req.company_name,
                    slug: "instance",
                },
            )
            .one()
            .await
            .map_err(|e| {
                InstanceApiError::DatabaseError("unable to create instance".to_string(), e)
            })?;

        let _ = self
            .eventbus
            .publish(Event::instance_inited(actor, org.id))
            .await;

        Ok(Response::new(InitInstanceResponse {
            instance: Some(Instance {
                company_name: org.name,
                organization_id: org.id.to_string(),
            }),
        }))
    }

    async fn get_invite(
        &self,
        _request: Request<GetInviteRequest>,
    ) -> Result<Response<GetInviteResponse>, Status> {
        let connection = self.get_connection().await?;
        let instance = db::organizations::instance()
            .bind(&connection)
            .one()
            .await
            .map_err(|e| {
                InstanceApiError::DatabaseError("unable to get instance".to_string(), e)
            })?;

        let invite_opt = db::organizations::get_invite()
            .bind(&connection, &instance.id)
            .one()
            .await
            .map_err(|e| InstanceApiError::DatabaseError("unable to get invite".to_string(), e))?;

        let invite = match invite_opt {
            None => {
                let id = Uuid::new_v4();
                let hash = base62::encode_alternative(id.as_u128());

                db::organizations::set_invite()
                    .bind(&connection, &hash, &instance.id)
                    .await
                    .map_err(|e| {
                        InstanceApiError::DatabaseError("unable to create invite".to_string(), e)
                    })?;

                hash
            }
            Some(hash) => hash,
        };

        Ok(Response::new(GetInviteResponse {
            invite_hash: invite,
        }))
    }
}

use secrecy::{ExposeSecret, SecretString};
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{
    users_service_server::UsersService, FindUserByEmailRequest, FindUserByEmailResponse,
    GetUserByIdRequest, GetUserByIdResponse, ListUsersRequest, ListUsersResponse, LoginRequest,
    LoginResponse, MeRequest, MeResponse, RegisterRequest, RegisterResponse,
};
use meteroid_store::domain::users::{LoginUserRequest, RegisterUserRequest};
use meteroid_store::repositories::TenantInterface;
use meteroid_store::repositories::users::UserInterface;

use crate::api::users::error::UserApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, UsersServiceComponents};

#[tonic::async_trait]
impl UsersService for UsersServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn me(&self, request: Request<MeRequest>) -> Result<Response<MeResponse>, Status> {
        let actor = request.actor()?;

        let me = self
            .store
            .me(actor)
            .await
            .map(mapping::user::domain_to_proto)
            .map_err(Into::<UserApiError>::into)?;

        let response = MeResponse { user: Some(me) };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_user_by_id(
        &self,
        request: Request<GetUserByIdRequest>,
    ) -> Result<Response<GetUserByIdResponse>, Status> {
        let actor = request.actor()?;

        let req = request.into_inner();

        let user = self
            .store
            .find_user_by_id(parse_uuid!(&req.id)?, actor)
            .await
            .map(mapping::user::domain_to_proto)
            .map_err(Into::<UserApiError>::into)?;

        let response = GetUserByIdResponse { user: Some(user) };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let tenant = request.tenant()?;
        let tenant = self.store.find_tenant_by_id(tenant).await.map_err(Into::<UserApiError>::into)?;

        let users = self
            .store
            .list_users_for_organization(tenant.organization_id)
            .await
            .map_err(Into::<UserApiError>::into)?
            .into_iter()
            .map(mapping::user::domain_to_proto)
            .collect();

        let response = ListUsersResponse { users };

        Ok(Response::new(response))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();

            let resp = self
                .store
                .login_user(LoginUserRequest {
                    email: req.email,
                    password: SecretString::new(req.password),
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(LoginResponse {
                token: resp.token.expose_secret().clone(),
                user: Some(mapping::user::domain_to_proto(resp.user)),
            }))
        })
            .await
    }

    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();

            let resp = self
                .store
                .register_user(RegisterUserRequest {
                    email: req.email,
                    password: SecretString::new(req.password),
                    invite_key: req.invite_key.map(SecretString::new),
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(RegisterResponse {
                token: resp.token.expose_secret().clone(),
                user: Some(mapping::user::domain_to_proto(resp.user)),
            }))
        })
            .await
    }
}

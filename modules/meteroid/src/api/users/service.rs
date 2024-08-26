use secrecy::{ExposeSecret, SecretString};
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{users_service_server::UsersService, GetUserByIdRequest, GetUserByIdResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse, MeRequest, MeResponse, RegisterRequest, RegisterResponse, OnboardMeRequest, OnboardMeResponse};
use meteroid_store::domain::users::{LoginUserRequest, RegisterUserRequest, UpdateUser};
use meteroid_store::repositories::users::UserInterface;

use crate::api::users::error::UserApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, UsersServiceComponents};

#[tonic::async_trait]
impl UsersService for UsersServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn me(&self, request: Request<MeRequest>) -> Result<Response<MeResponse>, Status> {
        let actor = request.actor()?;
        let organization = request.organization()
            .ok();

        let me = self
            .store
            .me(actor, organization)
            .await
            .map(mapping::user::me_to_proto)
            .map_err(Into::<UserApiError>::into)?;


        Ok(Response::new(me))
    }

    #[tracing::instrument(skip_all)]
    async fn onboard_me(&self, request: Request<OnboardMeRequest>) -> Result<Response<OnboardMeResponse>, Status> {
        let actor = request.actor()?;
 
        let request = request.into_inner();

        let data = UpdateUser {
            first_name: request.first_name,
            last_name: request.last_name,
            department: request.department,
            know_us_from: request.know_us_from,
        };

        let me = self
            .store
            .update_user_details(actor, data)
            .await
            .map(mapping::user::domain_to_proto)
            .map_err(Into::<UserApiError>::into)?;


        Ok(Response::new(OnboardMeResponse {
            user: Some(me)
        }))
    }


    #[tracing::instrument(skip_all)]
    async fn get_user_by_id(
        &self,
        request: Request<GetUserByIdRequest>,
    ) -> Result<Response<GetUserByIdResponse>, Status> {
        let tenant = request.tenant()?;

        let req = request.into_inner();

        let user = self
            .store
            .find_user_by_id_and_tenant(parse_uuid!(&req.id)?, tenant)
            .await
            .map(mapping::user::domain_with_role_to_proto)
            .map_err(Into::<UserApiError>::into)?;

        let response = GetUserByIdResponse { user: Some(user) };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let organization = request.organization()?;

        let users = self
            .store
            .list_users_for_organization(organization)
            .await
            .map_err(Into::<UserApiError>::into)?
            .into_iter()
            .map(mapping::user::domain_with_role_to_proto)
            .collect();

        let response = ListUsersResponse { users };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
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

    #[tracing::instrument(skip_all)]
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

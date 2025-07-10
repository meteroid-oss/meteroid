use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{
    CompleteRegistrationRequest, CompleteRegistrationResponse, GetUserByIdRequest,
    GetUserByIdResponse, InitRegistrationRequest, InitRegistrationResponse,
    InitResetPasswordRequest, InitResetPasswordResponse, ListUsersRequest, ListUsersResponse,
    LoginRequest, LoginResponse, MeRequest, MeResponse, OnboardMeRequest, OnboardMeResponse,
    ResetPasswordRequest, ResetPasswordResponse, users_service_server::UsersService,
};
use meteroid_store::domain::users::{LoginUserRequest, RegisterUserRequest, UpdateUser};
use meteroid_store::repositories::users::UserInterface;
use secrecy::{ExposeSecret, SecretString};
use tonic::{Request, Response, Status};
use validator::{ValidateEmail, ValidateLength};

use crate::api::users::error::UserApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{UsersServiceComponents, mapping};

/// **Modifying this service ?**
/// Make sure to update **api_layer.ANONYMOUS_SERVICES** in meteroid-middleware if any anonymous rpc is updated/added
#[tonic::async_trait]
impl UsersService for UsersServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn me(&self, request: Request<MeRequest>) -> Result<Response<MeResponse>, Status> {
        let actor = request.actor()?;
        let organization = request.organization().ok();

        let me = self
            .store
            .me(actor, organization)
            .await
            .map(mapping::user::me_to_proto)
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(me))
    }

    #[tracing::instrument(skip_all)]
    async fn onboard_me(
        &self,
        request: Request<OnboardMeRequest>,
    ) -> Result<Response<OnboardMeResponse>, Status> {
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

        Ok(Response::new(OnboardMeResponse { user: Some(me) }))
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
    async fn init_reset_password(
        &self,
        request: Request<InitResetPasswordRequest>,
    ) -> Result<Response<InitResetPasswordResponse>, Status> {
        self.store
            .init_reset_password(request.into_inner().email)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(InitResetPasswordResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn reset_password(
        &self,
        request: Request<ResetPasswordRequest>,
    ) -> Result<Response<ResetPasswordResponse>, Status> {
        let inner = request.into_inner();

        if !inner.new_password.validate_length(Some(8), Some(64), None) {
            return Err(UserApiError::InvalidArgument(
                "Password must be between 8 and 64 characters long".to_string(),
            )
            .into());
        }

        self.store
            .reset_password(
                SecretString::new(inner.token),
                SecretString::new(inner.new_password),
            )
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(ResetPasswordResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn init_registration(
        &self,
        request: Request<InitRegistrationRequest>,
    ) -> Result<Response<InitRegistrationResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();

            if !req.email.validate_email() {
                return Err(
                    UserApiError::InvalidArgument("Invalid email format".to_string()).into(),
                );
            }

            let resp = self
                .store
                .init_registration(req.email, req.invite_key.map(SecretString::new))
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(InitRegistrationResponse {
                validation_required: resp.validation_required,
            }))
        })
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn complete_registration(
        &self,
        request: Request<CompleteRegistrationRequest>,
    ) -> Result<Response<CompleteRegistrationResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();

            if !req.password.validate_length(Some(8), Some(64), None) {
                return Err(UserApiError::InvalidArgument(
                    "Password must be between 8 and 64 characters long".to_string(),
                )
                .into());
            }

            if !req.email.is_empty() && !req.email.validate_email() {
                return Err(
                    UserApiError::InvalidArgument("Invalid email format".to_string()).into(),
                );
            }

            let resp = self
                .store
                .complete_registration(RegisterUserRequest {
                    email: req.email,
                    password: Some(SecretString::new(req.password)),
                    invite_key: req.invite_key.map(SecretString::new),
                    email_validation_token: req.validation_token.map(SecretString::new),
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(CompleteRegistrationResponse {
                token: resp.token.expose_secret().clone(),
                user: Some(mapping::user::domain_to_proto(resp.user)),
            }))
        })
        .await
    }
}

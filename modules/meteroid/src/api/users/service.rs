use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{
    AcceptInviteRequest, AcceptInviteResponse, CompleteRegistrationRequest,
    CompleteRegistrationResponse, GetUserByIdRequest, GetUserByIdResponse, InitRegistrationRequest,
    InitRegistrationResponse, InitResetPasswordRequest, InitResetPasswordResponse,
    InviteMemberRequest, InviteMemberResponse, LeaveOrganizationRequest, LeaveOrganizationResponse,
    ListPendingInvitesRequest, ListPendingInvitesResponse, ListUsersRequest, ListUsersResponse,
    LoginRequest, LoginResponse, MeRequest, MeResponse, OnboardMeRequest, OnboardMeResponse,
    OrganizationInvite as GrpcOrganizationInvite, OrganizationUserRole as GrpcOrganizationUserRole,
    RemoveMemberRequest, RemoveMemberResponse, ResendInviteRequest, ResendInviteResponse,
    ResetPasswordRequest, ResetPasswordResponse, RevokeInviteRequest, RevokeInviteResponse,
    users_service_server::UsersService,
};
use meteroid_store::domain::users::{LoginUserRequest, RegisterUserRequest, UpdateUser};
use meteroid_store::repositories::organizations::OrganizationsInterface;
use meteroid_store::repositories::users::UserInterface;
use secrecy::{ExposeSecret, SecretString};
use tonic::{Request, Response, Status};
use validator::{ValidateEmail, ValidateLength};

use crate::api::shared::conversions::ProtoConv;
use crate::api::users::error::UserApiError;
use common_domain::ids::OrganizationInviteId;

use super::{UsersServiceComponents, mapping};

/// **Modifying this service ?**
/// Make sure to update **`api_layer.ANONYMOUS_SERVICES`** in meteroid-middleware if any anonymous rpc is updated/added
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

        let user_id = common_domain::ids::UserId::from_proto(&req.id)?;
        let user = self
            .store
            .find_user_by_id_and_tenant(common_domain::ids::BaseId::as_uuid(&user_id), tenant)
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
                    password: SecretString::from(req.password),
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(LoginResponse {
                token: resp.token.expose_secret().to_string(),
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
                SecretString::from(inner.token),
                SecretString::from(inner.new_password),
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
                .init_registration(
                    req.email,
                    req.invite_key.map(SecretString::from),
                    req.return_path,
                )
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

            // we validate email only if we don't have a validation token (as req.email is empty otherwise)
            if req.validation_token.is_none() && !req.email.validate_email() {
                return Err(
                    UserApiError::InvalidArgument("Invalid email format".to_string()).into(),
                );
            }

            let resp = self
                .store
                .complete_registration(RegisterUserRequest {
                    email: req.email,
                    password: Some(SecretString::from(req.password)),
                    invite_key: req.invite_key.map(SecretString::from),
                    email_validation_token: req.validation_token.map(SecretString::from),
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(CompleteRegistrationResponse {
                token: resp.token.expose_secret().to_string(),
                user: Some(mapping::user::domain_to_proto(resp.user)),
            }))
        })
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn accept_invite(
        &self,
        request: Request<AcceptInviteRequest>,
    ) -> Result<Response<AcceptInviteResponse>, Status> {
        let actor = request.actor()?;
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        let organization = self
            .store
            .accept_invite(actor, invite_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(AcceptInviteResponse {
            organization: Some(
                super::super::organizations::mapping::organization::domain_to_proto(organization),
            ),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn invite_member(
        &self,
        request: Request<InviteMemberRequest>,
    ) -> Result<Response<InviteMemberResponse>, Status> {
        let actor = request.actor()?;
        let org_id = request.organization()?;
        request.require_admin()?;
        let req = request.into_inner();

        let role = GrpcOrganizationUserRole::try_from(req.role)
            .map_err(|_| Status::invalid_argument("Invalid role"))?;
        let role = mapping::role::server_to_domain(role);

        let invite = self
            .store
            .invite_member(org_id, actor, req.email, role.into())
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(InviteMemberResponse {
            invite_id: invite.id.to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resend_invite(
        &self,
        request: Request<ResendInviteRequest>,
    ) -> Result<Response<ResendInviteResponse>, Status> {
        let actor = request.actor()?;
        let org_id = request.organization()?;
        request.require_admin()?;
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        self.store
            .resend_invite(invite_id, actor, org_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(ResendInviteResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn revoke_invite(
        &self,
        request: Request<RevokeInviteRequest>,
    ) -> Result<Response<RevokeInviteResponse>, Status> {
        let org_id = request.organization()?;
        request.require_admin()?;
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        self.store
            .revoke_invite(invite_id, org_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(RevokeInviteResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_pending_invites(
        &self,
        request: Request<ListPendingInvitesRequest>,
    ) -> Result<Response<ListPendingInvitesResponse>, Status> {
        let org_id = request.organization()?;
        request.require_admin()?;

        let invites = self
            .store
            .list_pending_invites(org_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        let grpc_invites = invites
            .into_iter()
            .map(|inv| GrpcOrganizationInvite {
                id: inv.id.as_proto(),
                invited_email: inv.invited_email,
                role: mapping::role::domain_to_server(inv.role.into()).into(),
                invited_by_email: inv.invited_by_email,
                created_at: inv.created_at.as_proto(),
                expires_at: inv.expires_at.as_proto(),
                is_expired: inv.is_expired,
            })
            .collect();

        Ok(Response::new(ListPendingInvitesResponse {
            invites: grpc_invites,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn leave_organization(
        &self,
        request: Request<LeaveOrganizationRequest>,
    ) -> Result<Response<LeaveOrganizationResponse>, Status> {
        let actor = request.actor()?;
        let org_id = request.organization()?;

        self.store
            .leave_organization(actor, org_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(LeaveOrganizationResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_member(
        &self,
        request: Request<RemoveMemberRequest>,
    ) -> Result<Response<RemoveMemberResponse>, Status> {
        let actor = request.actor()?;
        let org_id = request.organization()?;
        let req = request.into_inner();

        let target_user_id = *common_domain::ids::UserId::from_proto(&req.user_id)?;

        self.store
            .remove_member(actor, target_user_id, org_id)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(RemoveMemberResponse {}))
    }
}

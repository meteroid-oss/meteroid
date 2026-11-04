use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{
    AcceptInviteRequest, AcceptInviteResponse, CompleteRegistrationRequest,
    CompleteRegistrationResponse, ConfirmMfaEnrollmentRequest, ConfirmMfaEnrollmentResponse,
    DisableMfaRequest, DisableMfaResponse, GetUserByIdRequest, GetUserByIdResponse,
    InitMfaEnrollmentRequest, InitMfaEnrollmentResponse, InitRegistrationRequest,
    InitRegistrationResponse, InitResetPasswordRequest, InitResetPasswordResponse,
    InviteMemberRequest, InviteMemberResponse, LeaveOrganizationRequest, LeaveOrganizationResponse,
    ListPendingInvitesRequest, ListPendingInvitesResponse, ListUsersRequest, ListUsersResponse,
    LoginRequest, LoginResponse, MeRequest, MeResponse, OnboardMeRequest, OnboardMeResponse,
    OrganizationInvite as GrpcOrganizationInvite, OrganizationUserRole as GrpcOrganizationUserRole,
    RegenerateMfaRecoveryCodesRequest, RegenerateMfaRecoveryCodesResponse, RemoveMemberRequest,
    RemoveMemberResponse, ResendInviteRequest, ResendInviteResponse, ResetPasswordRequest,
    ResetPasswordResponse, RevokeInviteRequest, RevokeInviteResponse, UpdateUserPreferencesRequest,
    UpdateUserPreferencesResponse, VerifyMfaRequest, VerifyMfaResponse,
    users_service_server::UsersService,
};
use meteroid_store::domain::users::{
    LoginUserRequest, LoginUserResponse, RegisterUserRequest, UpdateUser,
};
use meteroid_store::repositories::organizations::OrganizationsInterface;
use meteroid_store::repositories::users::UserInterface;
use secrecy::{ExposeSecret, SecretString};
use tonic::{Request, Response, Status};
use validator::{ValidateEmail, ValidateLength};

use super::{UsersServiceComponents, mapping};
use crate::users::error::UserApiError;
use common_domain::ids::{OrganizationInviteId, RoleId, UserId};
use meteroid_app::conversions::ProtoConv;
use meteroid_app::features::gates;
use meteroid_app::security_context;

/// **Modifying this service ?**
/// Make sure to update **`api_layer.ANONYMOUS_SERVICES`** in meteroid-middleware if any anonymous rpc is updated/added
#[tonic::async_trait]
impl UsersService for UsersServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn me(&self, request: Request<MeRequest>) -> Result<Response<MeResponse>, Status> {
        let actor = request.actor_user()?;

        let me = self
            .store
            .me(actor)
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
        let actor = request.actor_user()?;

        let request = request.into_inner();

        let data = UpdateUser {
            first_name: request.first_name,
            last_name: request.last_name,
            department: request.department,
            heard_from: request.heard_from,
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
    async fn update_user_preferences(
        &self,
        request: Request<UpdateUserPreferencesRequest>,
    ) -> Result<Response<UpdateUserPreferencesResponse>, Status> {
        let actor = request.actor_user()?;

        let req = request.into_inner();

        let patch: serde_json::Value = serde_json::from_str(&req.preferences)
            .map_err(|e| Status::invalid_argument(format!("invalid preferences JSON: {e}")))?;

        if !patch.is_object() {
            return Err(Status::invalid_argument(
                "preferences must be a JSON object",
            ));
        }

        let merged = self
            .store
            .update_user_preferences(actor, patch)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(UpdateUserPreferencesResponse {
            preferences: serde_json::to_string(&merged).unwrap_or_else(|_| "{}".to_string()),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_user_by_id(
        &self,
        request: Request<GetUserByIdRequest>,
    ) -> Result<Response<GetUserByIdResponse>, Status> {
        let tenant = request.tenant()?;

        let req = request.into_inner();

        let user_id = UserId::from_proto(&req.id)?;
        let user = self
            .store
            .find_user_by_id_and_tenant(user_id, tenant)
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
            let request_context = security_context::from_request(&request);
            let req = request.into_inner();

            self.auth_throttle
                .check_login(&request_context, &req.email)
                .await?;

            let resp = self
                .store
                .login_user(LoginUserRequest {
                    email: req.email,
                    password: SecretString::from(req.password),
                    request_context,
                })
                .await
                .map_err(Into::<UserApiError>::into)?;

            Ok(Response::new(login_response_to_proto(resp)))
        })
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn verify_mfa(
        &self,
        request: Request<VerifyMfaRequest>,
    ) -> Result<Response<VerifyMfaResponse>, Status> {
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        // The challenge token's `sub` is the user id. We don't decode the JWT
        // here (the store does), but `sub` is opaque to us — we can pull the
        // first segment as a fingerprint for the per-user bucket. This is the
        // anti-MFA-brute-force key (10^6 codes; without per-user throttle one
        // IP could try the entire space in ~3 weeks).
        let challenge_fingerprint = challenge_user_fingerprint(&req.mfa_challenge_token);
        self.auth_throttle
            .check_verify_mfa(&request_context, &challenge_fingerprint)
            .await?;

        let resp = self
            .store
            .verify_mfa_login(
                SecretString::from(req.mfa_challenge_token),
                &req.code,
                request_context,
            )
            .await
            .map_err(Into::<UserApiError>::into)?;

        match resp {
            LoginUserResponse::Authenticated { token, user } => {
                Ok(Response::new(VerifyMfaResponse {
                    token: token.expose_secret().to_string(),
                    user: Some(mapping::user::domain_to_proto(user)),
                }))
            }
            LoginUserResponse::MfaChallenge { .. } => Err(UserApiError::AuthenticationError(
                "MFA verification did not produce a session".to_string(),
            )
            .into()),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn init_mfa_enrollment(
        &self,
        request: Request<InitMfaEnrollmentRequest>,
    ) -> Result<Response<InitMfaEnrollmentResponse>, Status> {
        let actor = request.actor_user()?;

        let init = self
            .store
            .init_mfa_enrollment(actor)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(InitMfaEnrollmentResponse {
            secret_base32: init.secret_base32,
            otpauth_uri: init.otpauth_uri.expose_secret().to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_mfa_enrollment(
        &self,
        request: Request<ConfirmMfaEnrollmentRequest>,
    ) -> Result<Response<ConfirmMfaEnrollmentResponse>, Status> {
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let activated = self
            .store
            .confirm_mfa_enrollment(actor, &req.code)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(ConfirmMfaEnrollmentResponse {
            recovery_codes: activated.recovery_codes,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn disable_mfa(
        &self,
        request: Request<DisableMfaRequest>,
    ) -> Result<Response<DisableMfaResponse>, Status> {
        let actor = request.actor_user()?;
        let req = request.into_inner();

        self.store
            .disable_mfa(actor, SecretString::from(req.password))
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(DisableMfaResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn regenerate_mfa_recovery_codes(
        &self,
        request: Request<RegenerateMfaRecoveryCodesRequest>,
    ) -> Result<Response<RegenerateMfaRecoveryCodesResponse>, Status> {
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let activated = self
            .store
            .regenerate_mfa_recovery_codes(actor, SecretString::from(req.password))
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(RegenerateMfaRecoveryCodesResponse {
            recovery_codes: activated.recovery_codes,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn init_reset_password(
        &self,
        request: Request<InitResetPasswordRequest>,
    ) -> Result<Response<InitResetPasswordResponse>, Status> {
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        self.auth_throttle
            .check_init_reset_password(&request_context, &req.email)
            .await?;

        self.store
            .init_reset_password(req.email, request_context)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(InitResetPasswordResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn reset_password(
        &self,
        request: Request<ResetPasswordRequest>,
    ) -> Result<Response<ResetPasswordResponse>, Status> {
        let request_context = security_context::from_request(&request);
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
                request_context,
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
            let request_context = security_context::from_request(&request);
            let req = request.into_inner();

            if !req.email.validate_email() {
                return Err(
                    UserApiError::InvalidArgument("Invalid email format".to_string()).into(),
                );
            }

            if !self
                .store
                .organization_hooks
                .accepts_signup_email(&req.email)
                .await
            {
                return Err(UserApiError::InvalidArgument(
                    "Please use your work email address.".to_string(),
                )
                .into());
            }

            self.auth_throttle
                .check_init_registration(&request_context, &req.email)
                .await?;

            let resp = self
                .store
                .init_registration(
                    req.email,
                    req.invite_key.map(SecretString::from),
                    req.return_path,
                    req.signup_source.and_then(sanitize_signup_source),
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
            let request_context = security_context::from_request(&request);
            let req = request.into_inner();

            if !req.password.validate_length(Some(8), Some(64), None) {
                return Err(UserApiError::InvalidArgument(
                    "Password must be between 8 and 64 characters long".to_string(),
                )
                .into());
            }

            // we validate email only if we don't have a validation token (as req.email is empty otherwise)
            // and not registering via invite (admin already approved the email)
            if req.validation_token.is_none() && req.invite_key.is_none() {
                if !req.email.validate_email() {
                    return Err(
                        UserApiError::InvalidArgument("Invalid email format".to_string()).into(),
                    );
                }

                if self.store.settings.mailer_enabled
                    && !self
                        .store
                        .organization_hooks
                        .accepts_signup_email(&req.email)
                        .await
                {
                    return Err(UserApiError::InvalidArgument(
                        "Please use your work email address.".to_string(),
                    )
                    .into());
                }
            }

            // The per-email key falls back to the validation_token when the
            // user submits via the email-link flow (req.email is empty).
            let throttle_key = if !req.email.is_empty() {
                req.email.clone()
            } else if let Some(tok) = req.validation_token.as_deref() {
                tok.to_string()
            } else {
                String::new()
            };
            if !throttle_key.is_empty() {
                self.auth_throttle
                    .check_complete_registration(&request_context, &throttle_key)
                    .await?;
            }

            let resp = self
                .store
                .complete_registration(
                    RegisterUserRequest {
                        email: req.email,
                        password: Some(SecretString::from(req.password)),
                        invite_key: req.invite_key.map(SecretString::from),
                        email_validation_token: req.validation_token.map(SecretString::from),
                    },
                    request_context,
                )
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
        let actor = request.actor_user()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        let organization = self
            .store
            .accept_invite(actor, invite_id, request_context)
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
        // Authorization (members:manage) is enforced centrally by the RBAC
        // middleware, so custom governance roles can invite too — not just admins.
        let actor = request.actor_user()?;
        let org_id = request.organization()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        let role = GrpcOrganizationUserRole::try_from(req.role)
            .map_err(|_| Status::invalid_argument("Invalid role"))?;
        let role = mapping::role::server_to_domain(role);

        let custom_role_id = RoleId::from_proto_opt(req.custom_role_id)?;
        // Granting a custom role requires the RBAC feature; plain tiers do not.
        if custom_role_id.is_some()
            && !self
                .feature_resolver
                .resolve(org_id)
                .await
                .is_enabled(gates::RBAC)
        {
            return Err(Status::permission_denied(
                "Custom roles are not enabled for this organization.",
            ));
        }

        let invite = self
            .store
            .invite_member(
                org_id,
                actor,
                req.email,
                role.into(),
                custom_role_id,
                request_context,
            )
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
        let actor = request.actor_user()?;
        let org_id = request.organization()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        self.store
            .resend_invite(invite_id, actor, org_id, request_context)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(ResendInviteResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn revoke_invite(
        &self,
        request: Request<RevokeInviteRequest>,
    ) -> Result<Response<RevokeInviteResponse>, Status> {
        let actor = request.actor_user()?;
        let org_id = request.organization()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        let invite_id = OrganizationInviteId::from_proto(&req.invite_id)?;

        self.store
            .revoke_invite(invite_id, org_id, actor, request_context)
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
                custom_role_id: inv.custom_role_id.map(|id| id.as_proto()),
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
        let actor = request.actor_user()?;
        let org_id = request.organization()?;
        let request_context = security_context::from_request(&request);

        self.store
            .leave_organization(actor, org_id, request_context)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(LeaveOrganizationResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_member(
        &self,
        request: Request<RemoveMemberRequest>,
    ) -> Result<Response<RemoveMemberResponse>, Status> {
        let actor = request.actor_user()?;
        let org_id = request.organization()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();

        let target_user_id = UserId::from_proto(&req.user_id)?;

        self.store
            .remove_member(actor, target_user_id, org_id, request_context)
            .await
            .map_err(Into::<UserApiError>::into)?;

        Ok(Response::new(RemoveMemberResponse {}))
    }
}

const MAX_SIGNUP_SOURCE_LEN: usize = 64;

/// Drops invalid tags to `None` instead of failing the registration.
fn sanitize_signup_source(source: String) -> Option<String> {
    let source = source.trim();
    let valid = !source.is_empty()
        && source.len() <= MAX_SIGNUP_SOURCE_LEN
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    valid.then(|| source.to_string())
}

fn login_response_to_proto(resp: LoginUserResponse) -> LoginResponse {
    match resp {
        LoginUserResponse::Authenticated { token, user } => LoginResponse {
            token: token.expose_secret().to_string(),
            user: Some(mapping::user::domain_to_proto(user)),
            mfa_challenge_token: None,
        },
        LoginUserResponse::MfaChallenge { challenge_token } => LoginResponse {
            token: String::new(),
            user: None,
            mfa_challenge_token: Some(challenge_token.expose_secret().to_string()),
        },
    }
}

/// Cheap "who is being challenged" fingerprint for the per-user MFA throttle.
/// Reads the JWT payload segment without verifying the signature — we only
/// need a stable per-user key for rate-limit bucketing. If parsing fails (the
/// store will reject the token anyway), fall back to the raw token so the
/// limiter still buckets distinct attempts.
fn challenge_user_fingerprint(token: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    if let Some(payload_b64) = token.split('.').nth(1)
        && let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload_b64)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(sub) = v.get("sub").and_then(|s| s.as_str())
    {
        return sub.to_string();
    }
    token.to_string()
}

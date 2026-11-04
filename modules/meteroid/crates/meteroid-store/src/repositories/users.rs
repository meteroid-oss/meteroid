use crate::domain::Organization;
use crate::domain::oauth::{OauthConnection, OauthVerifierData};
use crate::domain::users::{
    InitRegistrationResponse, LoginUserRequest, LoginUserResponse, Me, MfaActivated,
    MfaEnrollmentInit, RegisterUserRequest, RegisterUserRequestInternal, RegisterUserResponse,
    UpdateUser, User, UserWithRole,
};
use crate::errors::StoreError;
use crate::repositories::oauth::OauthInterface;
use crate::repositories::org_security_audit_log::SecurityAuditInterface;
use crate::repositories::sso::SsoInterface;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use common_domain::auth::{Audience, JwtClaims, JwtPayload};
use common_domain::ids::{BaseId, OrganizationId, OrganizationInviteId, TenantId, UserId};
use common_eventbus::Event;
use diesel_models::organization_invites::OrganizationInviteRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::OrganizationRow;
use diesel_models::users::{UserRow, UserRowNew, UserRowPatch};
use error_stack::{Report, ResultExt, bail};
use jsonwebtoken::{DecodingKey, Validation};
use meteroid_mailer::model::{EmailRecipient, EmailValidationLink, ResetPasswordLink};
use meteroid_oauth::model::OauthProvider;
use scoped_futures::ScopedFutureExt;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashSet;
use tracing::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait UserInterface {
    async fn init_registration(
        &self,
        email: String,
        invite_key: Option<SecretString>,
        return_path: Option<String>,
        signup_source: Option<String>,
    ) -> StoreResult<InitRegistrationResponse>;
    async fn complete_registration(
        &self,
        req: RegisterUserRequest,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<RegisterUserResponse>;

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse>;
    async fn me(&self, auth_user_id: UserId) -> StoreResult<Me>;
    async fn update_user_details(
        &self,
        auth_user_id: UserId,
        data: UpdateUser,
    ) -> StoreResult<User>;
    async fn mark_user_onboarded(&self, user_id: UserId) -> StoreResult<()>;
    /// Shallow-merges `patch` into the user's preferences and returns the merged object.
    async fn update_user_preferences(
        &self,
        auth_user_id: UserId,
        patch: serde_json::Value,
    ) -> StoreResult<serde_json::Value>;
    // async fn update_user_role(&self, auth_user_id: UserId, organization_id: OrganizationId, data: UpdateUserRole) -> StoreResult<User>;

    async fn find_user_by_id_and_organization(
        &self,
        id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole>;
    async fn find_user_by_id_and_tenant(
        &self,
        id: UserId,
        tenant_id: TenantId,
    ) -> StoreResult<UserWithRole>;

    async fn find_user_by_email_and_organization(
        &self,
        email: String,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole>;

    /// Instance bootstrap: the seed user, created on first run and returned as is
    /// afterwards. Outside the registration flow on purpose — no mailer round-trip, no
    /// invite and no closed-registration check, as it comes from startup env config.
    ///
    /// The flag reports whether the user was created, so a caller can say that a supplied
    /// password was not applied to an account it merely adopted.
    async fn find_or_create_instance_user(
        &self,
        email: String,
        password: SecretString,
    ) -> StoreResult<(User, bool)>;

    async fn list_users_for_organization(
        &self,
        org_id: OrganizationId,
    ) -> StoreResult<Vec<UserWithRole>>;

    /** Internal use only. For API/external, use `me()` or `find_user_by_id_and_organization()` */
    async fn _find_user_by_id(&self, id: UserId) -> StoreResult<User>;

    /// Platform staff among `user_ids` who aren't members of `org_id`: actors of a support
    /// session, to be shown anonymously to the organization.
    async fn support_staff_outside_org(
        &self,
        org_id: OrganizationId,
        user_ids: &[UserId],
    ) -> StoreResult<HashSet<UserId>>;

    async fn init_reset_password(
        &self,
        email: String,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn reset_password(
        &self,
        token: SecretString,
        new_password: SecretString,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn oauth_signin(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse>;

    /// Initiate TOTP enrollment for the authenticated user. Persists a pending
    /// (not-yet-activated) secret and returns it along with an otpauth:// URI
    /// for QR-code rendering. Calling this for a user with MFA already active
    /// is a no-op error.
    async fn init_mfa_enrollment(&self, user_id: UserId) -> StoreResult<MfaEnrollmentInit>;

    /// Confirm enrollment by verifying a TOTP code generated from the pending
    /// secret, then flip MFA on and return one-time-use plaintext recovery
    /// codes (only hashes are persisted; the caller must show them ONCE).
    async fn confirm_mfa_enrollment(
        &self,
        user_id: UserId,
        code: &str,
    ) -> StoreResult<MfaActivated>;

    /// Disable MFA for the authenticated user. Requires re-confirming the
    /// password to prevent session-hijack attackers from turning it off.
    async fn disable_mfa(&self, user_id: UserId, password: SecretString) -> StoreResult<()>;

    /// Regenerate the recovery code set, invalidating any previously issued codes.
    /// Requires password re-confirmation.
    async fn regenerate_mfa_recovery_codes(
        &self,
        user_id: UserId,
        password: SecretString,
    ) -> StoreResult<MfaActivated>;

    /// Verify a TOTP code (or recovery code) against a previously issued MFA
    /// challenge token, returning an authenticated session.
    async fn verify_mfa_login(
        &self,
        challenge_token: SecretString,
        code: &str,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse>;
}

#[async_trait::async_trait]
impl UserInterface for Store {
    async fn complete_registration(
        &self,
        req: RegisterUserRequest,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<RegisterUserResponse> {
        if !self.settings.mailer_enabled {
            validate_domain(&req.email, &self.settings.domains_whitelist)?;

            return register_user_internal(
                self,
                RegisterUserRequestInternal {
                    password: req.password,
                    email: req.email,
                    invite_key: req.invite_key,
                    signup_source: None,
                },
                request_context,
            )
            .await;
        }

        let token = match req.email_validation_token {
            Some(token) => token.expose_secret().to_string(),
            None => {
                bail!(StoreError::InvalidArgument(
                    "email validation token is required".into()
                ));
            }
        };

        let mut validation = Validation::default();
        validation.set_audience(&[Audience::EmailValidation.as_str()]);

        let token_data = crate::jwt::decode::<JwtClaims>(
            &token,
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let email = token_data.claims.sub.as_str();

        let (invite_key, signup_source) = match token_data.claims.payload {
            Some(JwtPayload::EmailValidation {
                invite_key,
                signup_source,
            }) => (invite_key, signup_source),
            _ => (None, None),
        };

        register_user_internal(
            self,
            RegisterUserRequestInternal {
                password: req.password,
                email: email.to_string(),
                invite_key: invite_key.map(SecretString::from),
                signup_source,
            },
            request_context,
        )
        .await
    }

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse> {
        let mut conn = self.get_conn().await?;

        validate_domain(&req.email, &self.settings.domains_whitelist)?;
        // SSO enforcement takes precedence over password auth — an enforced org
        // disables the password path entirely, before any credential check.
        ensure_not_sso_enforced(self, &req.email).await?;

        let ctx = req.request_context;

        let user = UserRow::find_by_email(&mut conn, req.email.clone()).await?;

        let Some(user) = user else {
            // No org context to attribute this to; we drop the event rather
            // than guessing. Failed credential-stuffing across unknown emails
            // belongs in a global infra log, not an org's compliance trail.
            bail!(StoreError::LoginError(
                "incorrect email and/or password".into(),
            ));
        };

        let password_hash = user.password_hash.clone().ok_or(StoreError::LoginError(
            "Password is not set. Login is forbidden".into(),
        ))?;

        let argon2 = Argon2::default();

        let db_hash_parsed = PasswordHash::new(&password_hash)
            .map_err(|_| StoreError::InvalidArgument("password hash".to_string()))?;

        let user_email = user.email.clone();
        let user_id_typed = user.id;

        if argon2
            .verify_password(req.password.expose_secret().as_bytes(), &db_hash_parsed)
            .is_err()
        {
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthLoginFailedPassword,
                None,
                ctx,
            )
            .await;
            bail!(StoreError::LoginError(
                "invalid email or password".to_string(),
            ));
        }

        let response = finalize_login(self, &mut conn, user).await?;

        // Record the password-step success regardless of whether an MFA
        // challenge follows. The MFA challenge result is logged separately.
        if matches!(response, LoginUserResponse::Authenticated { .. }) {
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthLoginSucceeded,
                None,
                ctx,
            )
            .await;
        }

        Ok(response)
    }

    async fn me(&self, auth_user_id: UserId) -> StoreResult<Me> {
        let mut conn = self.get_conn().await?;

        let org_rows = OrganizationRow::list_by_user_id(&mut conn, *auth_user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let any_org_requires_mfa = org_rows.iter().any(|o| o.mfa_required);
        let organizations: Vec<Organization> = org_rows.into_iter().map(Into::into).collect();

        let user: User = UserRow::find_by_id(&mut conn, *auth_user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let mfa_enrollment_required = any_org_requires_mfa && !user.mfa_enabled;

        Ok(Me {
            user,
            organizations,
            mfa_enrollment_required,
        })
    }

    async fn update_user_details(
        &self,
        auth_user_id: UserId,
        data: UpdateUser,
    ) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        let patch = UserRowPatch {
            id: auth_user_id,
            first_name: data.first_name,
            last_name: data.last_name,
            department: data.department.clone(),
            onboarded: Some(true),
        };

        let res = patch
            .update_user(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let _ = self
            .eventbus
            .publish(Event::user_updated(
                Actor::User { id: auth_user_id },
                auth_user_id,
                data.department,
                data.heard_from,
            ))
            .await;

        Ok(res)
    }

    async fn mark_user_onboarded(&self, user_id: UserId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let patch = UserRowPatch {
            id: user_id,
            first_name: None,
            last_name: None,
            department: None,
            onboarded: Some(true),
        };

        patch
            .update_user(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    async fn update_user_preferences(
        &self,
        auth_user_id: UserId,
        patch: serde_json::Value,
    ) -> StoreResult<serde_json::Value> {
        let mut conn = self.get_conn().await?;

        UserRow::merge_preferences(&mut conn, auth_user_id, patch)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn find_user_by_id_and_organization(
        &self,
        id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id_and_org_id(&mut conn, *id, org_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_user_by_id_and_tenant(
        &self,
        id: UserId,
        tenant_id: TenantId,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id_and_tenant_id(&mut conn, *id, tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn support_staff_outside_org(
        &self,
        org_id: OrganizationId,
        user_ids: &[UserId],
    ) -> StoreResult<HashSet<UserId>> {
        let Some(admin_org) = self.settings.admin_organization else {
            return Ok(HashSet::new());
        };
        if user_ids.is_empty() || admin_org == org_id {
            return Ok(HashSet::new());
        }
        let uuids: Vec<Uuid> = user_ids.iter().map(|u| u.as_uuid()).collect();
        let mut conn = self.get_conn().await?;
        let staff = OrganizationMemberRow::filter_members(&mut conn, admin_org, &uuids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if staff.is_empty() {
            return Ok(HashSet::new());
        }
        let members: HashSet<Uuid> =
            OrganizationMemberRow::filter_members(&mut conn, org_id, &staff)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .collect();
        Ok(staff
            .into_iter()
            .filter(|u| !members.contains(u))
            .map(UserId::from)
            .collect())
    }

    async fn find_user_by_email_and_organization(
        &self,
        email: String,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_email_and_org_id(&mut conn, email, org_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_or_create_instance_user(
        &self,
        email: String,
        password: SecretString,
    ) -> StoreResult<(User, bool)> {
        let mut conn = self.get_conn().await?;

        if let Some(existing) = UserRow::find_by_email(&mut conn, email.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            return Ok((existing.into(), false));
        }

        let user_new = UserRowNew {
            id: Uuid::now_v7().into(),
            email,
            password_hash: Some(hash_password(password.expose_secret())?),
        };

        let inserted = user_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into);

        // A replica booting alongside may have won the insert; adopt its user.
        if let Err(err) = inserted {
            if !matches!(err.current_context(), StoreError::DuplicateValue { .. }) {
                return Err(err);
            }
            return UserRow::find_by_email(&mut conn, user_new.email)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .map(|row| (row.into(), false))
                .ok_or(err);
        }

        let _ = self
            .eventbus
            .publish(Event::user_created(None, user_new.id, None))
            .await;

        UserRow::find_by_id(&mut conn, *user_new.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|row| (row.into(), true))
    }

    async fn list_users_for_organization(
        &self,
        org_id: OrganizationId,
    ) -> StoreResult<Vec<UserWithRole>> {
        let mut conn = self.get_conn().await?;

        UserRow::list_by_org_id(&mut conn, org_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn _find_user_by_id(&self, id: UserId) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id(&mut conn, *id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn init_reset_password(
        &self,
        email: String,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let user = {
            let mut conn = self.get_conn().await?;
            UserRow::find_by_email(&mut conn, email.clone())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };

        if let Some(user) = user {
            // todo add expire_in to config
            let url_expires_in = chrono::Duration::minutes(10);

            let token = generate_jwt_token(
                &user.id.as_uuid().to_string(),
                &self.settings.jwt_secret,
                Utc::now() + url_expires_in,
                Audience::ResetPassword,
                None,
            )?;

            let url = SecretString::from(format!(
                "{}/reset-password?token={}",
                self.settings.public_url.as_str(),
                token.expose_secret()
            ));

            let user_id = user.id;
            let user_email = user.email.clone();
            let recipient = EmailRecipient {
                email,
                first_name: user.first_name,
                last_name: user.last_name,
            };

            self.mailer
                .send_reset_password_link(ResetPasswordLink {
                    url,
                    recipient,
                    url_expires_in,
                })
                .await
                .change_context(StoreError::MailServiceError)?;

            log::info!("Reset password email sent for user: {}", user_id);

            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthPasswordResetRequested,
                None,
                request_context,
            )
            .await;
        } else {
            log::warn!("User with email {email} not found");
        }

        Ok(())
    }

    async fn init_registration(
        &self,
        email: String,
        invite_key: Option<SecretString>,
        return_path: Option<String>,
        signup_source: Option<String>,
    ) -> StoreResult<InitRegistrationResponse> {
        validate_domain(&email, &self.settings.domains_whitelist)?;

        let user_opt = {
            let mut conn = self.get_conn().await?;
            UserRow::find_by_email(&mut conn, email.clone()).await?
        };

        if user_opt.is_some() {
            return Err(StoreError::DuplicateValue {
                entity: "user",
                key: None,
            }
            .into());
        }

        if !self.settings.mailer_enabled {
            return Ok(InitRegistrationResponse {
                validation_required: false,
            });
        }

        let url_expires_in = chrono::Duration::hours(24);

        let token = generate_jwt_token(
            &email,
            &self.settings.jwt_secret,
            Utc::now() + url_expires_in,
            Audience::EmailValidation,
            Some(JwtPayload::EmailValidation {
                invite_key: invite_key.map(|s| s.expose_secret().to_string()),
                signup_source,
            }),
        )?;

        let url = if let Some(ref path) = return_path {
            SecretString::from(format!(
                "{}/validate-email?token={}&return_path={}",
                self.settings.public_url.as_str(),
                token.expose_secret(),
                urlencoding::encode(path)
            ))
        } else {
            SecretString::from(format!(
                "{}/validate-email?token={}",
                self.settings.public_url.as_str(),
                token.expose_secret()
            ))
        };

        let recipient = EmailRecipient {
            email,
            first_name: None,
            last_name: None,
        };

        self.mailer
            .send_email_validation_link(EmailValidationLink {
                url,
                recipient,
                url_expires_in,
            })
            .await
            .change_context(StoreError::MailServiceError)?;

        Ok(InitRegistrationResponse {
            validation_required: true,
        })
    }

    async fn reset_password(
        &self,
        token: SecretString,
        new_password: SecretString,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut validation = Validation::default();
        validation.set_audience(&[Audience::ResetPassword.as_str()]);

        let token_data = crate::jwt::decode::<JwtClaims>(
            token.expose_secret(),
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let user_id = Uuid::parse_str(token_data.claims.sub.as_str())
            .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let new_password_hash = hash_password(new_password.expose_secret())?;

        let mut conn = self.get_conn().await?;

        if !UserRow::is_active(&mut conn, UserId::from(user_id))
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            bail!(StoreError::InvalidArgument("invalid token".into()));
        }

        UserRow::update_password_hash(&mut conn, user_id, new_password_hash.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let user = UserRow::find_by_id(&mut conn, user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
            self,
            user.id,
            &user.email,
            crate::domain::org_security_audit_log::SecurityEventType::AuthPasswordResetCompleted,
            None,
            request_context,
        )
        .await;
        Ok(())
    }

    async fn oauth_signin(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse> {
        let OauthConnection {
            user,
            tokens: _,
            verifier_data,
        } = self.oauth_exchange_code(provider, code, state).await?;

        let signin_data = match verifier_data {
            OauthVerifierData::SignIn(data) => data,
            _ => {
                bail!(StoreError::OauthError(
                    "invalid oauth verifier data".to_string()
                ))
            }
        };

        let email = user.email;

        validate_domain(&email, &self.settings.domains_whitelist)?;
        // Social OAuth can't be used to sidestep an org's SSO requirement.
        ensure_not_sso_enforced(self, &email).await?;

        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_email(&mut conn, email.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        match user {
            None => {
                if !signin_data.is_signup {
                    bail!(StoreError::OauthError("User not found".into()))
                }

                let user_new = RegisterUserRequestInternal {
                    email: email.clone(),
                    password: None,
                    invite_key: signin_data.invite_key.map(SecretString::from),
                    signup_source: None,
                };

                drop(conn);
                let res = register_user_internal(self, user_new, request_context).await?;

                Ok(LoginUserResponse::Authenticated {
                    token: res.token,
                    user: res.user,
                })
            }
            Some(user) => {
                if signin_data.is_signup {
                    bail!(StoreError::OauthError("User already exists".into()))
                }

                let user_email = user.email.clone();
                let user_id_typed = user.id;
                let response = finalize_login(self, &mut conn, user).await?;
                drop(conn);

                if matches!(response, LoginUserResponse::Authenticated { .. }) {
                    crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                        self,
                        user_id_typed,
                        &user_email,
                        crate::domain::org_security_audit_log::SecurityEventType::AuthOauthLoginSucceeded,
                        Some(serde_json::json!({"provider": format!("{:?}", provider).to_lowercase()})),
                        request_context,
                    )
                    .await;
                }

                Ok(response)
            }
        }
    }

    async fn init_mfa_enrollment(&self, user_id: UserId) -> StoreResult<MfaEnrollmentInit> {
        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_id(&mut conn, user_id.as_uuid())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if user.totp_enabled {
            bail!(StoreError::InvalidArgument(
                "MFA is already enabled for this user".into()
            ));
        }

        let secret = totp_rs::Secret::generate_secret().to_encoded();
        let secret_base32 = secret.to_string();

        UserRow::set_pending_totp_secret(&mut conn, user_id, &secret_base32)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let totp = build_totp(&secret_base32, &user.email, &self.settings.mfa_issuer)?;
        let otpauth_uri = SecretString::from(totp.get_url());

        Ok(MfaEnrollmentInit {
            secret_base32,
            otpauth_uri,
        })
    }

    async fn confirm_mfa_enrollment(
        &self,
        user_id: UserId,
        code: &str,
    ) -> StoreResult<MfaActivated> {
        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_id(&mut conn, user_id.as_uuid())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if user.totp_enabled {
            bail!(StoreError::InvalidArgument(
                "MFA is already enabled for this user".into()
            ));
        }

        let secret_b32 = user.totp_secret.clone().ok_or(StoreError::InvalidArgument(
            "No pending MFA enrollment for this user. Call init_mfa_enrollment first.".into(),
        ))?;

        let totp = build_totp(&secret_b32, &user.email, &self.settings.mfa_issuer)?;
        if !verify_totp_code(&totp, code) {
            // InvalidArgument, not LoginError: the user is already logged in here, so Unauthenticated would log them out.
            bail!(StoreError::InvalidArgument(
                "invalid authenticator code".into()
            ));
        }

        let (recovery_plain, recovery_hashes) = generate_recovery_codes()?;

        UserRow::activate_totp(&mut conn, user_id, &recovery_hashes)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
            self,
            user_id,
            &user.email,
            crate::domain::org_security_audit_log::SecurityEventType::UserMfaEnrolled,
            None,
            Default::default(),
        )
        .await;

        Ok(MfaActivated {
            recovery_codes: recovery_plain,
        })
    }

    async fn disable_mfa(&self, user_id: UserId, password: SecretString) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_id(&mut conn, user_id.as_uuid())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        verify_user_password(&user, &password)?;

        UserRow::disable_totp(&mut conn, user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
            self,
            user_id,
            &user.email,
            crate::domain::org_security_audit_log::SecurityEventType::UserMfaDisabled,
            None,
            Default::default(),
        )
        .await;
        Ok(())
    }

    async fn regenerate_mfa_recovery_codes(
        &self,
        user_id: UserId,
        password: SecretString,
    ) -> StoreResult<MfaActivated> {
        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_id(&mut conn, user_id.as_uuid())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if !user.totp_enabled {
            bail!(StoreError::InvalidArgument(
                "MFA is not enabled for this user".into()
            ));
        }

        verify_user_password(&user, &password)?;

        let (recovery_plain, recovery_hashes) = generate_recovery_codes()?;

        UserRow::replace_recovery_codes(&mut conn, user_id, &recovery_hashes)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
            self,
            user_id,
            &user.email,
            crate::domain::org_security_audit_log::SecurityEventType::UserMfaRecoveryCodesRegenerated,
            None,
            Default::default(),
        )
        .await;

        Ok(MfaActivated {
            recovery_codes: recovery_plain,
        })
    }

    async fn verify_mfa_login(
        &self,
        challenge_token: SecretString,
        code: &str,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<LoginUserResponse> {
        let mut validation = Validation::default();
        validation.set_audience(&[Audience::MfaChallenge.as_str()]);

        let token_data = crate::jwt::decode::<JwtClaims>(
            challenge_token.expose_secret(),
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map_err(|_| StoreError::LoginError("invalid or expired MFA challenge".into()))?;

        let user_uuid = Uuid::parse_str(token_data.claims.sub.as_str())
            .map_err(|_| StoreError::LoginError("invalid MFA challenge".into()))?;

        let mut conn = self.get_conn().await?;
        let user = UserRow::find_by_id(&mut conn, user_uuid)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if user.archived_at.is_some() {
            bail!(StoreError::LoginError("account is deleted".into()));
        }
        if !user.totp_enabled {
            bail!(StoreError::LoginError("MFA not enrolled".into()));
        }

        let secret_b32 = user
            .totp_secret
            .clone()
            .ok_or(StoreError::LoginError("MFA misconfigured".into()))?;
        let totp = build_totp(&secret_b32, &user.email, &self.settings.mfa_issuer)?;

        let user_email = user.email.clone();
        let user_id_typed = user.id;

        if verify_totp_code(&totp, code) {
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthMfaChallengeSucceeded,
                None,
                request_context.clone(),
            )
            .await;
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthLoginSucceeded,
                None,
                request_context,
            )
            .await;
            return Ok(LoginUserResponse::Authenticated {
                token: generate_auth_jwt_token(&user.id, &self.settings.jwt_secret)?,
                user: user.into(),
            });
        }

        // Otherwise try to consume a recovery code.
        let stored_recovery_codes: Vec<String> =
            user.totp_recovery_codes.iter().flatten().cloned().collect();
        if let Some(remaining) = try_consume_recovery_code(&stored_recovery_codes, code) {
            UserRow::consume_recovery_code(&mut conn, user.id, &remaining)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthMfaRecoveryCodeConsumed,
                Some(serde_json::json!({"remaining": remaining.len()})),
                request_context.clone(),
            )
            .await;
            crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
                self,
                user_id_typed,
                &user_email,
                crate::domain::org_security_audit_log::SecurityEventType::AuthLoginSucceeded,
                None,
                request_context,
            )
            .await;
            return Ok(LoginUserResponse::Authenticated {
                token: generate_auth_jwt_token(&user.id, &self.settings.jwt_secret)?,
                user: user.into(),
            });
        }

        crate::repositories::org_security_audit_log::record_auth_event_for_user_orgs(
            self,
            user_id_typed,
            &user_email,
            crate::domain::org_security_audit_log::SecurityEventType::AuthMfaChallengeFailed,
            None,
            request_context,
        )
        .await;
        bail!(StoreError::LoginError("invalid authenticator code".into()));
    }
}

fn generate_jwt_token(
    sub: &str,
    secret: &SecretString,
    expires_at: DateTime<Utc>,
    audience: Audience,
    payload: Option<JwtPayload>,
) -> StoreResult<SecretString> {
    let claims = serde_json::to_value(JwtClaims {
        sub: sub.to_owned(),
        exp: expires_at.timestamp() as usize,
        aud: audience,
        payload,
    })
    .map_err(|err| StoreError::SerdeError("failed to generate JWT token".into(), err))?;

    let token = crate::jwt::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.expose_secret().as_bytes()),
    )
    .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()))?;

    Ok(SecretString::from(token))
}

fn generate_auth_jwt_token(user_id: &UserId, secret: &SecretString) -> StoreResult<SecretString> {
    generate_jwt_token(
        user_id.as_uuid().to_string().as_str(),
        secret,
        Utc::now() + chrono::Duration::weeks(1),
        Audience::WebApi,
        None,
    )
}

fn hash_password(password: &str) -> StoreResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| StoreError::InvalidArgument("unable to hash password".to_string()))?;
    Ok(hash.to_string())
}

fn verify_user_password(user: &UserRow, password: &SecretString) -> StoreResult<()> {
    let password_hash = user.password_hash.clone().ok_or(StoreError::LoginError(
        "Password is not set. Cannot verify.".into(),
    ))?;
    let db_hash_parsed = PasswordHash::new(&password_hash)
        .map_err(|_| StoreError::InvalidArgument("password hash".to_string()))?;
    Argon2::default()
        .verify_password(password.expose_secret().as_bytes(), &db_hash_parsed)
        .map_err(|_| StoreError::LoginError("invalid password".to_string()))?;
    Ok(())
}

fn build_totp(secret_base32: &str, email: &str, issuer: &str) -> StoreResult<totp_rs::TOTP> {
    let raw = totp_rs::Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|_| StoreError::InvalidArgument("invalid TOTP secret".into()))?;
    totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        raw,
        Some(issuer.to_string()),
        email.to_string(),
    )
    .map_err(|_| StoreError::InvalidArgument("invalid TOTP configuration".into()).into())
}

fn verify_totp_code(totp: &totp_rs::TOTP, code: &str) -> bool {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    totp.check_current(&code).unwrap_or(false)
}

/// Generates 10 random base32-style recovery codes (e.g. "4HKQ-9XPM-7RFD")
/// and returns (plaintext, argon2-hashes).
fn generate_recovery_codes() -> StoreResult<(Vec<String>, Vec<String>)> {
    use rand::RngExt;
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut rng = rand::rng();

    let mut plain = Vec::with_capacity(10);
    let mut hashes = Vec::with_capacity(10);
    for _ in 0..10 {
        let chars: String = (0..12)
            .map(|i| {
                let c = ALPHABET[rng.random_range(0..ALPHABET.len())] as char;
                if i == 4 || i == 8 {
                    format!("-{c}")
                } else {
                    c.to_string()
                }
            })
            .collect();
        hashes.push(hash_password(&chars)?);
        plain.push(chars);
    }
    Ok((plain, hashes))
}

/// If `code` (trimmed) matches one of the stored argon2 hashes, returns the
/// remaining hashes (with the matched one removed). Otherwise None.
fn try_consume_recovery_code(stored_hashes: &[String], code: &str) -> Option<Vec<String>> {
    let normalized = code.trim().to_uppercase();
    let argon2 = Argon2::default();
    for (idx, hash) in stored_hashes.iter().enumerate() {
        if let Ok(parsed) = PasswordHash::new(hash)
            && argon2
                .verify_password(normalized.as_bytes(), &parsed)
                .is_ok()
        {
            let mut remaining = stored_hashes.to_vec();
            remaining.remove(idx);
            return Some(remaining);
        }
    }
    None
}

/// After password (or oauth) validation, decide whether to issue a session
/// token directly or to require a second-factor challenge. Enrollment
/// enforcement (org policy requiring MFA but user not yet enrolled) is NOT
/// performed here — that's gated at the UI level using `Me.mfa_enrollment_required`,
/// since the user is allowed a transient session to walk through enrollment.
pub(crate) async fn finalize_login(
    store: &Store,
    _conn: &mut PgConn,
    user: UserRow,
) -> StoreResult<LoginUserResponse> {
    if user.archived_at.is_some() {
        bail!(StoreError::LoginError("account is deleted".into()));
    }
    if user.totp_enabled {
        return Ok(LoginUserResponse::MfaChallenge {
            challenge_token: generate_mfa_challenge_token(&user.id, &store.settings.jwt_secret)?,
        });
    }

    Ok(LoginUserResponse::Authenticated {
        token: generate_auth_jwt_token(&user.id, &store.settings.jwt_secret)?,
        user: user.into(),
    })
}

fn generate_mfa_challenge_token(
    user_id: &UserId,
    secret: &SecretString,
) -> StoreResult<SecretString> {
    generate_jwt_token(
        user_id.as_uuid().to_string().as_str(),
        secret,
        Utc::now() + chrono::Duration::minutes(5),
        Audience::MfaChallenge,
        None,
    )
}

pub(crate) async fn register_user_internal(
    store: &Store,
    req: RegisterUserRequestInternal,
    request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
) -> StoreResult<RegisterUserResponse> {
    let mut conn = store.get_conn().await?;

    let user_opt = UserRow::find_by_email(&mut conn, req.email.clone()).await?;

    if user_opt.is_some() {
        return Err(StoreError::DuplicateValue {
            entity: "user",
            key: None,
        }
        .into());
    }

    async fn create_user(
        conn: &mut PgConn,
        req: &RegisterUserRequestInternal,
    ) -> StoreResult<UserId> {
        // Hash password
        let hashed_password = req
            .password
            .as_ref()
            .map(|x| hash_password(x.expose_secret()))
            .transpose()?;

        let user_new = UserRowNew {
            id: Uuid::now_v7().into(),
            email: req.email.clone(),
            password_hash: hashed_password,
        };

        user_new
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(user_new.id)
    }

    let (user_id, joined_organization_id) = match req.invite_key {
        None => {
            if !store.multi_organization_enabled() {
                let users_non_empty = UserRow::any_exists(&mut conn).await?;

                if users_non_empty {
                    return Err(Report::new(StoreError::UserRegistrationClosed("registration is currently closed. Please request an invite key from your administrator.".into())));
                }
            }

            // we don't initiate an organization yet. User will be prompted to onboard.
            (create_user(&mut conn, &req).await?, None)
        }
        Some(ref invite_key) => {
            let invite_id =
                <OrganizationInviteId as std::str::FromStr>::from_str(invite_key.expose_secret())
                    .map_err(|_| {
                    Report::new(StoreError::InvalidArgument(
                        "Invalid invite key".to_string(),
                    ))
                })?;

            let cloned_req = req.clone();
            store
                .transaction(|conn| {
                    async move {
                        let invite = OrganizationInviteRow::find_by_id(conn, invite_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        if invite.accepted_at.is_some()
                            || invite.revoked_at.is_some()
                            || invite.expires_at < Utc::now()
                        {
                            return Err(StoreError::InvalidArgument(
                                "Invalid or expired invite link".to_string(),
                            )
                            .into());
                        }
                        if invite.invited_email.to_lowercase() != cloned_req.email.to_lowercase() {
                            return Err(StoreError::Forbidden(
                                "This invite is not valid for your account".to_string(),
                            )
                            .into());
                        }

                        let created = create_user(conn, &cloned_req).await?;

                        let om = OrganizationMemberRow {
                            user_id: created,
                            organization_id: invite.organization_id,
                            role: invite.role,
                            role_id: invite.role_id,
                        };
                        om.insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        OrganizationInviteRow::set_accepted_at(conn, invite_id, Utc::now())
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        Ok((created, invite.organization_id))
                    }
                    .scope_boxed()
                })
                .await
                .map(|(created, org_id)| (created, Some(org_id)))?
        }
    };

    let _ = store
        .eventbus
        .publish(Event::user_created(
            None,
            user_id,
            req.signup_source.clone(),
        ))
        .await;

    // A user registering through an invite link joins the organization in the
    // same step (bypassing the AcceptInvite RPC), so the org's audit trail must
    // record the membership here too.
    if let Some(organization_id) = joined_organization_id {
        store
            .record_security_event_best_effort(
                crate::domain::org_security_audit_log::RecordSecurityEvent {
                    organization_id,
                    event_type:
                        crate::domain::org_security_audit_log::SecurityEventType::MemberInviteAccepted,
                    actor: crate::domain::org_security_audit_log::Actor::User { id: user_id },
                    actor_email: Some(req.email.clone()),
                    target_user_id: Some(user_id.as_uuid()),
                    target_email: Some(req.email.clone()),
                    metadata: None,
                    context: request_context,
                },
            )
            .await;
    }

    let user: User = UserRow::find_by_id(&mut conn, *user_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(Into::into)?;

    Ok(RegisterUserResponse {
        token: generate_auth_jwt_token(&user_id, &store.settings.jwt_secret)?,
        user,
    })
}

/// Refuse a non-SSO sign-in when the email's domain belongs to an organization
/// that enforces SSO *and the user is a member of that organization*.
///
/// Member-scoped on purpose: an org's SSO requirement governs its own members,
/// not every account that merely shares the email domain. So a user who only
/// belongs to *other* orgs (or is a brand-new signup) keeps password access —
/// one org can't lock outsiders out of their own orgs by claiming a domain.
/// Fails open on lookup errors: an unreachable SSO / membership table must not
/// hard-block logins.
async fn ensure_not_sso_enforced(store: &Store, email: &str) -> StoreResult<()> {
    let connection = match store.find_sso_connection_for_email(email).await {
        Ok(Some(conn)) if conn.enforced => conn,
        Ok(_) => return Ok(()),
        Err(err) => {
            tracing::warn!(
                ?err,
                "SSO enforcement lookup failed; allowing password login"
            );
            return Ok(());
        }
    };

    if user_is_org_member(store, email, connection.organization_id).await {
        bail!(StoreError::LoginError(
            "Your organization requires Single Sign-On. Please sign in via SSO.".into()
        ));
    }
    Ok(())
}

/// Whether the account for `email` is a member of `organization_id`. Best-effort:
/// any lookup failure (or unknown user) resolves to `false` so enforcement fails
/// open rather than hard-blocking on a transient error.
async fn user_is_org_member(store: &Store, email: &str, organization_id: OrganizationId) -> bool {
    let Ok(mut conn) = store.get_conn().await else {
        return false;
    };
    let Ok(Some(user)) = UserRow::find_by_email(&mut conn, email.to_string()).await else {
        return false;
    };
    OrganizationMemberRow::find_by_user_and_org(&mut conn, user.id.as_uuid(), organization_id)
        .await
        .is_ok()
}

pub(crate) fn validate_domain(email: &str, allowed_domains: &[String]) -> StoreResult<bool> {
    if allowed_domains.is_empty() {
        return Ok(true);
    }
    let domain = email
        .split('@')
        .nth(1)
        .ok_or_else(|| StoreError::InvalidArgument("email does not contain domain".into()))?;

    if allowed_domains.iter().any(|d| d == domain) {
        Ok(true)
    } else {
        Err(Report::new(StoreError::LoginError(
            "New signups are currently restricted. Please contact us to request access."
                .to_string(),
        )))
    }
}

use crate::domain::Organization;
use crate::domain::enums::OrganizationUserRole;
use crate::domain::oauth::{OauthConnection, OauthVerifierData};
use crate::domain::users::{
    InitRegistrationResponse, LoginUserRequest, LoginUserResponse, Me, RegisterUserRequest,
    RegisterUserRequestInternal, RegisterUserResponse, UpdateUser, User, UserWithRole,
};
use crate::errors::StoreError;
use crate::repositories::oauth::OauthInterface;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use common_domain::auth::{Audience, JwtClaims, JwtPayload};
use common_domain::ids::{OrganizationId, TenantId};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::OrganizationRow;
use diesel_models::users::{UserRow, UserRowNew, UserRowPatch};
use error_stack::{Report, ResultExt, bail};
use jsonwebtoken::{DecodingKey, Validation};
use meteroid_mailer::model::{EmailRecipient, EmailValidationLink, ResetPasswordLink};
use meteroid_oauth::model::OauthProvider;
use secrecy::{ExposeSecret, SecretString};
use tracing::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait UserInterface {
    async fn init_registration(
        &self,
        email: String,
        invite_key: Option<SecretString>,
    ) -> StoreResult<InitRegistrationResponse>;
    async fn complete_registration(
        &self,
        req: RegisterUserRequest,
    ) -> StoreResult<RegisterUserResponse>;

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse>;
    async fn me(
        &self,
        auth_user_id: Uuid,
        organization_id: Option<OrganizationId>,
    ) -> StoreResult<Me>;
    async fn update_user_details(&self, auth_user_id: Uuid, data: UpdateUser) -> StoreResult<User>;
    // async fn update_user_role(&self, auth_user_id: Uuid, organization_id: Uuid, data: UpdateUserRole) -> StoreResult<User>;

    async fn find_user_by_id_and_organization(
        &self,
        id: Uuid,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole>;
    async fn find_user_by_id_and_tenant(
        &self,
        id: Uuid,
        tenant_id: TenantId,
    ) -> StoreResult<UserWithRole>;

    async fn find_user_by_email_and_organization(
        &self,
        email: String,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole>;
    async fn list_users_for_organization(
        &self,
        org_id: OrganizationId,
    ) -> StoreResult<Vec<UserWithRole>>;

    /** Internal use only. For API/external, use `me()` or `find_user_by_id_and_organization()` */
    async fn _find_user_by_id(&self, id: Uuid) -> StoreResult<User>;

    async fn init_reset_password(&self, email: String) -> StoreResult<()>;

    async fn reset_password(
        &self,
        token: SecretString,
        new_password: SecretString,
    ) -> StoreResult<()>;

    async fn oauth_signin(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
    ) -> StoreResult<LoginUserResponse>;
}

#[async_trait::async_trait]
impl UserInterface for Store {
    async fn complete_registration(
        &self,
        req: RegisterUserRequest,
    ) -> StoreResult<RegisterUserResponse> {
        if self.settings.skip_email_validation {
            validate_domain(&req.email, &self.settings.domains_whitelist)?;

            return register_user_internal(
                self,
                RegisterUserRequestInternal {
                    password: req.password,
                    email: req.email,
                    invite_key: req.invite_key,
                },
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

        let token_data = jsonwebtoken::decode::<JwtClaims>(
            &token,
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let email = token_data.claims.sub.as_str();

        let invite_key = match token_data.claims.payload {
            Some(JwtPayload::EmailValidation { invite_key }) => invite_key,
            _ => None,
        };

        register_user_internal(
            self,
            RegisterUserRequestInternal {
                password: req.password,
                email: email.to_string(),
                invite_key: invite_key.map(SecretString::new),
            },
        )
        .await
    }

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse> {
        let mut conn = self.get_conn().await?;

        validate_domain(&req.email, &self.settings.domains_whitelist)?;

        let user =
            UserRow::find_by_email(&mut conn, req.email)
                .await?
                .ok_or(StoreError::LoginError(
                    "incorrect email and/or password".into(),
                ))?;

        let password_hash = user.password_hash.clone().ok_or(StoreError::LoginError(
            "Password is not set. Login is forbidden".into(),
        ))?;

        let argon2 = Argon2::default();

        let db_hash_parsed = PasswordHash::new(&password_hash)
            .map_err(|_| StoreError::InvalidArgument("password hash".to_string()))?;

        argon2
            .verify_password(req.password.expose_secret().as_bytes(), &db_hash_parsed)
            .map_err(|_| StoreError::LoginError("invalid email or password".to_string()))?;

        Ok(LoginUserResponse {
            token: generate_auth_jwt_token(&user.id.to_string(), &self.settings.jwt_secret)?,
            user: user.into(),
        })
    }

    async fn me(
        &self,
        auth_user_id: Uuid,
        organization_id: Option<OrganizationId>,
    ) -> StoreResult<Me> {
        let mut conn = self.get_conn().await?;

        let organizations: Vec<Organization> =
            OrganizationRow::list_by_user_id(&mut conn, auth_user_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .map(|x| x.into_iter().map(Into::into).collect())?;

        let (user, current_organization_role) = if let Some(org_id) = organization_id {
            let user: UserWithRole =
                UserRow::find_by_id_and_org_id(&mut conn, auth_user_id, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .map(Into::into)?;

            let role = user.role.clone();
            (user.into(), Some(role))
        } else {
            let user: User = UserRow::find_by_id(&mut conn, auth_user_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .map(Into::into)?;

            (user, None)
        };

        Ok(Me {
            user,
            organizations,
            current_organization_role,
        })
    }

    async fn update_user_details(&self, auth_user_id: Uuid, data: UpdateUser) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        let patch = UserRowPatch {
            id: auth_user_id,
            first_name: data.first_name,
            last_name: data.last_name,
            department: data.department.clone(),
            onboarded: Some(true),
        };

        //TODO send know_us_from & department to analytics

        let res = patch
            .update_user(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let _ = self
            .eventbus
            .publish(Event::user_updated(
                auth_user_id,
                auth_user_id,
                data.department,
                data.know_us_from,
            ))
            .await;

        Ok(res)
    }

    async fn find_user_by_id_and_organization(
        &self,
        id: Uuid,
        org_id: OrganizationId,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id_and_org_id(&mut conn, id, org_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_user_by_id_and_tenant(
        &self,
        id: Uuid,
        tenant_id: TenantId,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id_and_tenant_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
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

    async fn _find_user_by_id(&self, id: Uuid) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_id(&mut conn, id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn init_reset_password(&self, email: String) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let user = UserRow::find_by_email(&mut conn, email.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if let Some(user) = user {
            // todo add expire_in to config
            let url_expires_in = chrono::Duration::minutes(10);

            let token = generate_jwt_token(
                &user.id.to_string(),
                &self.settings.jwt_secret,
                Utc::now() + url_expires_in,
                Audience::ResetPassword,
                None,
            )?;

            let url = SecretString::new(format!(
                "{}/reset-password?token={}",
                self.settings.public_url.as_str(),
                token.expose_secret()
            ));

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

            log::info!("Reset password email sent for user: {}", user.id);
        } else {
            log::warn!("User with email {email} not found");
        }

        Ok(())
    }

    async fn init_registration(
        &self,
        email: String,
        invite_key: Option<SecretString>,
    ) -> StoreResult<InitRegistrationResponse> {
        let mut conn = self.get_conn().await?;

        validate_domain(&email, &self.settings.domains_whitelist)?;

        let user_opt = UserRow::find_by_email(&mut conn, email.clone()).await?;

        if user_opt.is_some() {
            return Err(StoreError::DuplicateValue {
                entity: "user",
                key: None,
            }
            .into());
        }

        if self.settings.skip_email_validation {
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
            }),
        )?;

        let url = SecretString::new(format!(
            "{}/validate-email?token={}",
            self.settings.public_url.as_str(),
            token.expose_secret()
        ));

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
    ) -> StoreResult<()> {
        let mut validation = Validation::default();
        validation.set_audience(&[Audience::ResetPassword.as_str()]);

        let token_data = jsonwebtoken::decode::<JwtClaims>(
            token.expose_secret(),
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let user_id = Uuid::parse_str(token_data.claims.sub.as_str())
            .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let new_password_hash = hash_password(new_password.expose_secret())?;

        let mut conn = self.get_conn().await?;

        UserRow::update_password_hash(&mut conn, user_id, new_password_hash.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn oauth_signin(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
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
                    invite_key: signin_data.invite_key.map(SecretString::new),
                };

                let res = register_user_internal(self, user_new).await?;

                Ok(LoginUserResponse {
                    token: res.token,
                    user: res.user,
                })
            }
            Some(user) => {
                if signin_data.is_signup {
                    bail!(StoreError::OauthError("User already exists".into()))
                }

                Ok(LoginUserResponse {
                    token: generate_auth_jwt_token(
                        &user.id.to_string(),
                        &self.settings.jwt_secret,
                    )?,
                    user: user.into(),
                })
            }
        }
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

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.expose_secret().as_bytes()),
    )
    .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()))?;

    Ok(SecretString::new(token))
}

fn generate_auth_jwt_token(user_id: &str, secret: &SecretString) -> StoreResult<SecretString> {
    generate_jwt_token(
        user_id,
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

async fn register_user_internal(
    store: &Store,
    req: RegisterUserRequestInternal,
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
    ) -> StoreResult<Uuid> {
        // Hash password
        let hashed_password = req
            .password
            .as_ref()
            .map(|x| hash_password(x.expose_secret()))
            .transpose()?;

        let user_new = UserRowNew {
            id: Uuid::now_v7(),
            email: req.email.clone(),
            password_hash: hashed_password,
        };

        user_new
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(user_new.id)
    }

    let user_id = match req.invite_key {
        None => {
            if !store.settings.multi_organization_enabled {
                let users_non_empty = UserRow::any_exists(&mut conn).await?;

                if users_non_empty {
                    return Err(Report::new(StoreError::UserRegistrationClosed("registration is currently closed. Please request an invite key from your administrator.".into())));
                }
            }

            // we don't initiate an organization yet. User will be prompted to onboard.
            create_user(&mut conn, &req).await?
        }
        Some(ref invite_link) => {
            let cloned_req = req.clone();
            store
                .transaction(|conn| {
                    async move {
                        let org_id = OrganizationRow::find_by_invite_link(
                            conn,
                            invite_link.expose_secret().clone(),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .id;

                        let created = create_user(conn, &cloned_req).await?;

                        let om = OrganizationMemberRow {
                            user_id: created,
                            organization_id: org_id,
                            role: OrganizationUserRole::Member.into(),
                        };
                        om.insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        Ok(created)
                    }
                    .scope_boxed()
                })
                .await?
        }
    };

    let _ = store
        .eventbus
        .publish(Event::user_created(None, user_id))
        .await;

    let user: User = UserRow::find_by_id(&mut conn, user_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(Into::into)?;

    Ok(RegisterUserResponse {
        token: generate_auth_jwt_token(&user_id.to_string(), &store.settings.jwt_secret)?,
        user,
    })
}

fn validate_domain(email: &str, allowed_domains: &[String]) -> StoreResult<bool> {
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
            "Domain not authorized".to_string(),
        )))
    }
}

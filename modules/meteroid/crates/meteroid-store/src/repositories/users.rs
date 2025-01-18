use crate::domain::enums::OrganizationUserRole;
use crate::domain::users::{
    LoginUserRequest, LoginUserResponse, Me, RegisterUserRequest, RegisterUserResponse, UpdateUser,
    User, UserWithRole,
};
use crate::domain::Organization;
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::OrganizationRow;
use diesel_models::users::{UserRow, UserRowNew, UserRowPatch};
use error_stack::{Report, ResultExt};
use jsonwebtoken::{DecodingKey, Validation};
use meteroid_mailer::model::{EmailRecipient, ResetPasswordLink};
use secrecy::{ExposeSecret, SecretString};
use tracing::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait UserInterface {
    async fn register_user(&self, req: RegisterUserRequest) -> StoreResult<RegisterUserResponse>;
    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse>;
    async fn me(&self, auth_user_id: Uuid, organization_id: Option<Uuid>) -> StoreResult<Me>;
    async fn update_user_details(&self, auth_user_id: Uuid, data: UpdateUser) -> StoreResult<User>;
    // async fn update_user_role(&self, auth_user_id: Uuid, organization_id: Uuid, data: UpdateUserRole) -> StoreResult<User>;

    async fn find_user_by_id_and_organization(
        &self,
        id: Uuid,
        org_id: Uuid,
    ) -> StoreResult<UserWithRole>;
    async fn find_user_by_id_and_tenant(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<UserWithRole>;

    async fn find_user_by_email_and_organization(
        &self,
        email: String,
        org_id: Uuid,
    ) -> StoreResult<UserWithRole>;
    async fn list_users_for_organization(&self, org_id: Uuid) -> StoreResult<Vec<UserWithRole>>;

    /** Internal use only. For API/external, use me() or find_user_by_id_and_organization() */
    async fn _find_user_by_id(&self, id: Uuid) -> StoreResult<User>;

    async fn init_reset_password(&self, email: String) -> StoreResult<()>;

    async fn reset_password(&self, token: String, new_password: String) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl UserInterface for Store {
    async fn register_user(&self, req: RegisterUserRequest) -> StoreResult<RegisterUserResponse> {
        let mut conn = self.get_conn().await?;

        let user_opt = UserRow::find_by_email(&mut conn, req.email.clone()).await?;

        if user_opt.is_some() {
            return Err(StoreError::DuplicateValue {
                entity: "user",
                key: None,
            }
            .into());
        }

        async fn create_user(conn: &mut PgConn, req: &RegisterUserRequest) -> StoreResult<Uuid> {
            // Hash password
            let hashed_password = hash_password(req.password.expose_secret())?;

            let user_new = UserRowNew {
                id: Uuid::now_v7(),
                email: req.email.clone(),
                password_hash: Some(hashed_password),
            };

            user_new
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            Ok(user_new.id)
        }

        let user_id = match req.invite_key {
            None => {
                if !self.settings.multi_organization_enabled {
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
                self.transaction(|conn| {
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

        let _ = self
            .eventbus
            .publish(Event::user_created(None, user_id))
            .await;

        let user: User = UserRow::find_by_id(&mut conn, user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        Ok(RegisterUserResponse {
            token: generate_auth_jwt_token(&user_id.to_string(), &self.settings.jwt_secret)?,
            user,
        })
    }

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse> {
        let mut conn = self.get_conn().await?;

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

    async fn me(&self, auth_user_id: Uuid, organization_id: Option<Uuid>) -> StoreResult<Me> {
        let mut conn = self.get_conn().await?;

        let organizations: Vec<Organization> =
            OrganizationRow::list_by_user_id(&mut conn, auth_user_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .map(|x| x.into_iter().map(Into::into).collect())?;

        let (user, current_organization_role) = match organization_id {
            Some(org_id) => {
                let user: UserWithRole =
                    UserRow::find_by_id_and_org_id(&mut conn, auth_user_id, org_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                let role = user.role.clone();
                (user.into(), Some(role))
            }
            None => {
                let user: User = UserRow::find_by_id(&mut conn, auth_user_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .map(Into::into)?;

                (user, None)
            }
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
        org_id: Uuid,
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
        tenant_id: Uuid,
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
        org_id: Uuid,
    ) -> StoreResult<UserWithRole> {
        let mut conn = self.get_conn().await?;

        UserRow::find_by_email_and_org_id(&mut conn, email, org_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_users_for_organization(&self, org_id: Uuid) -> StoreResult<Vec<UserWithRole>> {
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
            log::warn!("User with email {} not found", email);
        }

        Ok(())
    }

    async fn reset_password(&self, token: String, new_password: String) -> StoreResult<()> {
        let token_data = jsonwebtoken::decode::<JwtClaims>(
            token.as_str(),
            &DecodingKey::from_secret(self.settings.jwt_secret.expose_secret().as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let user_id = Uuid::parse_str(token_data.claims.sub.as_str())
            .map_err(|_| StoreError::InvalidArgument("invalid token".into()))?;

        let new_password_hash = hash_password(new_password.as_str())?;

        let mut conn = self.get_conn().await?;

        UserRow::update_password_hash(&mut conn, user_id, new_password_hash.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

fn generate_jwt_token(
    user_id: &str,
    secret: &SecretString,
    expires_at: DateTime<Utc>,
) -> StoreResult<SecretString> {
    let claims = serde_json::to_value(JwtClaims {
        sub: user_id.to_owned(),
        exp: expires_at.timestamp() as usize,
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
    generate_jwt_token(user_id, secret, Utc::now() + chrono::Duration::weeks(1))
}

fn hash_password(password: &str) -> StoreResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| StoreError::InvalidArgument("unable to hash password".to_string()))?;
    Ok(hash.to_string())
}

// todo reuse in common-grpc as well
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct JwtClaims {
    sub: String,
    exp: usize,
}

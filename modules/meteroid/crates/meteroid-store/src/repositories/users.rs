use crate::domain::enums::{OrganizationUserRole, TenantEnvironmentEnum};
use crate::domain::users::{
    LoginUserRequest, LoginUserResponse, RegisterUserRequest, RegisterUserResponse, User,
};
use crate::domain::OrgTenantNew;
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait UserInterface {
    async fn register_user(&self, req: RegisterUserRequest) -> StoreResult<RegisterUserResponse>;
    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse>;
    async fn me(&self, auth_user_id: Uuid) -> StoreResult<User>;
    async fn find_user_by_id(&self, id: Uuid, auth_user_id: Uuid) -> StoreResult<User>;
    async fn find_user_by_email(&self, email: String, auth_user_id: Uuid) -> StoreResult<User>;
    async fn list_users(&self, auth_user_id: Uuid) -> StoreResult<Vec<User>>;
}

#[async_trait::async_trait]
impl UserInterface for Store {
    async fn register_user(&self, req: RegisterUserRequest) -> StoreResult<RegisterUserResponse> {
        let mut conn = self.get_conn().await?;

        let user_opt =
            diesel_models::users::User::find_by_email(&mut conn, req.email.clone()).await?;

        if user_opt.is_some() {
            return Err(StoreError::DuplicateValue {
                entity: "user",
                key: None,
            }
            .into());
        }

        async fn create_user(
            conn: &mut PgConn,
            req: &RegisterUserRequest,
            organization_id: Uuid,
            role: OrganizationUserRole,
        ) -> StoreResult<Uuid> {
            // Hash password
            let hashed_password = hash_password(&req.password.expose_secret())?;

            let user_new = diesel_models::users::UserNew {
                id: Uuid::now_v7(),
                email: req.email.clone(),
                password_hash: Some(hashed_password),
            };

            user_new
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let om = diesel_models::organization_members::OrganizationMember {
                user_id: user_new.id,
                organization_id,
                role: role.into(),
            };

            om.insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            Ok(user_new.id)
        }

        let user_id = match req.invite_key {
            None => {
                let users_non_empty = diesel_models::users::User::any_exists(&mut conn).await?;

                if users_non_empty {
                    return Err(Report::new(StoreError::UserRegistrationClosed("registration is currently closed. Please request an invite key from your administrator.".into())));
                }

                // This is the first user. We allow invite-less registration & init the instance
                self.transaction(|conn| {
                    async move {
                        let org = diesel_models::organizations::OrganizationNew {
                            id: Uuid::now_v7(),
                            name: "ACME Inc.".into(),
                            slug: "instance".into(),
                        };

                        org.insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        let user_id =
                            create_user(conn, &req, org.id, OrganizationUserRole::Admin).await?;

                        let tenant: diesel_models::tenants::TenantNew = OrgTenantNew {
                            name: "Sandbox".into(),
                            slug: "sandbox".into(),
                            organization_id: org.id,
                            currency: "EUR".into(),
                            environment: Some(TenantEnvironmentEnum::Sandbox),
                        }
                        .into();

                        tenant
                            .insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        Ok(user_id)
                    }
                    .scope_boxed()
                })
                .await?
            }
            Some(ref invite_link) => {
                let org_id = diesel_models::organizations::Organization::find_by_invite_link(
                    &mut conn,
                    invite_link.expose_secret().clone(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .id;

                self.transaction(|conn| {
                    async move {
                      create_user(conn, &req, org_id, OrganizationUserRole::Member).await
                    }.scope_boxed()
                })
                .await?
            }
        };

        let _ = self
            .eventbus
            .publish(Event::user_created(None, user_id))
            .await;

        let user: User = diesel_models::users::User::find_by_id(&mut conn, user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        Ok(RegisterUserResponse {
            token: generate_jwt_token(&user_id.to_string(), &self.jwt_secret)?,
            user: user.into(),
        })
    }

    async fn login_user(&self, req: LoginUserRequest) -> StoreResult<LoginUserResponse> {
        let mut conn = self.get_conn().await?;

        let user = diesel_models::users::User::find_by_email(&mut conn, req.email)
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
            token: generate_jwt_token(&user.id.to_string(), &self.jwt_secret)?,
            user: user.into(),
        })
    }

    async fn me(&self, auth_user_id: Uuid) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        diesel_models::users::User::find_by_id(&mut conn, auth_user_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_user_by_id(&self, id: Uuid, auth_user_id: Uuid) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        let org =
            diesel_models::organizations::Organization::find_by_user_id(&mut conn, auth_user_id)
                .await?;

        diesel_models::users::User::find_by_id_and_org_id(&mut conn, id, org.id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_user_by_email(&self, email: String, auth_user_id: Uuid) -> StoreResult<User> {
        let mut conn = self.get_conn().await?;

        let org =
            diesel_models::organizations::Organization::find_by_user_id(&mut conn, auth_user_id)
                .await?;

        diesel_models::users::User::find_by_email_and_org_id(&mut conn, email, org.id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_users(&self, auth_user_id: Uuid) -> StoreResult<Vec<User>> {
        let mut conn = self.get_conn().await?;

        let org =
            diesel_models::organizations::Organization::find_by_user_id(&mut conn, auth_user_id)
                .await?;

        diesel_models::users::User::list_by_org_id(&mut conn, org.id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }
}

fn generate_jwt_token(user_id: &str, secret: &SecretString) -> StoreResult<SecretString> {
    // todo create Claims struct and reuse in common-grpc as well
    let claims = json!({
      "sub": user_id.to_owned(),
      "exp": chrono::Utc::now().timestamp() as usize + 60 * 60 * 24 * 7, // 1 week validity
    });

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.expose_secret().as_bytes()),
    )
    .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()))?;

    Ok(SecretString::new(token))
}

fn hash_password(password: &str) -> StoreResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| StoreError::InvalidArgument("unable to hash password".to_string()))?;
    Ok(hash.to_string())
}

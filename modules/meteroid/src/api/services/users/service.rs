use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use deadpool_postgres::Transaction;
use jsonwebtoken::{EncodingKey, Header};
use meteroid_repository as db;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::{api::services::utils::parse_uuid, parse_uuid};

use super::{mapping, UsersDbService};
use crate::api::services::utils::uuid_gen;
use common_grpc::middleware::common::jwt::Claims;
use common_grpc::middleware::server::auth::RequestExt;
use common_grpc::middleware::server::idempotency::idempotency_cache;
use meteroid_grpc::meteroid::api::users::v1::{
    users_service_server::UsersService, FindUserByEmailRequest, FindUserByEmailResponse,
    GetUserByIdRequest, GetUserByIdResponse, ListUsersRequest, ListUsersResponse, LoginRequest,
    LoginResponse, MeRequest, MeResponse, RegisterRequest, RegisterResponse, UpsertUserRequest,
    UpsertUserResponse, User,
};
use meteroid_repository::{OrganizationUserRole, Params};

#[tonic::async_trait]
impl UsersService for UsersDbService {
    #[tracing::instrument(skip(self))]
    async fn upsert_user(
        &self,
        request: Request<UpsertUserRequest>,
    ) -> Result<Response<UpsertUserResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::users::UpsertUserParams {
            id: parse_uuid!(&req.id)?,
            email: req.email,
            password_hash: Some(req.password_hash),
        };

        let res = db::users::upsert_user()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                tonic::Status::internal("Unable to create user.")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = UpsertUserResponse {
            id: res.id.to_string(),
            email: res.email,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn me(&self, request: Request<MeRequest>) -> Result<Response<MeResponse>, Status> {
        let actor = request.actor()?;
        let connection = self.get_connection().await?;
        let res = db::users::get_user_by_id()
            .bind(&connection, &actor)
            .one()
            .await
            .map_err(|e| {
                Status::not_found("Unable to find user.")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = MeResponse {
            user: Some(User {
                id: res.id.to_string(),
                email: res.email,
                role: mapping::role::db_to_server(res.role).into(),
            }),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn get_user_by_id(
        &self,
        request: Request<GetUserByIdRequest>,
    ) -> Result<Response<GetUserByIdResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::users::get_user_by_id()
            .bind(&connection, &parse_uuid!(&req.id)?)
            .one()
            .await
            .map_err(|e| {
                Status::not_found("Unable to find user.")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = GetUserByIdResponse {
            user: Some(User {
                id: res.id.to_string(),
                email: res.email,
                role: mapping::role::db_to_server(res.role).into(),
            }),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn find_user_by_email(
        &self,
        request: Request<FindUserByEmailRequest>,
    ) -> Result<Response<FindUserByEmailResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::users::get_user_by_email()
            .bind(&connection, &req.email)
            .one()
            .await
            .map_err(|e| {
                tonic::Status::not_found("Unable to find user.")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = FindUserByEmailResponse {
            user: Some(User {
                id: res.id.to_string(),
                email: res.email,
                role: mapping::role::db_to_server(res.role).into(),
            }),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip(self))]
    async fn list_users(
        &self,
        _request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let connection = self.get_connection().await?;

        let res = db::users::list_users()
            .bind(&connection)
            .all()
            .await
            .map_err(|e| {
                tonic::Status::internal("Unable to list users.")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let response = ListUsersResponse {
            users: res
                .into_iter()
                .map(|user| User {
                    id: user.id.to_string(),
                    email: user.email,
                    role: mapping::role::db_to_server(user.role).into(),
                })
                .collect(),
        };

        Ok(Response::new(response))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();
            let connection = self.get_connection().await?;

            // Fetch user by email
            let user = db::users::get_user_hash_by_email()
                .bind(&connection, &req.email)
                .one()
                .await
                .map_err(|_| Status::unauthenticated("Invalid email or password."))?;

            // Validate password if any
            let argon2 = Argon2::default();

            if user.password_hash.is_none() {
                return Err(Status::unauthenticated(
                    "User does not have a password hash. Login is forbidden.",
                ));
            }

            let hash_binding = user.password_hash.unwrap();
            let db_hash_parsed = PasswordHash::new(&hash_binding)
                .map_err(|_| Status::internal("Invalid password hash."))?;

            argon2
                .verify_password(req.password.as_bytes(), &db_hash_parsed)
                .map_err(|_| Status::unauthenticated("Invalid email or password."))?;

            // Generate JWT token
            let token = generate_jwt_token(&user.id.to_string(), &self.jwt_secret)?;

            let response = LoginResponse {
                token,
                user: Some(User {
                    id: user.id.to_string(),
                    email: user.email,
                    role: mapping::role::db_to_server(user.role).into(),
                }),
            };

            Ok(Response::new(response))
        })
        .await
    }

    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        idempotency_cache(request, |request| async {
            let req = request.into_inner();
            let mut connection = self.get_connection().await?;

            // check if user already exists
            let exists = db::users::get_user_by_email()
                .bind(&connection, &req.email)
                .opt()
                .await
                .map_err(|_| Status::internal("Failed to check user existence"))?;

            if exists.is_some() {
                return Err(Status::already_exists("User already exists."));
            }

            async fn create_user(
                req: &RegisterRequest,
                jwt_secret: &SecretString,
                transaction: &Transaction<'_>,
                user_role: OrganizationUserRole,
                organization_id: Uuid,
            ) -> Result<Response<RegisterResponse>, Status> {
                // Hash password
                let hashed_password = hash_password(&req.password)?;

                let params = db::users::UpsertUserParams {
                    id: uuid_gen::v7(),
                    email: &req.email,
                    password_hash: Some(hashed_password),
                };

                // Insert new user into database
                let new_user = db::users::upsert_user()
                    .params(transaction, &params)
                    .one()
                    .await
                    .map_err(|e| {
                        Status::internal("Failed to create user.")
                            .set_source(Arc::new(e))
                            .clone()
                    })?;

                let role_params = db::organizations::CreateOrganizationMemberParams {
                    role: user_role,
                    user_id: new_user.id,
                    organization_id,
                };

                let _ = db::organizations::create_organization_member()
                    .params(transaction, &role_params)
                    .one()
                    .await
                    .map_err(|e| {
                        Status::internal("Failed to set user role.")
                            .set_source(Arc::new(e))
                            .clone()
                    })?;

                // Generate JWT token
                let token = generate_jwt_token(&new_user.id.to_string(), &jwt_secret)?;

                Ok(Response::new(RegisterResponse {
                    token,
                    user: Some(User {
                        id: new_user.id.to_string(),
                        email: new_user.email,
                        role: mapping::role::db_to_server(user_role).into(),
                    }),
                }))
            }

            match req.invite_key {
                Some(ref invite_key) => {
                    let instance = db::organizations::get_organization_by_invite_hash()
                        .bind(&connection, &invite_key)
                        .one()
                        .await
                        .map_err(|e| {
                            Status::internal("Failed to validate invite.")
                                .set_source(Arc::new(e))
                                .clone()
                        })?;

                    let transaction = self.get_transaction(&mut connection).await?;

                    let res = create_user(
                        &req,
                        &self.jwt_secret,
                        &transaction,
                        OrganizationUserRole::MEMBER,
                        instance.id,
                    )
                    .await?;

                    transaction.commit().await.map_err(|e| {
                        Status::internal("Failed to commit transaction")
                            .set_source(Arc::new(e))
                            .clone()
                    })?;

                    Ok(res)
                }
                None => {
                    // Check if there are any existing users
                    let has_users = db::users::exist_users()
                        .bind(&connection)
                        .one()
                        .await
                        .map_err(|_| Status::internal("Failed to check instance users"))?;
                    if has_users {
                        Err(Status::permission_denied("Registration is currently closed. Please request an invite key from your administrator."))
                    } else {
                        // This is the first user. We allow invite-less registration & init the instance
                        let transaction = self.get_transaction(&mut connection).await?;

                        let org = db::organizations::create_organization()
                            .params(
                                &transaction,
                                &db::organizations::CreateOrganizationParams {
                                    id: uuid_gen::v7(),
                                    name: "ACME Inc.",
                                    slug: "instance",
                                },
                            )
                            .one()
                            .await
                            .map_err(|e| {
                                Status::internal("Unable to create instance")
                                    .set_source(Arc::new(e))
                                    .clone()
                            })?;

                        let res = create_user(
                            &req,
                            &self.jwt_secret,
                            &transaction,
                            OrganizationUserRole::ADMIN,
                            org.id,
                        )
                        .await?;

                        transaction.commit().await.map_err(|e| {
                            Status::internal("Failed to commit transaction")
                                .set_source(Arc::new(e))
                                .clone()
                        })?;

                        Ok(res)
                    }
                }
            }
        }).await
    }
}

fn generate_jwt_token(user_id: &str, secret: &SecretString) -> Result<String, Status> {
    let claims = Claims {
        sub: user_id.to_owned(),
        exp: chrono::Utc::now().timestamp() as usize + 60 * 60 * 24 * 7, // 1 week validity
    };

    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.expose_secret().as_bytes()),
    )
    .map_err(|e| {
        Status::internal("Failed to generate JWT token.")
            .set_source(Arc::new(e))
            .clone()
    })
}

fn hash_password(password: &str) -> Result<String, Status> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| Status::internal("Unable to hash password"))?;
    Ok(hash.to_string())
}

use crate::domain::enums::OrganizationUserRole;
use o2o::o2o;
use secrecy::SecretString;
use uuid::Uuid;

use diesel_models::users::UserRow;

#[derive(Clone, Debug, o2o)]
#[from_owned(UserRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[map(~.into())]
    pub role: OrganizationUserRole,
}

#[derive(Clone, Debug)]
pub struct LoginUserRequest {
    pub email: String,
    pub password: SecretString,
}

#[derive(Clone, Debug)]
pub struct LoginUserResponse {
    pub token: SecretString,
    pub user: User,
}

#[derive(Clone, Debug)]
pub struct RegisterUserRequest {
    pub email: String,
    pub password: SecretString,
    pub invite_key: Option<SecretString>,
}

#[derive(Clone, Debug)]
pub struct RegisterUserResponse {
    pub token: SecretString,
    pub user: User,
}

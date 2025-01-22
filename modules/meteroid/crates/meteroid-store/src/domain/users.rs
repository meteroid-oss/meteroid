use crate::domain::enums::OrganizationUserRole;
use o2o::o2o;
use secrecy::SecretString;
use uuid::Uuid;

use crate::domain::Organization;
use diesel_models::users::{UserRow, UserWithRoleRow};

#[derive(Clone, Debug, o2o)]
#[from_owned(UserRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(UserWithRoleRow)]
pub struct UserWithRole {
    pub id: Uuid,
    pub email: String,
    #[map(~.into())]
    pub role: OrganizationUserRole,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
}

impl From<UserWithRole> for User {
    fn from(val: UserWithRole) -> Self {
        User {
            id: val.id,
            email: val.email,
            onboarded: val.onboarded,
            first_name: val.first_name,
            last_name: val.last_name,
            department: val.department,
        }
    }
}

pub struct Me {
    pub user: User,
    pub organizations: Vec<Organization>,
    pub current_organization_role: Option<OrganizationUserRole>,
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

#[derive(Clone, Debug, o2o)]
#[from_owned(RegisterUserRequest)]
pub(crate) struct RegisterUserRequestInternal {
    pub email: String,
    #[map(Some(~))]
    pub password: Option<SecretString>,
    pub invite_key: Option<SecretString>,
}

#[derive(Clone, Debug)]
pub struct RegisterUserResponse {
    pub token: SecretString,
    pub user: User,
}

#[derive(Debug)]
pub struct UpdateUser {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub know_us_from: Option<String>,
}

#[derive(Debug)]
pub struct UpdateUserRole {
    pub role: OrganizationUserRole,
    pub user_id: Uuid,
}

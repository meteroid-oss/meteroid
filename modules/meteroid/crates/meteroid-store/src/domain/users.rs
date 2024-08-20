use crate::domain::enums::OrganizationUserRole;
use o2o::o2o;
use secrecy::SecretString;
use uuid::Uuid;

use diesel_models::users::{UserRow, UserWithRoleRow, UserRowOnboardingPatch};


#[derive(Clone, Debug, o2o)]
#[from_owned(UserRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub onboarded: bool,
    pub first_name: String,
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
    pub first_name: String,
    pub last_name: Option<String>,
    pub department: Option<String>,
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

#[derive(Debug, o2o)]
#[owned_into(UserRowOnboardingPatch)]
#[ghosts(onboarded: {true})]
pub struct OnboardingAccountNew {
    pub id: Uuid,
    pub first_name: String,
    pub last_name: Option<String>,
    pub department: Option<String>,
    #[ghost({None})]
    pub know_us_from: Option<String>,
}



use chrono::NaiveDateTime;

use crate::enums::OrganizationUserRole;
use common_domain::ids::{RoleId, UserId};
use diesel::{AsChangeset, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRow {
    pub id: UserId,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub totp_recovery_codes: Vec<Option<String>>,
    pub totp_enrolled_at: Option<NaiveDateTime>,
    pub preferences: serde_json::Value,
}

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserWithRoleRow {
    pub id: UserId,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
    #[diesel(select_expression = crate::schema::organization_member::role)]
    #[diesel(select_expression_type = crate::schema::organization_member::role)]
    pub role: OrganizationUserRole,
    #[diesel(select_expression = crate::schema::organization_member::role_id)]
    #[diesel(select_expression_type = crate::schema::organization_member::role_id)]
    pub role_id: Option<RoleId>,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub totp_enabled: bool,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRowNew {
    pub id: UserId,
    pub email: String,
    pub password_hash: Option<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRowPatch {
    pub id: UserId,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub onboarded: Option<bool>,
}

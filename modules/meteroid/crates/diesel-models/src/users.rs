use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::OrganizationUserRole;
use diesel::{AsChangeset, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
}

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserWithRoleRow {
    pub id: Uuid,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
    #[diesel(select_expression = crate::schema::organization_member::role)]
    #[diesel(select_expression_type = crate::schema::organization_member::role)]
    pub role: OrganizationUserRole,
    pub onboarded: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
}


#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRowNew {
    pub id: Uuid,
    pub email: String,
    pub password_hash: Option<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserRowPatch {
    pub id: Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub department: Option<String>,
    pub onboarded: Option<bool>,
}
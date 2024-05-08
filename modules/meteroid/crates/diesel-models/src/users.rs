use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::OrganizationUserRole;
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
    #[diesel(select_expression = crate::schema::organization_member::role)]
    #[diesel(select_expression_type = crate::schema::organization_member::role)]
    pub role: OrganizationUserRole,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserNew {
    pub id: Uuid,
    pub email: String,
    pub password_hash: Option<String>,
}

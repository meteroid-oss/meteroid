use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable};

#[derive(Debug, Queryable, Identifiable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiTokenNew {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}

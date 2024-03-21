use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::user)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub password_hash: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::user)]
pub struct UserNew {
    pub email: String,
    pub password_hash: Option<String>,
}

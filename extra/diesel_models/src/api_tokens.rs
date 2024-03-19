
use chrono::NaiveDateTime;
use uuid::Uuid;


use diesel::{Identifiable, Queryable};



#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::api_token)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}

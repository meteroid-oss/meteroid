use chrono::NaiveDateTime;
use common_domain::ids::{QuoteCommentId, QuoteId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_comment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteCommentRow {
    pub id: QuoteCommentId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub author_user_id: Option<Uuid>,
    pub author_email: Option<String>,
    pub author_name: String,
    pub body: String,
    pub shared: bool,
    pub is_change_request: bool,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub anchor: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_comment)]
pub struct QuoteCommentRowNew {
    pub id: QuoteCommentId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub author_user_id: Option<Uuid>,
    pub author_email: Option<String>,
    pub author_name: String,
    pub body: String,
    pub shared: bool,
    pub is_change_request: bool,
    pub anchor: Option<String>,
}

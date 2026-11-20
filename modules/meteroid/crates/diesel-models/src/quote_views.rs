use chrono::NaiveDateTime;
use common_domain::ids::{QuoteId, QuoteViewId, TenantId};
use diesel::{Identifiable, Queryable, QueryableByName, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, QueryableByName, Selectable)]
#[diesel(table_name = crate::schema::quote_view)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteViewRow {
    pub id: QuoteViewId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub email: String,
    pub revision: i32,
    pub viewed_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

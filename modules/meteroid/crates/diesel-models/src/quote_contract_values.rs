use chrono::NaiveDateTime;
use common_domain::ids::{QuoteId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::quote_contract_value)]
#[diesel(primary_key(quote_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteContractValueRow {
    pub quote_id: QuoteId,
    pub tenant_id: TenantId,
    pub value: serde_json::Value,
    pub computed_at: NaiveDateTime,
}

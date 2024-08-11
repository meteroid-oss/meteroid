use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer_balance_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalanceTxRow {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub amount_cents: i32,
    pub balance_cents_after: i32,
    pub note: Option<String>,
    pub invoice_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_balance_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalanceTxRowNew {
    pub id: Uuid,
    pub amount_cents: i32,
    pub balance_cents_after: i32,
    pub note: Option<String>,
    pub invoice_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub created_by: Option<Uuid>,
}

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer_balance_pending_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalancePendingTxRow {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub amount_cents: i32,
    pub note: Option<String>,
    pub invoice_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub tx_id: Option<Uuid>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_balance_pending_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalancePendingTxRowNew {
    pub id: Uuid,
    pub amount_cents: i32,
    pub note: Option<String>,
    pub invoice_id: Option<Uuid>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub tx_id: Option<Uuid>,
    pub created_by: Uuid,
}

use chrono::NaiveDateTime;
use uuid::Uuid;

use common_domain::ids::{CustomerId, InvoiceId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer_balance_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalanceTxRow {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub amount_cents: i64,
    pub balance_cents_after: i64,
    pub note: Option<String>,
    pub invoice_id: Option<InvoiceId>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_balance_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalanceTxRowNew {
    pub id: Uuid,
    pub amount_cents: i64,
    pub balance_cents_after: i64,
    pub note: Option<String>,
    pub invoice_id: Option<InvoiceId>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub created_by: Option<Uuid>,
}

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer_balance_pending_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalancePendingTxRow {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub amount_cents: i64,
    pub note: Option<String>,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub tx_id: Option<Uuid>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer_balance_pending_tx)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBalancePendingTxRowNew {
    pub id: Uuid,
    pub amount_cents: i64,
    pub note: Option<String>,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub tx_id: Option<Uuid>,
    pub created_by: Uuid,
}

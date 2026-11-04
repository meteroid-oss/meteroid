use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;

use super::enums::MrrMovementType;
use common_domain::ids::{CustomerId, InvoiceId, PlanVersionId, SubscriptionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable};

// The MRR movement log: the transactional record of every MRR change, written
// with billing events and projected into ClickHouse `fact_mrr_movement` by the
// analytics projector. The former Postgres rollups over it (bi_delta_mrr_daily,
// bi_revenue_daily, bi_customer_ytd_summary) are gone — the analytics plane
// serves every aggregate read.

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::bi_mrr_movement_log)]
pub struct BiMrrMovementLogRow {
    pub id: Uuid,
    pub description: String,
    pub movement_type: MrrMovementType,
    pub net_mrr_change: i64,
    pub currency: String,
    pub created_at: NaiveDateTime,
    pub applies_to: NaiveDate,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<Uuid>,
    pub plan_version_id: PlanVersionId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub customer_id: CustomerId,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::bi_mrr_movement_log)]
pub struct BiMrrMovementLogRowNew {
    pub id: Uuid,
    pub description: String,
    pub movement_type: MrrMovementType,
    pub net_mrr_change: i64,
    pub currency: String,
    pub applies_to: NaiveDate,
    pub invoice_id: Option<InvoiceId>,
    pub credit_note_id: Option<Uuid>,
    pub plan_version_id: PlanVersionId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub customer_id: CustomerId,
}

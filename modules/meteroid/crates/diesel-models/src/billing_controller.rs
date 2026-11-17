use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel::{Queryable, QueryableByName, Selectable};

/// A customer's billing controller: when it runs next, whether an input changed, its lease.
#[derive(Debug, Clone, Queryable, Selectable, QueryableByName)]
#[diesel(table_name = crate::schema::billing_controller)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BillingControllerRow {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub next_wake_at: Option<DateTime<Utc>>,
    pub dirty_since: Option<DateTime<Utc>>,
    pub lease_until: Option<DateTime<Utc>>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub first_failed_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

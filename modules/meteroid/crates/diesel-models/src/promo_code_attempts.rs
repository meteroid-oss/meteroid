use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel::Insertable;
use uuid::Uuid;

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::promo_code_attempt)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PromoCodeAttemptRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub attempted_at: DateTime<Utc>,
}

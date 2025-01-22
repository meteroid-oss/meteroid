use crate::enums::PaymentMethodTypeEnum;
use chrono::NaiveDateTime;
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::customer_payment_method)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerPaymentMethodRow {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub payment_method_type: PaymentMethodTypeEnum,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::customer_payment_method)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerPaymentMethodRowNew {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub payment_method_type: PaymentMethodTypeEnum,
}

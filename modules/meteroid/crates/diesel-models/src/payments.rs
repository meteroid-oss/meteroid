use crate::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::payment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentRow {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub invoice_id: Uuid,
    pub provider_payment_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i32,
    pub currency: i32,
    // TODO fees ?
    pub payment_method_id: Option<Uuid>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::payment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentRowNew {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub invoice_id: Uuid,
    pub provider_payment_id: Option<String>,
    pub amount: i32,
    pub currency: i32,
    pub payment_method_id: Option<Uuid>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::payment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id))]
pub struct PaymentRowPatch {
    pub id: Uuid,
    pub status: Option<PaymentStatusEnum>,
    pub processed_at: Option<Option<NaiveDateTime>>,
    pub refunded_at: Option<Option<NaiveDateTime>>,
}

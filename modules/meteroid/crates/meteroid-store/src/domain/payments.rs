use super::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;

use diesel_models::payments::{PaymentRow, PaymentRowNew};
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, o2o)]
#[from_owned(PaymentRow)]
pub struct Payment {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub invoice_id: Uuid,
    pub provider_payment_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i32,
    pub currency: i32, // TODO ???
    // TODO fees ?
    pub payment_method_id: Option<Uuid>,
    #[map(~.into())]
    pub status: PaymentStatusEnum,
    #[map(~.into())]
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(PaymentRowNew)]
pub struct PaymentNew {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub invoice_id: Uuid,
    pub provider_payment_id: Option<String>,
    pub amount: i32,
    pub currency: i32,
    pub payment_method_id: Option<Uuid>,
    #[map(~.into())]
    pub status: PaymentStatusEnum,
    #[map(~.into())]
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
}

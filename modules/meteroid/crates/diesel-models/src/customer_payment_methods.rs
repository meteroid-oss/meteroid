use crate::enums::PaymentMethodTypeEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{CustomerConnectionId, CustomerId, CustomerPaymentMethodId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable, AsChangeset)]
#[diesel(table_name = crate::schema::customer_payment_method)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerPaymentMethodRow {
    pub id: CustomerPaymentMethodId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub connection_id: CustomerConnectionId,
    pub external_payment_method_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::customer_payment_method)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
pub struct CustomerPaymentMethodRowNew {
    pub id: CustomerPaymentMethodId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub connection_id: CustomerConnectionId,
    pub external_payment_method_id: String,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

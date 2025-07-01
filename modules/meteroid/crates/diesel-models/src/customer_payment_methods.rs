use crate::enums::PaymentMethodTypeEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{BankAccountId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, InvoicingEntityId, PlanId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use crate::subscriptions::SubscriptionRow;


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



#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionForDisplayRow {
    #[diesel(embed)]
    pub subscription: SubscriptionRow,
    #[diesel(select_expression = crate::schema::customer::id)]
    #[diesel(select_expression_type = crate::schema::customer::id)]
    pub customer_id: CustomerId,
    #[diesel(select_expression = crate::schema::customer::alias)]
    #[diesel(select_expression_type = crate::schema::customer::alias)]
    pub customer_alias: Option<String>,
    #[diesel(select_expression = crate::schema::customer::name)]
    #[diesel(select_expression_type = crate::schema::customer::name)]
    pub customer_name: String,
    #[diesel(select_expression = crate::schema::customer::invoicing_entity_id)]
    #[diesel(select_expression_type = crate::schema::customer::invoicing_entity_id)]
    pub invoicing_entity_id: InvoicingEntityId,
    #[diesel(select_expression = crate::schema::plan_version::version)]
    #[diesel(select_expression_type = crate::schema::plan_version::version)]
    pub version: i32,
    #[diesel(select_expression = crate::schema::plan::name)]
    #[diesel(select_expression_type = crate::schema::plan::name)]
    pub plan_name: String,
    #[diesel(select_expression = crate::schema::plan::id)]
    #[diesel(select_expression_type = crate::schema::plan::id)]
    pub plan_id: PlanId,
}


#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ResolvedSubscriptionPaymentMethod {
    #[diesel(select_expression = crate::schema::subscription::payment_method_type)]
    #[diesel(select_expression_type = crate::schema::subscription::payment_method_type)]
    pub subscription_payment_method: Option<PaymentMethodTypeEnum>,
    #[diesel(select_expression = crate::schema::subscription::bank_account_id)]
    #[diesel(select_expression_type = crate::schema::subscription::bank_account_id)]
    pub  subscription_bank_account_id: Option<BankAccountId>,
    #[diesel(select_expression = crate::schema::customer::bank_account_id)]
    #[diesel(select_expression_type = crate::schema::customer::bank_account_id)]
    pub  customer_bank_account_id: Option<BankAccountId>,
    #[diesel(select_expression = crate::schema::invoicing_entity::bank_account_id)]
    #[diesel(select_expression_type = crate::schema::invoicing_entity::bank_account_id)]
    pub  invoicing_entity_bank_account_id: Option<BankAccountId>,
    #[diesel(select_expression = crate::schema::subscription::payment_method)]
    #[diesel(select_expression_type = crate::schema::subscription::payment_method)]
    pub   subscription_payment_method_id: Option<CustomerPaymentMethodId>,
    #[diesel(select_expression = crate::schema::customer::current_payment_method_id)]
    #[diesel(select_expression_type = crate::schema::customer::current_payment_method_id)]
    pub customer_payment_method_id: Option<CustomerPaymentMethodId>,
}

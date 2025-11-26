use super::enums::PaymentMethodTypeEnum;
use chrono::NaiveDateTime;

use crate::domain::ConnectorProviderEnum;
use common_domain::ids::{
    BankAccountId, ConnectorId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, TenantId,
};
use diesel_models::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
};
use o2o::o2o;
use secrecy::SecretString;

#[derive(Clone, Debug, PartialEq, o2o)]
#[from_owned(CustomerPaymentMethodRow)]
pub struct CustomerPaymentMethod {
    pub id: CustomerPaymentMethodId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub connection_id: CustomerConnectionId,
    pub external_payment_method_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    #[map(~.into())]
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(CustomerPaymentMethodRowNew)]
pub struct CustomerPaymentMethodNew {
    pub id: CustomerPaymentMethodId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub connection_id: CustomerConnectionId,
    pub external_payment_method_id: String,
    #[map(~.into())]
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

pub struct SetupIntent {
    pub intent_id: String,
    pub client_secret: String,
    pub public_key: SecretString,
    pub provider: ConnectorProviderEnum,
    pub connector_id: ConnectorId,
    pub connection_id: CustomerConnectionId,
}

// pub struct ResolvedPaymentMethod {
//     subscription_payment_method: Option<PaymentMethodTypeEnum>,
//     subscription_bank_account_id: Option<BankAccountId>,
//     customer_bank_account_id: Option<BankAccountId>,
//     invoicing_entity_bank_account_id: Option<BankAccountId>,
//     subscription_payment_method_id: Option<CustomerPaymentMethodId>,
//     customer_payment_method_id: Option<CustomerPaymentMethodId>,
// }

pub enum ResolvedPaymentMethod {
    CustomerPaymentMethod(CustomerPaymentMethodId),
    BankTransfer(BankAccountId),
    NotConfigured, // TODO we could separate NotConfigured and RequiresCheckout (for the first there's no connector), or it's done elsewhere
}

pub struct CustomerPaymentMethodFromProvider {
    pub external_payment_method_id: String,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
}

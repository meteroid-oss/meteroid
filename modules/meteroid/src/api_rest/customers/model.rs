use crate::api_rest::addresses::model::{Address, ShippingAddress};
use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use common_domain::ids::{BankAccountId, CustomerId};
use common_domain::ids::{InvoicingEntityId, string_serde, string_serde_opt};
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct CustomerFilters {
    pub search: Option<String>,
    pub archived: Option<bool>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct CustomerListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(flatten)]
    #[validate(nested)]
    pub customer_filters: CustomerFilters,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Customer {
    #[serde(with = "string_serde")]
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub currency: Currency,
    #[serde(with = "string_serde")]
    pub invoicing_entity_id: InvoicingEntityId,
    #[serde(default, with = "string_serde_opt")]
    pub bank_account_id: Option<BankAccountId>,
    pub vat_number: Option<String>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Debug)]
pub struct CustomerCreateRequest {
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: Currency,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    #[serde(default, with = "string_serde_opt")]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    #[serde(default, with = "string_serde_opt")]
    pub bank_account_id: Option<BankAccountId>,
    pub vat_number: Option<String>,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub is_tax_exempt: Option<bool>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct CustomerUpdateRequest {
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: Currency,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    #[serde(with = "string_serde")]
    pub invoicing_entity_id: InvoicingEntityId,
    #[serde(default, with = "string_serde_opt")]
    pub bank_account_id: Option<BankAccountId>,
    pub vat_number: Option<String>,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub is_tax_exempt: Option<bool>,
}

// TODO : allow importing from stripe
// => Allow providing a stripe customer id and load the customer methods from stripe

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerListResponse {
    pub data: Vec<Customer>,
    pub pagination_meta: PaginationResponse,
}

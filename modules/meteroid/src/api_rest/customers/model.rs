use crate::api_rest::addresses::model::{Address, ShippingAddress};
use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::PaginatedRequest;
use meteroid_store::domain;
use utoipa::ToSchema;

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerFilters {
    pub search: Option<String>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerListRequest {
    #[serde(flatten)]
    pub pagination: PaginatedRequest,
    #[serde(flatten)]
    pub plan_filters: CustomerFilters,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Customer {
    pub id: String,
    pub name: String,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub currency: Currency,
    pub invoicing_entity_id: String,
    pub billing_config: BillingConfig, // todo revisit how we present billing config in the API
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
#[serde(tag = "object", rename_all = "snake_case")]
pub enum BillingConfig {
    Stripe(StripeBillingConfig),
    Manual,
}

#[derive(Debug, Clone, ToSchema, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StripeBillingConfig {
    pub customer_id: String,
    pub collection_method: StripeCollectionMethod,
}

#[derive(Debug, Clone, ToSchema, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StripeCollectionMethod {
    ChargeAutomatically,
    SendInvoice,
}

impl From<domain::BillingConfig> for BillingConfig {
    fn from(value: domain::BillingConfig) -> Self {
        match value {
            domain::BillingConfig::Stripe(stripe) => BillingConfig::Stripe(StripeBillingConfig {
                customer_id: stripe.customer_id,
                collection_method: match stripe.collection_method {
                    domain::StripeCollectionMethod::ChargeAutomatically => {
                        StripeCollectionMethod::ChargeAutomatically
                    }
                    domain::StripeCollectionMethod::SendInvoice => {
                        StripeCollectionMethod::SendInvoice
                    }
                },
            }),
            domain::BillingConfig::Manual => BillingConfig::Manual,
        }
    }
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerCreateRequest {
    pub name: String,
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: Currency,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<String>,
}

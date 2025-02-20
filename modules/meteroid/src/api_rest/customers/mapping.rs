use crate::api_rest::addresses;
use crate::api_rest::currencies;
use crate::api_rest::customers::model::{
    BillingConfig, Customer, CustomerCreateRequest, CustomerUpdateRequest, StripeCollectionMethod,
};
use crate::api_rest::model::IdOrAlias;
use crate::errors::RestApiError;
use meteroid_store::domain;
use meteroid_store::domain::{CustomerNew, Identity};
use uuid::Uuid;

pub fn domain_to_rest(d: domain::CustomerForDisplay) -> Result<Customer, RestApiError> {
    Ok(Customer {
        id: d.id.into(),
        name: d.name,
        alias: d.alias,
        email: d.email,
        invoicing_email: d.invoicing_email,
        phone: d.phone,
        billing_address: d
            .billing_address
            .map(addresses::mapping::address::domain_to_rest),
        shipping_address: d
            .shipping_address
            .map(addresses::mapping::shipping_address::domain_to_rest),
        currency: currencies::mapping::from_str(d.currency.as_str())?,
        billing_config: d.billing_config.into(),
        invoicing_entity_id: d.invoicing_entity_local_id,
    })
}

fn billing_config_to_domain(billing_config: BillingConfig) -> domain::BillingConfig {
    match billing_config {
        BillingConfig::Manual => domain::BillingConfig::Manual,
        BillingConfig::Stripe(stripe) => {
            domain::BillingConfig::Stripe(domain::StripeCustomerConfig {
                customer_id: stripe.customer_id,
                collection_method: match stripe.collection_method {
                    StripeCollectionMethod::ChargeAutomatically => {
                        domain::StripeCollectionMethod::ChargeAutomatically
                    }
                    StripeCollectionMethod::SendInvoice => {
                        domain::StripeCollectionMethod::SendInvoice
                    }
                },
            })
        }
    }
}

pub fn create_req_to_domain(created_by: Uuid, req: CustomerCreateRequest) -> CustomerNew {
    CustomerNew {
        name: req.name,
        created_by,
        invoicing_entity_id: req.invoicing_entity_id.map(Identity::LOCAL),
        billing_config: billing_config_to_domain(req.billing_config),
        alias: req.alias,
        email: req.email,
        invoicing_email: req.invoicing_email,
        phone: req.phone,
        balance_value_cents: 0,
        currency: req.currency.to_string(),
        billing_address: req
            .billing_address
            .map(addresses::mapping::address::rest_to_domain),
        shipping_address: req
            .shipping_address
            .map(addresses::mapping::shipping_address::rest_to_domain),
        force_created_date: None,
    }
}

pub fn update_req_to_domain(
    id_or_alias: IdOrAlias,
    req: CustomerUpdateRequest,
) -> domain::CustomerUpdate {
    domain::CustomerUpdate {
        local_id_or_alias: id_or_alias.0,
        name: req.name,
        invoicing_entity_id: Identity::LOCAL(req.invoicing_entity_id),
        billing_config: billing_config_to_domain(req.billing_config),
        alias: req.alias,
        email: req.email,
        invoicing_email: req.invoicing_email,
        phone: req.phone,
        currency: req.currency.to_string(),
        billing_address: req
            .billing_address
            .map(addresses::mapping::address::rest_to_domain),
        shipping_address: req
            .shipping_address
            .map(addresses::mapping::shipping_address::rest_to_domain),
    }
}

use crate::api_rest::addresses;
use crate::api_rest::currencies;
use crate::api_rest::customers::model::{Customer, CustomerCreateRequest, CustomerUpdateRequest};
use crate::errors::RestApiError;
use common_domain::ids::{AliasOr, CustomerId};
use meteroid_store::domain;
use meteroid_store::domain::CustomerNew;
use uuid::Uuid;

pub fn domain_to_rest(d: domain::Customer) -> Result<Customer, RestApiError> {
    Ok(Customer {
        id: d.id,
        name: d.name,
        alias: d.alias,
        billing_email: d.billing_email,
        invoicing_emails: d.invoicing_emails,
        phone: d.phone,
        billing_address: d
            .billing_address
            .map(addresses::mapping::address::domain_to_rest),
        shipping_address: d
            .shipping_address
            .map(addresses::mapping::shipping_address::domain_to_rest),
        currency: currencies::mapping::from_str(d.currency.as_str())?,
        invoicing_entity_id: d.invoicing_entity_id,
    })
}

pub fn create_req_to_domain(created_by: Uuid, req: CustomerCreateRequest) -> CustomerNew {
    CustomerNew {
        name: req.name,
        created_by,
        invoicing_entity_id: req.invoicing_entity_id,
        alias: req.alias,
        billing_email: req.billing_email,
        invoicing_emails: req.invoicing_emails,
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
        // TODO
        bank_account_id: None,
        vat_number: None,
        custom_vat_rate: None,
    }
}

pub fn update_req_to_domain(
    id_or_alias: AliasOr<CustomerId>,
    req: CustomerUpdateRequest,
) -> domain::CustomerUpdate {
    domain::CustomerUpdate {
        id_or_alias,
        name: req.name,
        invoicing_entity_id: req.invoicing_entity_id,
        alias: req.alias,
        billing_email: req.billing_email,
        invoicing_emails: req.invoicing_emails,
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

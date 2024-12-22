use crate::api_rest::addresses;
use crate::api_rest::currencies;
use crate::api_rest::customers::model::Customer;
use crate::errors::RestApiError;
use meteroid_store::domain;

pub fn domain_to_rest(d: domain::CustomerForDisplay) -> Result<Customer, RestApiError> {
    Ok(Customer {
        id: d.local_id,
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

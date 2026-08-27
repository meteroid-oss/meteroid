use crate::api_rest::addresses;
use crate::api_rest::currencies;
use crate::api_rest::customers::model::{
    CustomTaxRate, Customer, CustomerCreateRequest, CustomerPatchRequest, CustomerUpdateRequest,
};
use crate::errors::RestApiError;
use common_domain::ids::{AliasOr, ConnectedAccountId, CustomerId};
use meteroid_store::domain;
use meteroid_store::domain::{CustomerNew, CustomerTaxStatus};
use std::str::FromStr;

/// REST keeps the backward-compatible `is_tax_exempt` bool; true maps to the
/// Exempt tax status, false/absent to Taxable. ReverseCharge is set via gRPC.
fn tax_status_from_rest(is_tax_exempt: Option<bool>) -> CustomerTaxStatus {
    if is_tax_exempt.unwrap_or(false) {
        CustomerTaxStatus::Exempt
    } else {
        CustomerTaxStatus::Taxable
    }
}

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
        vat_number: d.vat_number,
        custom_taxes: d
            .custom_taxes
            .into_iter()
            .map(|t| CustomTaxRate {
                tax_code: t.tax_code,
                name: t.name,
                rate: t.rate,
            })
            .collect(),
        connected_account_id: d.connected_account_id.map(|id| id.to_string()),
    })
}

pub fn create_req_to_domain(req: CustomerCreateRequest) -> Result<CustomerNew, RestApiError> {
    Ok(CustomerNew {
        name: req.name,
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
        vat_number: req.vat_number,
        custom_taxes: req
            .custom_taxes
            .into_iter()
            .map(|t| domain::CustomerTaxRate {
                tax_code: t.tax_code,
                name: t.name,
                rate: t.rate,
            })
            .collect(),
        tax_status: tax_status_from_rest(req.is_tax_exempt),
        exemption_reason: req.exemption_reason,
        connected_account_id: req
            .connected_account_id
            .map(|id| ConnectedAccountId::from_str(&id))
            .transpose()
            .map_err(|_| RestApiError::InvalidInput("Invalid connected_account_id".to_string()))?,
    })
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
        vat_number: req.vat_number,
        custom_taxes: req
            .custom_taxes
            .into_iter()
            .map(|t| domain::CustomerTaxRate {
                tax_code: t.tax_code,
                name: t.name,
                rate: t.rate,
            })
            .collect(),
        tax_status: tax_status_from_rest(req.is_tax_exempt),
        exemption_reason: req.exemption_reason,
    }
}

pub fn patch_req_to_domain(id: CustomerId, req: CustomerPatchRequest) -> domain::CustomerPatch {
    domain::CustomerPatch {
        id,
        name: req.name,
        alias: req.alias,
        billing_email: req.billing_email,
        invoicing_emails: req.invoicing_emails,
        phone: req.phone,
        balance_value_cents: None,
        currency: req.currency.map(|c| c.to_string()),
        billing_address: req
            .billing_address
            .map(addresses::mapping::address::rest_to_domain),
        shipping_address: req
            .shipping_address
            .map(addresses::mapping::shipping_address::rest_to_domain),
        invoicing_entity_id: req.invoicing_entity_id,
        vat_number: req.vat_number.map(Some),
        custom_taxes: req.custom_taxes.map(|taxes| {
            taxes
                .into_iter()
                .map(|t| domain::CustomerTaxRate {
                    tax_code: t.tax_code,
                    name: t.name,
                    rate: t.rate,
                })
                .collect()
        }),
        current_payment_method_id: None,
        tax_status: req.is_tax_exempt.map(|b| {
            if b {
                CustomerTaxStatus::Exempt
            } else {
                CustomerTaxStatus::Taxable
            }
        }),
        exemption_reason: req.exemption_reason.map(Some),
        connected_account_id: None,
    }
}

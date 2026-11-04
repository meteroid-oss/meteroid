use crate::addresses;
use crate::currencies;
use crate::customers::model::{
    CustomTaxRate, Customer, CustomerCreateRequest, CustomerPatchRequest, CustomerType,
    CustomerUpdateRequest,
};
use common_domain::ids::{AliasOr, ConnectedAccountId, CustomerId};
use meteroid_app::errors::RestApiError;
use meteroid_store::domain;
use meteroid_store::domain::enums::CustomerType as DomainCustomerType;
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

fn customer_type_to_domain(t: CustomerType) -> DomainCustomerType {
    match t {
        CustomerType::Company => DomainCustomerType::Company,
        CustomerType::Individual => DomainCustomerType::Individual,
    }
}

fn customer_type_to_rest(t: DomainCustomerType) -> CustomerType {
    match t {
        DomainCustomerType::Company => CustomerType::Company,
        DomainCustomerType::Individual => CustomerType::Individual,
    }
}

pub fn domain_to_rest(d: domain::Customer) -> Result<Customer, RestApiError> {
    Ok(Customer {
        id: d.id,
        name: d.name,
        customer_type: customer_type_to_rest(d.customer_type),
        first_name: d.first_name,
        last_name: d.last_name,
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
        legal_number: d.legal_number,
        buyer_reference: d.buyer_reference,
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
        custom_properties: d.custom_properties,
        invoicing_language: d.preferred_locales.first().cloned(),
        preferred_locales: d.preferred_locales,
    })
}

pub fn create_req_to_domain(req: CustomerCreateRequest) -> Result<CustomerNew, RestApiError> {
    Ok(CustomerNew {
        name: req.name,
        customer_type: customer_type_to_domain(req.customer_type),
        first_name: req.first_name,
        last_name: req.last_name,
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
        force_created_date: None,
        vat_number: req.vat_number,
        legal_number: req.legal_number,
        buyer_reference: req.buyer_reference,
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
        custom_properties: req.custom_properties.unwrap_or(serde_json::Value::Null),
        preferred_locales: validated_locales(preferred_locales(
            req.preferred_locales,
            req.invoicing_language,
        ))?,
    })
}

pub fn update_req_to_domain(
    id_or_alias: AliasOr<CustomerId>,
    req: CustomerUpdateRequest,
    existing: &domain::Customer,
) -> Result<domain::CustomerUpdate, RestApiError> {
    Ok(domain::CustomerUpdate {
        id_or_alias,
        name: req.name,
        // Preserve the stored identity when the update omits it, so a partial replace
        // can't silently reclassify the customer or drop their person name.
        customer_type: req
            .customer_type
            .map(customer_type_to_domain)
            .unwrap_or(existing.customer_type),
        first_name: req.first_name.or_else(|| existing.first_name.clone()),
        last_name: req.last_name.or_else(|| existing.last_name.clone()),
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
        legal_number: req.legal_number,
        buyer_reference: req.buyer_reference,
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
        custom_properties: req.custom_properties.unwrap_or(serde_json::Value::Null),
        preferred_locales: validated_locales(preferred_locales(
            req.preferred_locales,
            req.invoicing_language,
        ))?,
    })
}

/// Reject a preference list we can serve no entry of, rather than storing it and letting
/// the level below quietly decide the language, with only a log line to show for it. One
/// servable entry is enough — falling through to it is what the list is for.
fn validated_locales(locales: Vec<String>) -> Result<Vec<String>, RestApiError> {
    reject_unsupported_locales(&locales)?;
    Ok(locales)
}

fn reject_unsupported_locales(locales: &[String]) -> Result<(), RestApiError> {
    meteroid_invoicing::lang::reject_unsupported_preferences(locales).map_err(|tag| {
        RestApiError::InvalidInput(format!(
            "Unsupported preferred_locales '{tag}'; supported: {:?}",
            meteroid_invoicing::lang::SUPPORTED
        ))
    })
}

/// Back-compat with clients written against the former single `invoicing_language`:
/// their value becomes a one-entry list, and an explicit `preferred_locales` wins.
fn preferred_locales(list: Option<Vec<String>>, legacy: Option<String>) -> Vec<String> {
    list.unwrap_or_else(|| {
        legacy
            .into_iter()
            .filter(|tag| !tag.trim().is_empty())
            .collect()
    })
}

/// Same, for PATCH: `None` leaves the preferences untouched, and the legacy blank
/// string keeps meaning "reset to inherit".
fn preferred_locales_patch(
    list: Option<Vec<String>>,
    legacy: Option<String>,
) -> Option<Vec<String>> {
    match (list, legacy) {
        (Some(list), _) => Some(list),
        (None, Some(tag)) if tag.trim().is_empty() => Some(Vec::new()),
        (None, Some(tag)) => Some(vec![tag]),
        (None, None) => None,
    }
}

pub fn patch_req_to_domain(
    id: CustomerId,
    mut req: CustomerPatchRequest,
) -> Result<domain::CustomerPatch, RestApiError> {
    let preferred_locales =
        preferred_locales_patch(req.preferred_locales.take(), req.invoicing_language.take());
    if let Some(locales) = preferred_locales.as_deref() {
        reject_unsupported_locales(locales)?;
    }
    Ok(domain::CustomerPatch {
        id,
        name: req.name,
        customer_type: req.customer_type.map(customer_type_to_domain),
        first_name: req.first_name,
        last_name: req.last_name,
        alias: req.alias,
        billing_email: req.billing_email,
        invoicing_emails: req.invoicing_emails,
        phone: req.phone,
        currency: req.currency.map(|c| c.to_string()),
        billing_address: req
            .billing_address
            .map(addresses::mapping::address::rest_to_domain),
        shipping_address: req
            .shipping_address
            .map(addresses::mapping::shipping_address::rest_to_domain),
        invoicing_entity_id: req.invoicing_entity_id,
        vat_number: req.vat_number.map(Some),
        legal_number: req.legal_number.map(Some),
        buyer_reference: req.buyer_reference.map(Some),
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
        custom_properties: req.custom_properties,
        preferred_locales,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn the_legacy_field_becomes_a_one_entry_list_unless_the_new_one_is_sent() {
        assert_eq!(
            preferred_locales(None, Some("fr-FR".to_string())),
            list(&["fr-FR"])
        );
        assert_eq!(preferred_locales(None, Some("  ".to_string())), list(&[]));
        assert_eq!(preferred_locales(None, None), list(&[]));
        // An explicit list wins, even an empty one meaning "reset to inherit".
        assert_eq!(
            preferred_locales(Some(list(&["en"])), Some("fr-FR".to_string())),
            list(&["en"])
        );
        assert_eq!(
            preferred_locales(Some(list(&[])), Some("fr-FR".to_string())),
            list(&[])
        );
    }

    #[test]
    fn patch_leaves_the_preferences_alone_when_neither_field_is_sent() {
        assert_eq!(preferred_locales_patch(None, None), None);
        // The legacy blank string kept meaning "reset to inherit"; an empty list does now.
        assert_eq!(
            preferred_locales_patch(None, Some("".to_string())),
            Some(list(&[]))
        );
        assert_eq!(
            preferred_locales_patch(None, Some("fr-FR".to_string())),
            Some(list(&["fr-FR"]))
        );
        assert_eq!(
            preferred_locales_patch(Some(list(&["en"])), Some("fr-FR".to_string())),
            Some(list(&["en"]))
        );
    }

    /// One servable entry carries the document, so the unservable ones beside it are
    /// preferences we could not honour rather than a bad request.
    #[test]
    fn a_list_is_refused_only_when_no_entry_can_be_served() {
        assert!(reject_unsupported_locales(&list(&["pl-PL", "de-DE"])).is_ok());
        assert!(reject_unsupported_locales(&list(&[])).is_ok());
        assert!(reject_unsupported_locales(&list(&["pl-PL"])).is_err());
    }
}

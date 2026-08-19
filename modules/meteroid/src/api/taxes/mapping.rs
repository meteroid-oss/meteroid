use crate::api::shared::conversions::ProtoConv;
use common_domain::country::CountryCode;
use common_domain::ids::{InvoicingEntityId, ProductId, TaxCategoryId};
use meteroid_grpc::meteroid::api::taxes::v1 as server;
use meteroid_store::domain::accounting::{
    CustomTax, CustomTaxNew, CustomTaxRule, ProductAccounting,
};
use meteroid_store::domain::tax_categories::TaxCategory;
use rust_decimal::Decimal;
use std::str::FromStr;
use tonic::Status;

pub fn custom_tax_new_from_server(value: server::CustomTaxNew) -> Result<CustomTaxNew, Status> {
    Ok(CustomTaxNew {
        invoicing_entity_id: InvoicingEntityId::from_proto(value.invoicing_entity_id)?,
        name: value.name,
        tax_code: value.tax_code,
        tax_category_id: tax_category_id_from_server(value.tax_category_id)?,
        rules: value
            .rules
            .into_iter()
            .map(tax_rule_from_server)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// An absent or empty id means "no category" — the tax stays product-wired.
pub fn tax_category_id_from_server(value: Option<String>) -> Result<Option<TaxCategoryId>, Status> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => Ok(Some(TaxCategoryId::from_proto(s)?)),
    }
}

pub fn tax_rule_from_server(value: server::TaxRule) -> Result<CustomTaxRule, Status> {
    let country = CountryCode::from_proto_opt(value.country)?;

    let region = match (country.as_ref(), value.region.as_deref()) {
        (Some(cc), Some(r)) => {
            if cc.subdivisions().iter().any(|sub| sub.code == r) {
                Some(r.to_string())
            } else {
                return Err(Status::invalid_argument(format!(
                    "Invalid region code '{}' for country '{}'",
                    r, cc.code
                )));
            }
        }
        (Some(_), None) => None,
        (None, Some(_)) => {
            return Err(Status::invalid_argument(
                "Region provided without a country".to_string(),
            ));
        }
        (None, None) => None,
    };

    Ok(CustomTaxRule {
        country,
        region,
        rate: Decimal::from_str(&value.rate)
            .map_err(|_| Status::invalid_argument("Invalid tax rate".to_string()))?,
    })
}

pub fn custom_tax_to_server(value: CustomTax) -> server::CustomTax {
    server::CustomTax {
        id: value.id.as_proto(),
        invoicing_entity_id: value.invoicing_entity_id.as_proto(),
        name: value.name,
        tax_code: value.tax_code,
        tax_category_id: value.tax_category_id.map(|id| id.as_proto()),
        rules: value.rules.into_iter().map(tax_rule_to_server).collect(),
    }
}

pub fn tax_rule_to_server(value: CustomTaxRule) -> server::TaxRule {
    server::TaxRule {
        country: value.country.map(|c| c.as_proto()),
        region: value.region,
        rate: value.rate.as_proto(),
    }
}

pub fn product_accounting_from_server(
    value: server::ProductAccounting,
) -> Result<ProductAccounting, Status> {
    Ok(ProductAccounting {
        product_id: ProductId::from_proto(value.product_id)?,
        invoicing_entity_id: InvoicingEntityId::from_proto(value.invoicing_entity_id)?,
        product_code: value.product_code,
        ledger_account_code: value.ledger_account_code,
    })
}

pub fn product_accounting_to_server(value: ProductAccounting) -> server::ProductAccounting {
    server::ProductAccounting {
        product_id: value.product_id.as_proto(),
        invoicing_entity_id: value.invoicing_entity_id.as_proto(),
        product_code: value.product_code,
        ledger_account_code: value.ledger_account_code,
    }
}

pub fn tax_category_to_server(value: TaxCategory) -> server::TaxCategory {
    server::TaxCategory {
        id: value.id.as_proto(),
        tenant_id: value.tenant_id.map(|t| t.as_proto()),
        parent_id: value.parent_id.map(|p| p.as_proto()),
        key: value.key,
        name: value.name,
        is_builtin: value.is_builtin,
    }
}

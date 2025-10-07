use crate::api::shared::conversions::ProtoConv;
use common_domain::country::CountryCode;
use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId};
use meteroid_grpc::meteroid::api::taxes::v1 as server;
use meteroid_store::domain::accounting::{
    CustomTax, CustomTaxNew, CustomTaxRule, ProductAccounting,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use tonic::Status;

pub fn custom_tax_new_from_server(value: server::CustomTaxNew) -> Result<CustomTaxNew, Status> {
    Ok(CustomTaxNew {
        invoicing_entity_id: InvoicingEntityId::from_proto(value.invoicing_entity_id)?,
        name: value.name,
        tax_code: value.tax_code,
        rules: value
            .rules
            .into_iter()
            .map(tax_rule_from_server)
            .collect::<Result<Vec<_>, _>>()?,
    })
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
        id: value.id.to_string(),
        invoicing_entity_id: value.invoicing_entity_id.to_string(),
        name: value.name,
        tax_code: value.tax_code,
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
        custom_tax_id: CustomTaxId::from_proto_opt(value.custom_tax_id)?,
        product_code: value.product_code,
        ledger_account_code: value.ledger_account_code,
    })
}

pub fn product_accounting_to_server(value: ProductAccounting) -> server::ProductAccounting {
    server::ProductAccounting {
        product_id: value.product_id.as_proto(),
        invoicing_entity_id: value.invoicing_entity_id.as_proto(),
        custom_tax_id: value.custom_tax_id.map(|id| id.as_proto()),
        product_code: value.product_code,
        ledger_account_code: value.ledger_account_code,
    }
}

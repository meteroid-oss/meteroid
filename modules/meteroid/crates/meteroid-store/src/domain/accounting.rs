use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomTaxId, InvoicingEntityId, ProductId};
use diesel_models::accounting::{CustomTaxRow, ProductAccountingRow, ProductAccountingWithTaxRow};
use o2o::o2o;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, o2o)]
#[try_map_owned(CustomTaxRow, StoreErrorReport)]
pub struct CustomTax {
    pub id: CustomTaxId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub tax_code: String,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize rules".to_string(), e)
    }) ?)]
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize rules".to_string(), e)
    }) ?)]
    pub rules: Vec<CustomTaxRule>,
}

impl From<CustomTax> for meteroid_tax::CustomTax {
    fn from(tax: CustomTax) -> Self {
        meteroid_tax::CustomTax {
            reference: tax.id.to_string(),
            name: tax.name,
            tax_rules: tax.rules.into_iter().map(|rule| rule.into()).collect(),
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[owned_try_into(CustomTaxRow, StoreErrorReport)]
#[ghosts(id: {CustomTaxId::new()})]
pub struct CustomTaxNew {
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub tax_code: String,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize rules".to_string(), e)
    }) ?)]
    pub rules: Vec<CustomTaxRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[owned_into(meteroid_tax::TaxRule)]
pub struct CustomTaxRule {
    pub country: Option<CountryCode>,
    pub region: Option<String>,
    pub rate: rust_decimal::Decimal,
}

json_value_serde!(CustomTaxRule);

#[derive(Debug, Clone, o2o)]
#[map_owned(ProductAccountingRow)]
pub struct ProductAccounting {
    pub product_id: ProductId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub custom_tax_id: Option<CustomTaxId>,
    pub product_code: Option<String>,
    pub ledger_account_code: Option<String>,
}

#[derive(Debug, Clone, o2o)]
#[try_from_owned(ProductAccountingWithTaxRow, StoreErrorReport)]
pub struct ProductAccountingWithTax {
    #[child(product_accounting)]
    pub product_id: ProductId,
    #[child(product_accounting)]
    pub invoicing_entity_id: InvoicingEntityId,
    #[child(product_accounting)]
    pub product_code: Option<String>,
    #[child(product_accounting)]
    pub ledger_account_code: Option<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub custom_tax: Option<CustomTax>,
}

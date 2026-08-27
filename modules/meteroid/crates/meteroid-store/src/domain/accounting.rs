use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::country::CountryCode;
use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId, TaxCategoryId};
use diesel_models::accounting::{CustomTaxRow, ProductAccountingRow};
use o2o::o2o;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, o2o)]
#[try_from_owned(CustomTaxRow, StoreErrorReport)]
pub struct TaxRate {
    pub id: CustomTaxId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub tax_code: String,
    /// When set, the tax applies to every line resolving to this category,
    /// on top of any product explicitly linked to it.
    pub tax_category_id: Option<TaxCategoryId>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize rules".to_string(), e)
    }) ?)]
    pub rules: Vec<TaxRateRule>,
}

impl From<TaxRate> for meteroid_tax::TaxRate {
    fn from(tax: TaxRate) -> Self {
        meteroid_tax::TaxRate {
            // The accounting/reporting code is the breakdown reference (W1).
            reference: tax.tax_code,
            name: tax.name,
            tax_rules: tax
                .rules
                .into_iter()
                .map(std::convert::Into::into)
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaxRateNew {
    pub invoicing_entity_id: InvoicingEntityId,
    pub name: String,
    pub tax_code: String,
    pub tax_category_id: Option<TaxCategoryId>,
    pub rules: Vec<TaxRateRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[owned_into(meteroid_tax::TaxRateRule)]
pub struct TaxRateRule {
    pub country: Option<CountryCode>,
    pub region: Option<String>,
    pub rate: rust_decimal::Decimal,
}

json_value_serde!(TaxRateRule);

#[derive(Debug, Clone, o2o)]
#[map_owned(ProductAccountingRow)]
pub struct ProductAccounting {
    pub product_id: ProductId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub product_code: Option<String>,
    pub ledger_account_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProductAccountingWithTaxes {
    pub product_id: ProductId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub product_code: Option<String>,
    pub ledger_account_code: Option<String>,
    pub custom_taxes: Vec<TaxRate>,
}

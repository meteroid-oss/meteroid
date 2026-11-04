//! What the tax engines are told about a customer and about the products being sold.
//!
//! Every caller that resolves tax goes through here, so an invoice and the subscription
//! whose MRR is netted against it can never disagree about who is being taxed or about
//! what a product carries. That matters most for the `Manual` resolver, where the tax
//! lives *only* in the product/category configuration: a caller that asked
//! `calculate_customer_tax` alone would be told "no tax" and silently miss all of it.

use crate::StoreResult;
use crate::domain::accounting::ProductAccountingWithTaxes;
use crate::domain::{Customer, InvoicingEntity, VatNumberValidationStatus};
use crate::repositories::accounting::AccountingInterface;
use crate::services::Services;
use common_domain::ids::{ProductId, TaxCategoryId};
use itertools::Itertools;
use std::collections::HashMap;

/// The customer as the tax engines read them. `billing_address` is passed in because a
/// customer without one is not taxable at all, and each caller decides what to do about
/// that before getting this far.
pub(in crate::services) fn customer_for_tax(
    customer: &Customer,
    invoicing_entity: &InvoicingEntity,
    billing_address: crate::domain::Address,
) -> meteroid_tax::CustomerForTax {
    meteroid_tax::CustomerForTax {
        vat_number: customer.vat_number.clone(),
        vat_number_format_valid: customer.vat_number_format_valid,
        vat_number_vies_valid: match customer.vat_number_validation_status {
            Some(VatNumberValidationStatus::Valid) => Some(true),
            Some(VatNumberValidationStatus::Invalid) => Some(false),
            _ => None,
        },
        require_vies_valid_for_reverse_charge: invoicing_entity
            .require_vies_valid_for_reverse_charge,
        custom_tax_rates: customer
            .custom_taxes
            .iter()
            .map(|t| meteroid_tax::CustomerTaxRate {
                tax_code: t.tax_code.clone(),
                name: t.name.clone(),
                rate: t.rate,
            })
            .collect(),
        tax_status: customer.tax_status.into(),
        exemption_reason: customer.exemption_reason.clone(),
        billing_address: billing_address.into(),
        shipping_address: customer
            .shipping_address
            .as_ref()
            .filter(|s| !s.same_as_billing)
            .and_then(|s| s.address.clone())
            .map(Into::into),
    }
}

/// The tax configuration a set of lines resolves against: the taxes wired to each
/// product, the category each line falls into (the product's, else the invoicing entity
/// default), and the taxes configured against that category.
///
/// Loaded once for a set of lines, then applied line by line. The set of lines is fixed at
/// load time because only the categories actually reached are worth resolving.
pub(in crate::services) struct ProductTaxConfig {
    product_categories: HashMap<ProductId, Option<TaxCategoryId>>,
    default_category: Option<TaxCategoryId>,
    product_taxes: Vec<ProductAccountingWithTaxes>,
    category_keys: HashMap<TaxCategoryId, String>,
    category_taxes: HashMap<TaxCategoryId, Vec<crate::domain::accounting::TaxRate>>,
}

impl ProductTaxConfig {
    /// The category a line resolves to: its product's, else the invoicing entity default.
    fn category_of(&self, product_id: Option<ProductId>) -> Option<TaxCategoryId> {
        product_id
            .and_then(|p| self.product_categories.get(&p).copied().flatten())
            .or(self.default_category)
    }

    /// The custom taxes and resolved category key one line carries, in the shape the tax
    /// engine takes them.
    pub(in crate::services) fn for_product(
        &self,
        product_id: Option<ProductId>,
    ) -> (Vec<meteroid_tax::TaxRate>, Option<String>) {
        let category_id = self.category_of(product_id);

        let product_linked = product_id
            .and_then(|p| self.product_taxes.iter().find(|tax| tax.product_id == p))
            .map(|p| p.custom_taxes.as_slice())
            .unwrap_or_default();

        let from_category = category_id
            .and_then(|id| self.category_taxes.get(&id))
            .map(Vec::as_slice)
            .unwrap_or_default();

        let custom_taxes = product_linked
            .iter()
            .chain(from_category)
            .unique_by(|t| t.id)
            .map(|t| meteroid_tax::TaxRate {
                // The accounting/reporting code is the breakdown reference (W1).
                reference: t.tax_code.clone(),
                name: t.name.clone(),
                tax_rules: t
                    .rules
                    .iter()
                    .cloned()
                    .map(std::convert::Into::into)
                    .collect(),
            })
            .collect();

        let tax_category = category_id.and_then(|id| self.category_keys.get(&id).cloned());

        (custom_taxes, tax_category)
    }
}

impl Services {
    /// The tax configuration for a set of lines, identified by the product each one carries
    /// (`None` for a line with no product, which still resolves to the entity's default
    /// category), read on the caller's connection: a second one could wait on a drained pool.
    pub(in crate::services) async fn load_product_tax_config_with_conn(
        &self,
        conn: &mut crate::store::PgConn,
        invoicing_entity: &InvoicingEntity,
        line_products: &[Option<ProductId>],
    ) -> StoreResult<ProductTaxConfig> {
        product_tax_config_with_conn(&self.store, conn, invoicing_entity, line_products).await
    }
}

pub(in crate::services) async fn product_tax_config_with_conn(
    store: &crate::Store,
    conn: &mut crate::store::PgConn,
    invoicing_entity: &InvoicingEntity,
    line_products: &[Option<ProductId>],
) -> StoreResult<ProductTaxConfig> {
    use diesel_models::products::ProductRow;
    use diesel_models::tax_categories::TaxCategoryRow;
    use error_stack::Report;

    let product_ids: Vec<ProductId> = line_products.iter().flatten().copied().unique().collect();

    let product_categories: HashMap<ProductId, Option<TaxCategoryId>> =
        ProductRow::list_by_ids(conn, &product_ids, invoicing_entity.tenant_id)
            .await
            .map_err(Into::<Report<crate::errors::StoreError>>::into)?
            .into_iter()
            .map(|p| (p.id, p.tax_category_id))
            .collect();

    let product_taxes = store
        .list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
            conn,
            invoicing_entity.tenant_id,
            product_ids,
            invoicing_entity.id,
        )
        .await?;

    let default_category = invoicing_entity.default_tax_category_id;
    let resolved_category_ids: Vec<TaxCategoryId> = line_products
        .iter()
        .filter_map(|product_id| {
            product_id
                .and_then(|p| product_categories.get(&p).copied().flatten())
                .or(default_category)
        })
        .unique()
        .collect();

    // Categories are only worth resolving when a line actually carries one.
    // The stable key drives the non-taxable short-circuit; all other categories
    // are standard-rated by the engine.
    let category_keys: HashMap<TaxCategoryId, String> = if resolved_category_ids.is_empty() {
        HashMap::new()
    } else {
        TaxCategoryRow::list_available(conn, invoicing_entity.tenant_id)
            .await
            .map_err(Into::<Report<crate::errors::StoreError>>::into)?
            .into_iter()
            .map(|c| (c.id, c.key))
            .collect()
    };

    // Custom taxes configured against a category apply to every line in it, on
    // top of the taxes wired to the product directly.
    let category_taxes = store
        .list_custom_taxes_by_categories(
            conn,
            invoicing_entity.tenant_id,
            invoicing_entity.id,
            &resolved_category_ids,
        )
        .await?;

    Ok(ProductTaxConfig {
        product_categories,
        default_category,
        product_taxes,
        category_keys,
        category_taxes,
    })
}

use crate::StoreResult;
use crate::domain::accounting::{
    ProductAccounting, ProductAccountingWithTaxes, TaxRate, TaxRateNew, TaxRateRule,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BaseId, CustomTaxId, InvoicingEntityId, ProductId, TaxCategoryId, TenantId,
};
use diesel_models::accounting::{CustomTaxRow, ProductAccountingRow, ProductAccountingWithTaxRow};
use error_stack::Report;
use std::collections::HashMap;

/// Builds a persistable tax-rate row, enforcing that the accounting `tax_code`
/// is present. Uniqueness of the code per tenant is enforced by the database
/// (`custom_tax_tenant_tax_code_key`); a collision surfaces as `DuplicateValue`.
fn build_custom_tax_row(
    id: CustomTaxId,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    name: String,
    tax_code: String,
    tax_category_id: Option<TaxCategoryId>,
    rules: &[TaxRateRule],
) -> StoreResult<CustomTaxRow> {
    if tax_code.trim().is_empty() {
        return Err(StoreError::InvalidArgument(
            "tax code is required and is used as the accounting reference".to_string(),
        )
        .into());
    }

    let rules = serde_json::to_value(rules)
        .map_err(|e| StoreError::SerdeError("Failed to serialize rules".to_string(), e))?;

    Ok(CustomTaxRow {
        id,
        invoicing_entity_id,
        name,
        tax_code,
        rules,
        tax_category_id,
        tenant_id,
    })
}

#[async_trait::async_trait]
pub trait AccountingInterface {
    async fn insert_custom_tax(&self, tenant_id: TenantId, tax: TaxRateNew)
    -> StoreResult<TaxRate>;
    async fn update_custom_tax(&self, tenant_id: TenantId, tax: TaxRate) -> StoreResult<TaxRate>;
    async fn delete_custom_tax(&self, tenant_id: TenantId, tax_id: CustomTaxId) -> StoreResult<()>;
    async fn list_custom_taxes_by_invoicing_entity_id(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<TaxRate>>;

    async fn upsert_product_accounting(
        &self,
        tenant_id: TenantId,
        product_accounting: ProductAccounting,
    ) -> StoreResult<ProductAccounting>;

    async fn list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_ids: Vec<ProductId>,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<ProductAccountingWithTaxes>>;

    /// Custom taxes that target one of these categories, keyed by category.
    async fn list_custom_taxes_by_categories(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        category_ids: &[TaxCategoryId],
    ) -> StoreResult<HashMap<TaxCategoryId, Vec<TaxRate>>>;
}

#[async_trait::async_trait]
impl AccountingInterface for Store {
    async fn insert_custom_tax(
        &self,
        tenant_id: TenantId,
        tax: TaxRateNew,
    ) -> StoreResult<TaxRate> {
        let mut conn = self.get_conn().await?;
        let tax_row = build_custom_tax_row(
            CustomTaxId::new(),
            tenant_id,
            tax.invoicing_entity_id,
            tax.name,
            tax.tax_code,
            tax.tax_category_id,
            &tax.rules,
        )?;

        let inserted_tax = tax_row
            .upsert(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted_tax.try_into()?)
    }

    async fn update_custom_tax(&self, tenant_id: TenantId, tax: TaxRate) -> StoreResult<TaxRate> {
        let mut conn = self.get_conn().await?;
        let tax_row = build_custom_tax_row(
            tax.id,
            tenant_id,
            tax.invoicing_entity_id,
            tax.name,
            tax.tax_code,
            tax.tax_category_id,
            &tax.rules,
        )?;

        let updated_tax = tax_row
            .upsert(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated_tax.try_into()?)
    }

    async fn delete_custom_tax(&self, tenant_id: TenantId, tax_id: CustomTaxId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        CustomTaxRow::delete(&mut conn, tax_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn list_custom_taxes_by_invoicing_entity_id(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<TaxRate>> {
        let mut conn = self.get_conn().await?;
        let tax_rows =
            CustomTaxRow::list_by_invoicing_entity_id(&mut conn, invoicing_entity_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let custom_taxes = tax_rows
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<TaxRate>, _>>()?;

        Ok(custom_taxes)
    }

    async fn upsert_product_accounting(
        &self,
        tenant_id: TenantId,
        product_accounting: ProductAccounting,
    ) -> StoreResult<ProductAccounting> {
        let mut conn = self.get_conn().await?;
        let product_accounting_row: ProductAccountingRow = product_accounting.into();

        let inserted_product_accounting = product_accounting_row
            .upsert(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted_product_accounting.into())
    }

    async fn list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_ids: Vec<ProductId>,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<ProductAccountingWithTaxes>> {
        let product_accounting_rows =
            ProductAccountingWithTaxRow::list_by_product_ids_and_invoicing_entity_id(
                conn,
                product_ids,
                invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Group by product_id
        let mut grouped: HashMap<ProductId, ProductAccountingWithTaxes> = HashMap::new();

        for row in product_accounting_rows {
            let product_id = row.product_accounting.product_id;
            let invoicing_entity_id = row.product_accounting.invoicing_entity_id;

            let entry = grouped
                .entry(product_id)
                .or_insert_with(|| ProductAccountingWithTaxes {
                    product_id,
                    invoicing_entity_id,
                    product_code: row.product_accounting.product_code.clone(),
                    ledger_account_code: row.product_accounting.ledger_account_code.clone(),
                    custom_taxes: Vec::new(),
                });

            if let Some(tax_row) = row.custom_tax {
                let custom_tax: TaxRate = tax_row.try_into()?;
                entry.custom_taxes.push(custom_tax);
            }
        }

        Ok(grouped.into_values().collect())
    }

    async fn list_custom_taxes_by_categories(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        category_ids: &[TaxCategoryId],
    ) -> StoreResult<HashMap<TaxCategoryId, Vec<TaxRate>>> {
        let rows = CustomTaxRow::list_by_invoicing_entity_and_categories(
            conn,
            invoicing_entity_id,
            tenant_id,
            category_ids,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut grouped: HashMap<TaxCategoryId, Vec<TaxRate>> = HashMap::new();

        for row in rows {
            let Some(category_id) = row.tax_category_id else {
                continue;
            };
            grouped
                .entry(category_id)
                .or_default()
                .push(row.try_into()?);
        }

        Ok(grouped)
    }
}

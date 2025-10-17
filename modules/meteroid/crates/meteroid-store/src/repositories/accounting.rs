use crate::StoreResult;
use crate::domain::accounting::{
    CustomTax, CustomTaxNew, ProductAccounting, ProductAccountingWithTaxes,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId, TenantId};
use diesel_models::accounting::{CustomTaxRow, ProductAccountingRow, ProductAccountingWithTaxRow};
use error_stack::Report;
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait AccountingInterface {
    async fn insert_custom_tax(
        &self,
        tenant_id: TenantId,
        tax: CustomTaxNew,
    ) -> StoreResult<CustomTax>;
    async fn update_custom_tax(
        &self,
        tenant_id: TenantId,
        tax: CustomTax,
    ) -> StoreResult<CustomTax>;
    async fn delete_custom_tax(&self, tenant_id: TenantId, tax_id: CustomTaxId) -> StoreResult<()>;
    async fn list_custom_taxes_by_invoicing_entity_id(
        &self,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<Vec<CustomTax>>;

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
}

#[async_trait::async_trait]
impl AccountingInterface for Store {
    async fn insert_custom_tax(
        &self,
        tenant_id: TenantId,
        tax: CustomTaxNew,
    ) -> StoreResult<CustomTax> {
        let mut conn = self.get_conn().await?;
        let tax_row: CustomTaxRow = tax.try_into()?;

        let inserted_tax = tax_row
            .upsert(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted_tax.try_into()?)
    }

    async fn update_custom_tax(
        &self,
        tenant_id: TenantId,
        tax: CustomTax,
    ) -> StoreResult<CustomTax> {
        let mut conn = self.get_conn().await?;
        let tax_row: CustomTaxRow = tax.try_into()?;

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
    ) -> StoreResult<Vec<CustomTax>> {
        let mut conn = self.get_conn().await?;
        let tax_rows =
            CustomTaxRow::list_by_invoicing_entity_id(&mut conn, invoicing_entity_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let custom_taxes = tax_rows
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<CustomTax>, _>>()?;

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
                let custom_tax: CustomTax = tax_row.try_into()?;
                entry.custom_taxes.push(custom_tax);
            }
        }

        Ok(grouped.into_values().collect())
    }
}

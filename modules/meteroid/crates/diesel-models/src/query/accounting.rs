use crate::accounting::{CustomTaxRow, ProductAccountingRow, ProductAccountingWithTaxRow};
use crate::errors::{DatabaseError, DatabaseErrorContainer, IntoDbResult};
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId, TenantId};
use diesel::upsert::excluded;
use diesel::{
    ExpressionMethods, JoinOnDsl, NullableExpressionMethods, OptionalExtension, QueryDsl,
    SelectableHelper, debug_query,
};
use error_stack::ResultExt;

impl CustomTaxRow {
    pub async fn upsert(&self, conn: &mut PgConn, tenant_id: TenantId) -> DbResult<CustomTaxRow> {
        use crate::schema::{custom_tax::dsl as ct_dsl, invoicing_entity::dsl as ie_dsl};
        use diesel_async::RunQueryDsl;

        let invoicing_entity_exists = ie_dsl::invoicing_entity
            .filter(ie_dsl::id.eq(self.invoicing_entity_id))
            .filter(ie_dsl::tenant_id.eq(tenant_id))
            .select(ie_dsl::id)
            .first::<InvoicingEntityId>(conn)
            .await
            .optional()
            .attach_printable("Error while checking invoicing entity tenant")
            .into_db_result()?;

        if invoicing_entity_exists.is_none() {
            return Err(DatabaseErrorContainer::from(
                DatabaseError::ValidationError(
                    "Invoicing entity not found or does not belong to tenant".to_string(),
                ),
            ));
        }

        let query = diesel::insert_into(ct_dsl::custom_tax)
            .values(self)
            .on_conflict(ct_dsl::id)
            .do_update()
            .set((
                ct_dsl::name.eq(excluded(ct_dsl::name)),
                ct_dsl::tax_code.eq(excluded(ct_dsl::tax_code)),
                ct_dsl::rules.eq(excluded(ct_dsl::rules)),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting custom_tax")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        tax_id: CustomTaxId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::{custom_tax::dsl as ct_dsl, invoicing_entity::dsl as ie_dsl};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(ct_dsl::custom_tax)
            .filter(ct_dsl::id.eq(tax_id))
            .filter(
                ct_dsl::invoicing_entity_id.eq_any(
                    ie_dsl::invoicing_entity
                        .filter(ie_dsl::tenant_id.eq(tenant_id))
                        .select(ie_dsl::id),
                ),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting custom_tax with tenant check")
            .into_db_result()
    }

    pub async fn list_by_invoicing_entity_id(
        conn: &mut PgConn,
        param_id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<CustomTaxRow>> {
        use crate::schema::{custom_tax::dsl as ct_dsl, invoicing_entity::dsl as ie_dsl};
        use diesel_async::RunQueryDsl;

        let query = ct_dsl::custom_tax
            .filter(ct_dsl::invoicing_entity_id.eq(param_id))
            .filter(
                ct_dsl::invoicing_entity_id.eq_any(
                    ie_dsl::invoicing_entity
                        .filter(ie_dsl::tenant_id.eq(tenant_id))
                        .select(ie_dsl::id),
                ),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable(
                "Error while fetching custom_tax by invoicing entity id with tenant check",
            )
            .into_db_result()
    }
}

impl ProductAccountingRow {
    pub async fn upsert(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<ProductAccountingRow> {
        use crate::schema::{invoicing_entity::dsl as ie_dsl, product_accounting::dsl as pa_dsl};
        use diesel_async::RunQueryDsl;

        let _invoicing_entity_exists = ie_dsl::invoicing_entity
            .filter(ie_dsl::id.eq(self.invoicing_entity_id))
            .filter(ie_dsl::tenant_id.eq(tenant_id))
            .select(ie_dsl::id)
            .first::<InvoicingEntityId>(conn)
            .await
            .attach_printable("Invoicing entity not found or does not belong to tenan")
            .into_db_result()?;

        let query = diesel::insert_into(pa_dsl::product_accounting)
            .values(self)
            .on_conflict((pa_dsl::product_id, pa_dsl::invoicing_entity_id))
            .do_update()
            .set((
                pa_dsl::product_code.eq(excluded(pa_dsl::product_code)),
                pa_dsl::custom_tax_id.eq(excluded(pa_dsl::custom_tax_id)),
                pa_dsl::ledger_account_code.eq(excluded(pa_dsl::ledger_account_code)),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting product_accounting")
            .into_db_result()
    }
}

impl ProductAccountingWithTaxRow {
    pub async fn list_by_product_id(
        conn: &mut PgConn,
        param_product_id: ProductId,
    ) -> DbResult<Vec<ProductAccountingWithTaxRow>> {
        use crate::schema::custom_tax::dsl as ct_dsl;
        use crate::schema::product_accounting::dsl as pa_dsl;
        use diesel_async::RunQueryDsl;

        let query = pa_dsl::product_accounting
            .filter(pa_dsl::product_id.eq(param_product_id))
            .left_join(ct_dsl::custom_tax.on(pa_dsl::custom_tax_id.eq(ct_dsl::id.nullable())))
            .select(ProductAccountingWithTaxRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching api token by id")
            .into_db_result()
    }

    pub async fn list_by_product_ids_and_invoicing_entity_id(
        conn: &mut PgConn,
        param_products_ids: Vec<ProductId>,
        param_invoicing_entity_id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<ProductAccountingWithTaxRow>> {
        use crate::schema::custom_tax::dsl as ct_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use crate::schema::product_accounting::dsl as pa_dsl;
        use diesel_async::RunQueryDsl;

        let query = pa_dsl::product_accounting
            .filter(pa_dsl::invoicing_entity_id.eq(param_invoicing_entity_id))
            .filter(pa_dsl::product_id.eq_any(&param_products_ids))
            .filter(
                pa_dsl::invoicing_entity_id.eq_any(
                    ie_dsl::invoicing_entity
                        .filter(ie_dsl::tenant_id.eq(tenant_id))
                        .select(ie_dsl::id),
                ),
            )
            .left_join(ct_dsl::custom_tax.on(pa_dsl::custom_tax_id.eq(ct_dsl::id.nullable())))
            .select(ProductAccountingWithTaxRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching product accounting with tenant check")
            .into_db_result()
    }
}

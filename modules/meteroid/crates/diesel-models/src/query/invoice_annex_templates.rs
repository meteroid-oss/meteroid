use crate::errors::IntoDbResult;
use crate::invoice_annex_templates::{InvoiceAnnexTemplateRow, PlanVersionAnnexTemplateRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{InvoiceAnnexTemplateId, PlanVersionId, TenantId};
use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl InvoiceAnnexTemplateRow {
    /// Templates bound to a plan version, used when resolving annexes for that plan's invoices.
    pub async fn list_for_plan_version(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> DbResult<Vec<InvoiceAnnexTemplateRow>> {
        use crate::schema::invoice_annex_template::dsl as t_dsl;
        use crate::schema::plan_version_annex_template::dsl as l_dsl;

        let query = t_dsl::invoice_annex_template
            .inner_join(
                l_dsl::plan_version_annex_template.on(l_dsl::annex_template_id.eq(t_dsl::id)),
            )
            .filter(l_dsl::plan_version_id.eq(plan_version_id))
            .filter(t_dsl::tenant_id.eq(tenant_id))
            .select(InvoiceAnnexTemplateRow::as_select());
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching invoice_annex_templates for plan version")
            .into_db_result()
    }
}

impl PlanVersionAnnexTemplateRowNew {
    /// Clones all annex-template bindings from one plan version to another (used when a new
    /// plan version is drafted from an existing one, so annexes carry forward).
    pub async fn clone_all(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
    ) -> DbResult<usize> {
        use crate::schema::plan_version_annex_template::dsl as l_dsl;

        let existing: Vec<(InvoiceAnnexTemplateId, TenantId)> = l_dsl::plan_version_annex_template
            .filter(l_dsl::plan_version_id.eq(src_plan_version_id))
            .select((l_dsl::annex_template_id, l_dsl::tenant_id))
            .get_results(conn)
            .await
            .attach("Error while listing plan_version_annex_template to clone")
            .into_db_result()?;

        if existing.is_empty() {
            return Ok(0);
        }

        let rows: Vec<PlanVersionAnnexTemplateRowNew> = existing
            .into_iter()
            .map(
                |(annex_template_id, tenant_id)| PlanVersionAnnexTemplateRowNew {
                    plan_version_id: dst_plan_version_id,
                    annex_template_id,
                    tenant_id,
                },
            )
            .collect();

        diesel::insert_into(l_dsl::plan_version_annex_template)
            .values(&rows)
            .execute(conn)
            .await
            .attach("Error while cloning plan_version_annex_template")
            .into_db_result()
    }
}

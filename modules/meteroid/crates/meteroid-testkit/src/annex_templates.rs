//! Annex templates are written by the annexes module; core billing tests seed them here.

use common_domain::ids::{BaseId, InvoiceAnnexTemplateId, PlanVersionId, TenantId};
use diesel::{ExpressionMethods, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::invoice_annex_templates::InvoiceAnnexTemplateRow;
use diesel_models::schema::{invoice_annex_template, plan_version_annex_template};
use meteroid_store::Store;
use meteroid_store::domain::invoice_annex_templates::{
    AnnexDefinition, AnnexTemplate, AnnexTemplateNew,
};

pub async fn create_annex_template(store: &Store, new: AnnexTemplateNew) -> AnnexTemplate {
    let definition = match &new.definition {
        AnnexDefinition::EventTable(d) => serde_json::to_value(d),
        AnnexDefinition::StaticText(d) => serde_json::to_value(d),
    }
    .expect("serialize annex definition");

    let mut conn = store.get_conn().await.expect("db connection");
    let row = diesel::insert_into(invoice_annex_template::table)
        .values((
            invoice_annex_template::id.eq(InvoiceAnnexTemplateId::new()),
            invoice_annex_template::tenant_id.eq(new.tenant_id),
            invoice_annex_template::name.eq(new.name),
            invoice_annex_template::kind.eq(new.definition.kind()),
            invoice_annex_template::definition.eq(definition),
        ))
        .returning(InvoiceAnnexTemplateRow::as_returning())
        .get_result::<InvoiceAnnexTemplateRow>(&mut conn)
        .await
        .expect("insert annex template");

    AnnexTemplate::try_from(row).expect("annex template")
}

/// Replaces the full set of annex templates bound to a plan version.
pub async fn set_plan_version_annex_templates(
    store: &Store,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    template_ids: &[InvoiceAnnexTemplateId],
) {
    let mut conn = store.get_conn().await.expect("db connection");

    diesel::delete(plan_version_annex_template::table)
        .filter(plan_version_annex_template::plan_version_id.eq(plan_version_id))
        .filter(plan_version_annex_template::tenant_id.eq(tenant_id))
        .execute(&mut conn)
        .await
        .expect("clear plan version annex templates");

    if template_ids.is_empty() {
        return;
    }

    let bindings: Vec<_> = template_ids
        .iter()
        .map(|id| {
            (
                plan_version_annex_template::plan_version_id.eq(plan_version_id),
                plan_version_annex_template::annex_template_id.eq(*id),
                plan_version_annex_template::tenant_id.eq(tenant_id),
            )
        })
        .collect();
    diesel::insert_into(plan_version_annex_template::table)
        .values(bindings)
        .execute(&mut conn)
        .await
        .expect("bind annex templates to plan version");
}

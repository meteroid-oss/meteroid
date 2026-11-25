use crate::errors::IntoDbResult;
use crate::product_family_journeys::ProductFamilyJourneyRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{ProductFamilyId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl ProductFamilyJourneyRow {
    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> DbResult<Option<ProductFamilyJourneyRow>> {
        use crate::schema::product_family_journey::dsl as j;

        let query = j::product_family_journey
            .filter(j::tenant_id.eq(tenant_id))
            .filter(j::product_family_id.eq(product_family_id))
            .select(ProductFamilyJourneyRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a product family's journey")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<ProductFamilyJourneyRow>> {
        use crate::schema::product_family_journey::dsl as j;

        let query = j::product_family_journey
            .filter(j::tenant_id.eq(tenant_id))
            .select(ProductFamilyJourneyRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing journeys")
            .into_db_result()
    }

    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<ProductFamilyJourneyRow> {
        use crate::schema::product_family_journey::dsl as j;

        let query = diesel::insert_into(j::product_family_journey)
            .values(self)
            .on_conflict(j::product_family_id)
            .do_update()
            .set((
                j::signup_plan_id.eq(self.signup_plan_id),
                j::signup_option.eq(self.signup_option.clone()),
                j::period_switch.eq(self.period_switch),
                j::self_serve_downgrade.eq(self.self_serve_downgrade),
                j::updated_at.eq(common_clock::now()),
            ))
            .returning(ProductFamilyJourneyRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while saving a product family's journey")
            .into_db_result()
    }
}

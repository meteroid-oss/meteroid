use crate::errors::IntoDbResult;
use crate::price_components::{PriceComponentRow, PriceComponentRowNew};
use std::collections::HashMap;

use crate::{DbResult, PgConn};

use common_domain::ids::{BaseId, PlanVersionId, PriceComponentId, ProductId, TenantId};
use diesel::{
    ExpressionMethods, Insertable, IntoSql, OptionalExtension, QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use itertools::Itertools;
use tap::prelude::*;

/// Fetch distinct product IDs referenced by price components for the given plan version.
pub async fn list_product_ids_for_plan_version(
    conn: &mut PgConn,
    tenant_id: &TenantId,
    plan_version_id: PlanVersionId,
) -> DbResult<Vec<ProductId>> {
    use crate::schema::plan_version::dsl as pv_dsl;
    use crate::schema::price_component::dsl as pc_dsl;
    use diesel::dsl::not;
    use diesel_async::RunQueryDsl;

    let query = pc_dsl::price_component
        .inner_join(pv_dsl::plan_version)
        .filter(pc_dsl::plan_version_id.eq(plan_version_id))
        .filter(pv_dsl::tenant_id.eq(tenant_id))
        .filter(not(pc_dsl::product_id.is_null()))
        .select(pc_dsl::product_id);

    crate::debug_sql(&query);

    let rows: Vec<Option<ProductId>> = query
        .get_results(conn)
        .await
        .tap_err(|e| log::error!("Error while fetching product ids for plan version: {e:?}"))
        .attach("Error while fetching product ids for plan version")
        .into_db_result()?;

    Ok(rows.into_iter().flatten().collect())
}

#[derive(diesel::QueryableByName)]
struct PriceChargeStructureRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    price_id: common_domain::ids::PriceId,
    #[diesel(sql_type = diesel::sql_types::Jsonb)]
    fee_structure: serde_json::Value,
}

/// The structures of the charges each of `price_ids` is linked to: plan components (in any
/// option) and add-ons, once each.
pub async fn charge_structures_of_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    price_ids: &[common_domain::ids::PriceId],
) -> DbResult<Vec<(common_domain::ids::PriceId, serde_json::Value)>> {
    use diesel::sql_types::{Array, Uuid};
    use diesel_async::RunQueryDsl;

    if price_ids.is_empty() {
        return Ok(Vec::new());
    }
    let query = diesel::sql_query(
        "SELECT DISTINCT pcp.price_id, pc.fee_structure
         FROM plan_component_price pcp
         JOIN price_component pc ON pc.id = pcp.plan_component_id
         JOIN plan_version pv ON pv.id = pc.plan_version_id
         WHERE pv.tenant_id = $1 AND pcp.price_id = ANY($2) AND pc.fee_structure IS NOT NULL
         UNION
         SELECT ap.price_id, a.fee_structure
         FROM add_on_price ap
         JOIN add_on a ON a.id = ap.add_on_id
         WHERE a.tenant_id = $1 AND ap.price_id = ANY($2)",
    )
    .bind::<Uuid, _>(tenant_id)
    .bind::<Array<Uuid>, _>(price_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>());
    crate::debug_sql(&query);
    query
        .load::<PriceChargeStructureRow>(conn)
        .await
        .attach("Error while reading the structures of the charges of prices")
        .into_db_result()
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.price_id, row.fee_structure))
                .collect()
        })
}

#[derive(diesel::QueryableByName)]
struct ProductChargeStructureRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    product_id: ProductId,
    #[diesel(sql_type = diesel::sql_types::Jsonb)]
    fee_structure: serde_json::Value,
}

/// How the live charges of each of `product_ids` bill it: the components of a live plan's
/// active or draft version, and live add-ons; each structure once per product.
pub async fn charge_structures_of_products(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_ids: &[ProductId],
) -> DbResult<Vec<(ProductId, serde_json::Value)>> {
    use diesel::sql_types::{Array, Uuid};
    use diesel_async::RunQueryDsl;

    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    let query = diesel::sql_query(
        "SELECT pc.product_id, pc.fee_structure
         FROM price_component pc
         JOIN plan_version pv ON pv.id = pc.plan_version_id
         JOIN plan p ON p.id = pv.plan_id
         WHERE pv.tenant_id = $1 AND pc.product_id = ANY($2) AND pc.fee_structure IS NOT NULL
           AND p.archived_at IS NULL
           AND (p.active_version_id = pv.id OR p.draft_version_id = pv.id)
         UNION
         SELECT a.product_id, a.fee_structure
         FROM add_on a
         WHERE a.tenant_id = $1 AND a.product_id = ANY($2) AND a.archived_at IS NULL",
    )
    .bind::<Uuid, _>(tenant_id)
    .bind::<Array<Uuid>, _>(
        product_ids
            .iter()
            .map(|id| id.as_uuid())
            .collect::<Vec<_>>(),
    );
    crate::debug_sql(&query);
    query
        .load::<ProductChargeStructureRow>(conn)
        .await
        .attach("Error while reading the structures of the charges of products")
        .into_db_result()
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.product_id, row.fee_structure))
                .collect()
        })
}

#[derive(diesel::QueryableByName)]
struct ChargeStructureByIdRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Jsonb)]
    fee_structure: serde_json::Value,
}

/// The structures of the plan components `component_ids` and the add-ons `add_on_ids` (of the
/// tenant), by id: what a subscription row's fee is priced in.
pub async fn charge_structures_by_ids(
    conn: &mut PgConn,
    tenant_id: TenantId,
    component_ids: &[PriceComponentId],
    add_on_ids: &[common_domain::ids::AddOnId],
) -> DbResult<(
    HashMap<PriceComponentId, serde_json::Value>,
    HashMap<common_domain::ids::AddOnId, serde_json::Value>,
)> {
    use diesel::sql_types::{Array, Uuid};
    use diesel_async::RunQueryDsl;

    let mut components = HashMap::new();
    if !component_ids.is_empty() {
        let query = diesel::sql_query(
            "SELECT pc.id, pc.fee_structure
             FROM price_component pc
             JOIN plan_version pv ON pv.id = pc.plan_version_id
             WHERE pv.tenant_id = $1 AND pc.id = ANY($2) AND pc.fee_structure IS NOT NULL",
        )
        .bind::<Uuid, _>(tenant_id)
        .bind::<Array<Uuid>, _>(
            component_ids
                .iter()
                .map(|id| id.as_uuid())
                .collect::<Vec<_>>(),
        );
        crate::debug_sql(&query);
        for row in query
            .load::<ChargeStructureByIdRow>(conn)
            .await
            .attach("Error while reading the structures of plan charges")
            .into_db_result()?
        {
            components.insert(PriceComponentId::from(row.id), row.fee_structure);
        }
    }
    let mut add_ons = HashMap::new();
    if !add_on_ids.is_empty() {
        let query = diesel::sql_query(
            "SELECT a.id, a.fee_structure FROM add_on a WHERE a.tenant_id = $1 AND a.id = ANY($2)",
        )
        .bind::<Uuid, _>(tenant_id)
        .bind::<Array<Uuid>, _>(add_on_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>());
        crate::debug_sql(&query);
        for row in query
            .load::<ChargeStructureByIdRow>(conn)
            .await
            .attach("Error while reading the structures of add-ons")
            .into_db_result()?
        {
            add_ons.insert(common_domain::ids::AddOnId::from(row.id), row.fee_structure);
        }
    }
    Ok((components, add_ons))
}

impl PriceComponentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PriceComponentRow> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }
}

impl PriceComponentRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: PriceComponentId,
    ) -> DbResult<PriceComponentRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version)
            .filter(pv_dsl::tenant_id.eq(param_tenant_id))
            .filter(pc_dsl::id.eq(param_id))
            .select(PriceComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while fetching price component")
            .into_db_result()
    }

    pub async fn insert(
        conn: &mut PgConn,
        price_component_param: PriceComponentRowNew,
    ) -> DbResult<PriceComponentRow> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_component_param);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price component: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        price_components: Vec<PriceComponentRowNew>,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::price_component::dsl::price_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price_component).values(&price_components);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while inserting price components: {e:?}"))
            .attach("Error while inserting price component")
            .into_db_result()
    }

    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        plan_version_id_param: PlanVersionId,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let query = price_component
            .inner_join(plan_version_dsl::plan_version)
            .filter(plan_version_id.eq(plan_version_id_param))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id_param))
            .select(PriceComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {e:?}"))
            .attach("Error while fetching price components")
            .into_db_result()
    }

    pub async fn get_by_plan_version_ids(
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
        param_tenant_id: TenantId,
    ) -> DbResult<HashMap<PlanVersionId, Vec<PriceComponentRow>>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl::{plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let query = price_component
            .inner_join(pv_dsl::plan_version)
            .filter(plan_version_id.eq_any(plan_version_ids))
            .filter(pv_dsl::tenant_id.eq(param_tenant_id))
            .select(PriceComponentRow::as_select());

        crate::debug_sql(&query);

        let res: Vec<PriceComponentRow> = query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching price components: {e:?}"))
            .attach("Error while fetching price components")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.plan_version_id);

        Ok(grouped)
    }

    pub async fn update(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Option<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{id, plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(self.plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::update(price_component)
            .filter(id.eq(self.id))
            .filter(plan_version_id.eq_any(plan_version_with_id_in_tenant))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating price component: {e:?}"))
            .attach("Error while updating price component")
            .into_db_result()
    }

    /// The charge has no shape: its product is included, its features only. `update` leaves a
    /// `None` column as it is.
    pub async fn clear_fee_structure(
        conn: &mut PgConn,
        tenant_id: TenantId,
        component_id: PriceComponentId,
    ) -> DbResult<()> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl as pc;
        use diesel_async::RunQueryDsl;

        let in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));
        let query = diesel::update(pc::price_component)
            .filter(pc::id.eq(component_id))
            .filter(pc::plan_version_id.eq_any(in_tenant))
            .set(pc::fee_structure.eq(None::<serde_json::Value>));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while clearing a price component's structure")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn delete_by_id_and_tenant(
        conn: &mut PgConn,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::plan_version::dsl as plan_version_dsl;
        use crate::schema::price_component::dsl::{id, plan_version_id, price_component};
        use diesel_async::RunQueryDsl;

        // check the tenant (https://github.com/diesel-rs/diesel/issues/1478)
        let plan_version_with_id_in_tenant = plan_version_dsl::plan_version
            .select(plan_version_dsl::id)
            .filter(plan_version_dsl::id.eq(plan_version_id))
            .filter(plan_version_dsl::tenant_id.eq(tenant_id));

        let query = diesel::delete(price_component)
            .filter(id.eq(component_id))
            .filter(plan_version_id.eq_any(plan_version_with_id_in_tenant));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while deleting price component: {e:?}"))
            .attach("Error while deleting price component")
            .into_db_result()?;

        Ok(())
    }

    pub async fn list_all_by_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PriceComponentRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        let query = pc_dsl::price_component
            .inner_join(pv_dsl::plan_version)
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .select(PriceComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing all price components by tenant")
            .into_db_result()
    }

    pub async fn clone_all(
        conn: &mut PgConn,
        src_plan_version_id: PlanVersionId,
        dst_plan_version_id: PlanVersionId,
    ) -> DbResult<usize> {
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        diesel::define_sql_function! {
            fn gen_random_uuid() -> Uuid;
        }

        let query = pc_dsl::price_component
            .filter(pc_dsl::plan_version_id.eq(src_plan_version_id))
            .select((
                gen_random_uuid(),
                pc_dsl::name,
                pc_dsl::legacy_fee,
                dst_plan_version_id.into_sql::<diesel::sql_types::Uuid>(),
                pc_dsl::product_id,
                pc_dsl::billable_metric_id,
                pc_dsl::fee_structure,
            ))
            .insert_into(pc_dsl::price_component)
            .into_columns((
                pc_dsl::id,
                pc_dsl::name,
                pc_dsl::legacy_fee,
                pc_dsl::plan_version_id,
                pc_dsl::product_id,
                pc_dsl::billable_metric_id,
                pc_dsl::fee_structure,
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while cloning price components")
            .into_db_result()
    }
}

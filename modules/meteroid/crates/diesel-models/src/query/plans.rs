use crate::errors::IntoDbResult;
use crate::plan_versions::PlanVersionFilter;
use crate::plans::{
    PlanFilters, PlanRow, PlanRowForSubscription, PlanRowNew, PlanRowOverview, PlanRowPatch,
    PlanWithVersionRow,
};

use crate::{DbResult, PgConn};

use crate::enums::{PlanStatusEnum, PlanTypeEnum};
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};

use common_domain::ids::{PlanId, ProductFamilyId, TenantId};
use diesel::NullableExpressionMethods;
use diesel::{
    alias, debug_query, BoolExpressionMethods, ExpressionMethods, JoinOnDsl,
    PgTextExpressionMethods, QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use uuid::Uuid;

impl PlanRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan")
            .into_db_result()
    }
}

impl PlanRow {
    pub async fn activate(conn: &mut PgConn, id: PlanId, tenant_id: TenantId) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::status.eq(PlanStatusEnum::Active),
                p_dsl::updated_at.eq(diesel::dsl::now),
            ))
            .returning(PlanRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while activating plan")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, id: PlanId, tenant_id: TenantId) -> DbResult<usize> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(diesel::dsl::not(diesel::dsl::exists(
                pv_dsl::plan_version
                    .filter(pv_dsl::plan_id.eq(id))
                    .filter(pv_dsl::tenant_id.eq(tenant_id))
                    .select(diesel::dsl::sql::<diesel::sql_types::Integer>("1")),
            )));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting plan")
            .into_db_result()
    }

    pub async fn get_with_version(
        conn: &mut PgConn,
        version_id: Uuid,
        tenant_id: TenantId,
    ) -> DbResult<PlanWithVersionRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .filter(pv_dsl::id.eq(version_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanWithVersionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan with version")
            .into_db_result()
    }

    pub async fn get_overview_by_id(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
    ) -> DbResult<PlanRowOverview> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let (active_version_alias, draft_version_alias) = alias!(
            plan_version as active_version_alias,
            plan_version as draft_version_alias
        );

        let query = p_dsl::plan
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::id.eq(id))
            .left_join(
                active_version_alias.on(active_version_alias
                    .field(plan_version::id)
                    .nullable()
                    .eq(p_dsl::active_version_id)),
            )
            .left_join(
                draft_version_alias.on(draft_version_alias
                    .field(plan_version::id)
                    .nullable()
                    .eq(p_dsl::draft_version_id)),
            )
            .select((
                p_dsl::id,
                p_dsl::name,
                p_dsl::description,
                p_dsl::created_at,
                p_dsl::plan_type,
                p_dsl::status,
                pf_dsl::name,
                pf_dsl::id,
                active_version_alias
                    .fields((pv_dsl::id, pv_dsl::version, pv_dsl::trial_duration_days))
                    .nullable(),
                draft_version_alias.field(pv_dsl::id).nullable(),
                diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>>("null"),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while getting plan with version")
            .into_db_result()
    }

    pub async fn get_with_version_by_id(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> DbResult<PlanWithVersionRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = p_dsl::plan
            .left_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        match version_filter {
            PlanVersionFilter::Draft => {
                query = query
                    .filter(p_dsl::draft_version_id.is_not_null())
                    .filter(pv_dsl::id.nullable().eq(p_dsl::draft_version_id));
            }
            PlanVersionFilter::Active => {
                query = query
                    .filter(p_dsl::active_version_id.is_not_null())
                    .filter(pv_dsl::id.nullable().eq(p_dsl::active_version_id));
            }
            PlanVersionFilter::Version(v) => {
                query = query.filter(pv_dsl::version.eq(v));
            }
        }

        // Finalize the query
        let query = query
            .order(pv_dsl::version.desc())
            .select(PlanWithVersionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan with version by local_id")
            .into_db_result()
    }
}

impl PlanRowOverview {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<PlanRowOverview>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel::dsl::today;

        let (active_version_alias, draft_version_alias) = alias!(
            plan_version as active_version_alias,
            plan_version as draft_version_alias
        );

        let active_subscriptions_count_subselect = s_dsl::subscription
            .inner_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .filter(pv_dsl::plan_id.eq(p_dsl::id))
            .filter(s_dsl::start_date.le(today))
            .filter(
                s_dsl::end_date
                    .is_null()
                    .or(s_dsl::end_date.nullable().ge(today)),
            )
            .count()
            .single_value(); // single_value transforms the query in subquery

        let mut query = p_dsl::plan
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .left_join(
                active_version_alias.on(active_version_alias
                    .field(plan_version::id)
                    .nullable()
                    .eq(p_dsl::active_version_id)),
            )
            .left_join(
                draft_version_alias.on(draft_version_alias
                    .field(plan_version::id)
                    .nullable()
                    .eq(p_dsl::draft_version_id)),
            )
            .select((
                p_dsl::id,
                p_dsl::name,
                p_dsl::description,
                p_dsl::created_at,
                p_dsl::plan_type,
                p_dsl::status,
                pf_dsl::name,
                pf_dsl::id,
                active_version_alias
                    .fields((pv_dsl::id, pv_dsl::version, pv_dsl::trial_duration_days))
                    .nullable(),
                draft_version_alias.field(pv_dsl::id).nullable(),
                active_subscriptions_count_subselect,
            ))
            .into_boxed();

        if let Some(product_family_id) = product_family_id {
            query = query.filter(pf_dsl::id.eq(product_family_id))
        }

        if !filters.filter_status.is_empty() {
            query = query.filter(p_dsl::status.eq_any(filters.filter_status));
        }

        if !filters.filter_type.is_empty() {
            query = query.filter(p_dsl::plan_type.eq_any(filters.filter_type));
        }

        if let Some(search) = filters.search.filter(|s| !s.is_empty()) {
            query = query.filter(p_dsl::name.ilike(format!("%{}%", search)));
        }

        match order_by {
            OrderByRequest::NameAsc => query = query.order(p_dsl::name.asc()),
            OrderByRequest::NameDesc => query = query.order(p_dsl::name.desc()),
            OrderByRequest::DateAsc => query = query.order(p_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(p_dsl::created_at.desc()),
            _ => query = query.order(p_dsl::created_at.desc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while listing plans")
            .into_db_result()
    }
}

impl PlanRowPatch {
    pub async fn update(&self, conn: &mut PgConn) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(self.id))
            .filter(p_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating plan")
            .into_db_result()
    }
}
impl PlanRowForSubscription {
    pub async fn get_plans_for_subscription_by_version_ids(
        conn: &mut PgConn,
        version_ids: Vec<Uuid>,
    ) -> DbResult<Vec<PlanRowForSubscription>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pv_dsl::id.eq_any(version_ids))
            .select((
                pv_dsl::id,
                pv_dsl::net_terms,
                p_dsl::name,
                pv_dsl::currency,
                p_dsl::plan_type,
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .load(conn)
            .await
            .attach_printable("Error while getting plan names by version ids")
            .into_db_result()
            .map(|rows: Vec<(Uuid, i32, String, String, PlanTypeEnum)>| {
                rows.into_iter()
                    .map(|(version_id, net_terms, name, currency, plan_type)| {
                        PlanRowForSubscription {
                            version_id,
                            net_terms,
                            name,
                            currency,
                            plan_type,
                        }
                    })
                    .collect()
            })
    }
}

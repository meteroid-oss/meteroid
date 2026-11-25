use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersionFilter, PlanVersionRow};
use crate::plans::{
    _FullPlanRowEmbed, FullPlanRow, PlanFilters, PlanMrrRow, PlanRow, PlanRowForSubscription,
    PlanRowNew, PlanRowOverview, PlanRowPatch, PlanWithVersionRow, SelfServicePlanRow,
};

use crate::{DbResult, PgConn};

use crate::enums::{PlanStatusEnum, PlanTypeEnum};
use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};

use crate::price_components::PriceComponentRow;
use common_domain::ids::{PlanId, PlanVersionId, PriceId, ProductFamilyId, ProductId, TenantId};
use diesel::NullableExpressionMethods;
use diesel::{
    BelongingToDsl, BoolExpressionMethods, ExpressionMethods, JoinOnDsl, PgSortExpressionMethods,
    PgTextExpressionMethods, QueryDsl, SelectableHelper, alias,
};
use error_stack::ResultExt;

impl PlanRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl::plan;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting plan")
            .into_db_result()
    }
}

impl PlanRow {
    /// Batch-load plan rows by id, scoped to a tenant. Used to resolve origin names.
    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[PlanId],
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let query = p_dsl::plan
            .filter(p_dsl::id.eq_any(ids))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing plans by ids")
            .into_db_result()
    }

    /// Locked, so two ladder rewrites of the same product family run one after the other.
    pub async fn lock_by_product_family(
        conn: &mut PgConn,
        product_family_id: ProductFamilyId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::product_family_id.eq(product_family_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .order(p_dsl::id.asc())
            .select(PlanRow::as_select())
            .for_update();

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while locking the plans of a product family")
            .into_db_result()
    }

    /// Ranks `ranked` 1..n and clears the rank of every other plan of the product family.
    pub async fn set_self_service_ranks(
        conn: &mut PgConn,
        product_family_id: ProductFamilyId,
        tenant_id: TenantId,
        ranked: &[PlanId],
    ) -> DbResult<()> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_clock::now().naive_utc();

        let clear = diesel::update(p_dsl::plan)
            .filter(p_dsl::product_family_id.eq(product_family_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::self_service_rank.is_not_null())
            .filter(diesel::dsl::not(p_dsl::id.eq_any(ranked)))
            .set((
                p_dsl::self_service_rank.eq(None::<i32>),
                p_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&clear);

        clear
            .execute(conn)
            .await
            .attach("Error while clearing self-service ranks")
            .into_db_result()?;

        for (index, plan_id) in ranked.iter().enumerate() {
            let rank = index as i32 + 1;
            let query = diesel::update(p_dsl::plan)
                .filter(p_dsl::id.eq(*plan_id))
                .filter(p_dsl::product_family_id.eq(product_family_id))
                .filter(p_dsl::tenant_id.eq(tenant_id))
                .set((
                    p_dsl::self_service_rank.eq(Some(rank)),
                    p_dsl::updated_at.eq(now),
                ));

            crate::debug_sql(&query);

            query
                .execute(conn)
                .await
                .attach("Error while setting a self-service rank")
                .into_db_result()?;
        }

        Ok(())
    }

    /// The product family's ladder, its pricing page, in rank order: ranked, live, published.
    /// Sales-led plans included.
    pub async fn list_ladder(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> DbResult<Vec<PlanRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::product_family_id.eq(product_family_id))
            .filter(p_dsl::self_service_rank.is_not_null())
            .filter(p_dsl::active_version_id.is_not_null())
            .filter(p_dsl::archived_at.is_null())
            .filter(p_dsl::status.ne(PlanStatusEnum::Archived))
            // Tied ranks (older data, or set through the API) read in a stable order.
            .order((
                p_dsl::self_service_rank.asc(),
                p_dsl::created_at.asc(),
                p_dsl::id.asc(),
            ))
            .select(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing the self-serve ladder")
            .into_db_result()
    }

    /// Locked, so two writes of the plan's draft run one after the other.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanRow::as_select())
            .for_update();

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while locking plan")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding plan")
            .into_db_result()
    }

    /// Unmarks every plan of the product family but `except` as popular.
    pub async fn clear_pricing_popular(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        except: PlanId,
    ) -> DbResult<()> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::product_family_id.eq(product_family_id))
            .filter(p_dsl::id.ne(except))
            .filter(p_dsl::pricing_popular.eq(true))
            .set(p_dsl::pricing_popular.eq(false));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing the popular plan")
            .into_db_result()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_pricing_page(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
        tagline: Option<String>,
        popular: bool,
        cta_label: Option<String>,
        highlights: serde_json::Value,
        sales_led: bool,
        hide_price: bool,
        contact_url: Option<String>,
        product_order: Vec<ProductId>,
        hidden_product_ids: Vec<ProductId>,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::pricing_tagline.eq(tagline),
                p_dsl::pricing_popular.eq(popular),
                p_dsl::pricing_cta_label.eq(cta_label),
                p_dsl::pricing_highlights.eq(highlights),
                p_dsl::pricing_sales_led.eq(sales_led),
                p_dsl::pricing_hide_price.eq(hide_price),
                p_dsl::pricing_contact_url.eq(contact_url),
                p_dsl::pricing_product_order
                    .eq(product_order.into_iter().map(Some).collect::<Vec<_>>()),
                p_dsl::pricing_hidden_product_ids
                    .eq(hidden_product_ids.into_iter().map(Some).collect::<Vec<_>>()),
                p_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ))
            .returning(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating a plan's pricing page")
            .into_db_result()
    }

    pub async fn activate(conn: &mut PgConn, id: PlanId, tenant_id: TenantId) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::status.eq(PlanStatusEnum::Active),
                p_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ))
            .returning(PlanRow::as_select());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while activating plan")
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

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan")
            .into_db_result()
    }

    pub async fn get_with_version(
        conn: &mut PgConn,
        version_id: PlanVersionId,
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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting plan with version")
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
        use crate::schema::subscription::dsl as s_dsl;
        use diesel::dsl::today;
        use diesel_async::RunQueryDsl;

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
            .single_value();

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
                active_subscriptions_count_subselect,
                p_dsl::self_service_rank,
                p_dsl::issues_license,
                p_dsl::eligibility,
                p_dsl::pricing_sales_led,
                p_dsl::pricing_hide_price,
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while getting plan with version")
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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting plan with version by local_id")
            .into_db_result()
    }

    pub async fn get_with_version_by_version_id(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<PlanWithVersionRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .left_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .filter(pv_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanWithVersionRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting plan with version by local_id")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        param_plan_id: PlanId,
        param_tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::plan::dsl::{
            archived_at, id, plan, self_service_rank, status, tenant_id, updated_at,
        };

        use diesel_async::RunQueryDsl;

        // An archived plan leaves the self-serve ladder; unarchiving does not put it back.
        let query = diesel::update(plan)
            .filter(id.eq(param_plan_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set((
                status.eq(PlanStatusEnum::Archived),
                self_service_rank.eq(None::<i32>),
                archived_at.eq(Some(common_clock::now().naive_utc())),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while archiving plan")
            .into_db_result()?;

        Ok(())
    }

    pub async fn unarchive(
        conn: &mut PgConn,
        param_plan_id: PlanId,
        param_tenant_id: TenantId,
    ) -> DbResult<()> {
        use crate::schema::plan::dsl::{archived_at, id, plan, status, tenant_id, updated_at};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(plan)
            .filter(id.eq(param_plan_id))
            .filter(tenant_id.eq(param_tenant_id))
            .set((
                status.eq(PlanStatusEnum::Active),
                archived_at.eq::<Option<chrono::NaiveDateTime>>(None),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while unarchiving plan")
            .into_db_result()?;

        Ok(())
    }

    /// Returns (plan_name, version) pairs for all plans linked to the given price_ids
    /// through plan_component_price -> price_component -> plan_version -> plan.
    pub async fn list_plan_info_by_price_ids(
        conn: &mut PgConn,
        price_ids: &[PriceId],
    ) -> DbResult<Vec<(String, i32)>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        if price_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = pcp_dsl::plan_component_price
            .inner_join(pc_dsl::price_component.on(pcp_dsl::plan_component_id.eq(pc_dsl::id)))
            .inner_join(pv_dsl::plan_version.on(pc_dsl::plan_version_id.eq(pv_dsl::id)))
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pcp_dsl::price_id.eq_any(price_ids))
            .select((p_dsl::name, pv_dsl::version))
            .distinct()
            .order((p_dsl::name.asc(), pv_dsl::version.asc()));

        crate::debug_sql(&query);

        query
            .load::<(String, i32)>(conn)
            .await
            .attach("Error while listing plan info by price ids")
            .into_db_result()
    }
}

impl FullPlanRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: Option<&str>,
    ) -> DbResult<PaginatedVec<FullPlanRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(p_dsl::active_version_id.eq(pv_dsl::id.nullable())))
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(_FullPlanRowEmbed::as_select())
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

        if let Some(currency) = filters.filter_currency {
            query = query.filter(pv_dsl::currency.eq(currency));
        }

        let order = OrderByParam::parse(order_by, "created_at.desc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => {
                query = query.order((p_dsl::name.asc(), p_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((p_dsl::name.desc(), p_dsl::id.desc()))
            }
            ("status", OrderDirection::Asc) => {
                query = query.order((p_dsl::status.asc(), p_dsl::id.asc()))
            }
            ("status", OrderDirection::Desc) => {
                query = query.order((p_dsl::status.desc(), p_dsl::id.desc()))
            }
            ("plan_type", OrderDirection::Asc) => {
                query = query.order((p_dsl::plan_type.asc(), p_dsl::id.asc()))
            }
            ("plan_type", OrderDirection::Desc) => {
                query = query.order((p_dsl::plan_type.desc(), p_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((p_dsl::created_at.asc(), p_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc()))
            }
            _ => query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        let paginated_rows: PaginatedVec<_FullPlanRowEmbed> = paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing full plans")
            .into_db_result()?;

        let versions: Vec<&PlanVersionRow> = paginated_rows
            .items
            .iter()
            .map(|row| &row.version)
            .collect();

        let price_components: Vec<PriceComponentRow> = PriceComponentRow::belonging_to(&versions)
            .select(PriceComponentRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing price components for plans")
            .into_db_result()?;

        let full_rows: Vec<FullPlanRow> = paginated_rows
            .items
            .into_iter()
            .map(|base| {
                let pcs: Vec<PriceComponentRow> = price_components
                    .iter()
                    .filter(|pc| pc.plan_version_id == base.version.id)
                    .cloned()
                    .collect();
                FullPlanRow {
                    plan: base.plan,
                    version: base.version,
                    product_family: base.product_family,
                    price_components: pcs,
                }
            })
            .collect();

        Ok(PaginatedVec {
            total_results: paginated_rows.total_results,
            total_pages: paginated_rows.total_pages,
            items: full_rows,
        })
    }

    pub async fn get_by_id(
        conn: &mut PgConn,
        id: PlanId,
        tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> DbResult<FullPlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
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

        crate::debug_sql(&query);

        let base: _FullPlanRowEmbed = query
            .select(_FullPlanRowEmbed::as_select())
            .first(conn)
            .await
            .attach("Error while getting full plan by id")
            .into_db_result()?;

        let price_components: Vec<PriceComponentRow> =
            PriceComponentRow::belonging_to(&base.version)
                .select(PriceComponentRow::as_select())
                .load(conn)
                .await
                .attach("Error while listing price components for plan")
                .into_db_result()?;

        Ok(FullPlanRow {
            plan: base.plan,
            version: base.version,
            product_family: base.product_family,
            price_components,
        })
    }
}

impl PlanRowOverview {
    /// The tenant's plans after `after` in id order, at most `limit` (API v2 keyset pages), read
    /// as `list` reads them; of `status` when given, else every plan but the archived ones.
    pub async fn list_page(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        status: Option<PlanStatusEnum>,
        after: Option<PlanId>,
        limit: i64,
    ) -> DbResult<Vec<PlanRowOverview>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel::dsl::today;
        use diesel_async::RunQueryDsl;

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
            .single_value();

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
                p_dsl::self_service_rank,
                p_dsl::issues_license,
                p_dsl::eligibility,
                p_dsl::pricing_sales_led,
                p_dsl::pricing_hide_price,
            ))
            .into_boxed();

        if let Some(product_family_id) = product_family_id {
            query = query.filter(p_dsl::product_family_id.eq(product_family_id));
        }
        // Archived plans are left out unless asked for, as every v2 list leaves archived out.
        query = match status {
            Some(status) => query.filter(p_dsl::status.eq(status)),
            None => query.filter(p_dsl::status.ne(PlanStatusEnum::Archived)),
        };
        if let Some(after) = after {
            query = query.filter(p_dsl::id.gt(after));
        }
        let query = query.order(p_dsl::id.asc()).limit(limit);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching a page of plans")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: Option<&str>,
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
                p_dsl::self_service_rank,
                p_dsl::issues_license,
                p_dsl::eligibility,
                p_dsl::pricing_sales_led,
                p_dsl::pricing_hide_price,
            ))
            .into_boxed();

        if let Some(product_family_id) = product_family_id {
            query = query.filter(pf_dsl::id.eq(product_family_id));
        }

        if !filters.filter_status.is_empty() {
            query = query.filter(p_dsl::status.eq_any(filters.filter_status));
        }

        if !filters.filter_type.is_empty() {
            query = query.filter(p_dsl::plan_type.eq_any(filters.filter_type));
        }

        if let Some(search) = filters.search.filter(|s| !s.is_empty()) {
            query = query.filter(p_dsl::name.ilike(format!("%{search}%")));
        }

        if let Some(currency) = filters.filter_currency {
            query = query.filter(
                active_version_alias
                    .field(plan_version::currency)
                    .eq(currency),
            );
        }

        // The self-serve ladder first, by rank; the other plans after it, newest first.
        let order = OrderByParam::parse(order_by, "self_service_rank.asc");

        match (order.column.as_str(), order.direction) {
            ("self_service_rank", OrderDirection::Asc) => {
                query = query.order((
                    p_dsl::self_service_rank.asc().nulls_last(),
                    p_dsl::created_at.desc(),
                    p_dsl::id.asc(),
                ))
            }
            ("self_service_rank", OrderDirection::Desc) => {
                query = query.order((
                    p_dsl::self_service_rank.desc().nulls_last(),
                    p_dsl::created_at.desc(),
                    p_dsl::id.asc(),
                ))
            }
            ("name", OrderDirection::Asc) => {
                query = query.order((p_dsl::name.asc(), p_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((p_dsl::name.desc(), p_dsl::id.desc()))
            }
            ("status", OrderDirection::Asc) => {
                query = query.order((p_dsl::status.asc(), p_dsl::id.asc()))
            }
            ("status", OrderDirection::Desc) => {
                query = query.order((p_dsl::status.desc(), p_dsl::id.desc()))
            }
            ("plan_type", OrderDirection::Asc) => {
                query = query.order((p_dsl::plan_type.asc(), p_dsl::id.asc()))
            }
            ("plan_type", OrderDirection::Desc) => {
                query = query.order((p_dsl::plan_type.desc(), p_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((p_dsl::created_at.asc(), p_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc()))
            }
            _ => query = query.order((p_dsl::created_at.desc(), p_dsl::id.desc())),
        }

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing plans")
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

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating plan")
            .into_db_result()
    }
}
impl PlanRowForSubscription {
    #[allow(clippy::type_complexity)]
    pub async fn get_plans_for_subscription_by_version_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        version_ids: Vec<PlanVersionId>,
    ) -> DbResult<Vec<PlanRowForSubscription>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pv_dsl::id.eq_any(version_ids))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select((
                pv_dsl::id,
                pv_dsl::net_terms,
                p_dsl::name,
                pv_dsl::currency,
                p_dsl::plan_type,
                pv_dsl::trial_duration_days,
                pv_dsl::trial_is_free,
                p_dsl::product_family_id,
                pv_dsl::tax_inclusive,
            ));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while getting plan names by version ids")
            .into_db_result()
            .map(
                |rows: Vec<(
                    PlanVersionId,
                    i32,
                    String,
                    String,
                    PlanTypeEnum,
                    Option<i32>,
                    bool,
                    ProductFamilyId,
                    bool,
                )>| {
                    rows.into_iter()
                        .map(
                            |(
                                version_id,
                                net_terms,
                                name,
                                currency,
                                plan_type,
                                trial_duration_days,
                                trial_is_free,
                                product_family_id,
                                tax_inclusive,
                            )| {
                                PlanRowForSubscription {
                                    version_id,
                                    net_terms,
                                    name,
                                    currency,
                                    plan_type,
                                    trial_duration_days,
                                    trial_is_free,
                                    product_family_id,
                                    tax_inclusive,
                                }
                            },
                        )
                        .collect()
                },
            )
    }
}

impl SelfServicePlanRow {
    pub async fn list_self_service_plans(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        currency: &str,
        exclude_plan_id: PlanId,
        with_sales_led: bool,
    ) -> DbResult<Vec<SelfServicePlanRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::price::dsl as pr_dsl;
        use crate::schema::price_component::dsl as pc_dsl;
        use diesel_async::RunQueryDsl;

        // Versions with a price in the currency.
        let offering: Vec<PlanVersionId> = pcp_dsl::plan_component_price
            .inner_join(pc_dsl::price_component.on(pc_dsl::id.eq(pcp_dsl::plan_component_id)))
            .inner_join(pr_dsl::price.on(pr_dsl::id.eq(pcp_dsl::price_id)))
            .filter(pr_dsl::tenant_id.eq(tenant_id))
            .filter(pr_dsl::currency.eq(currency))
            .filter(pr_dsl::archived_at.is_null())
            .select(pc_dsl::plan_version_id)
            .distinct()
            .load(conn)
            .await
            .attach("Error while listing the versions offered in a currency")
            .into_db_result()?;

        let query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(pv_dsl::id.nullable().eq(p_dsl::active_version_id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::product_family_id.eq(product_family_id))
            .filter(p_dsl::status.eq(PlanStatusEnum::Active))
            .filter(p_dsl::plan_type.ne(PlanTypeEnum::Custom))
            .filter(p_dsl::self_service_rank.is_not_null())
            // A sales-led plan is on the pricing page only: a customer never moves to it alone.
            .filter(
                p_dsl::pricing_sales_led
                    .eq(false)
                    .or(diesel::IntoSql::into_sql::<diesel::sql_types::Bool>(
                        with_sales_led,
                    )),
            )
            .filter(p_dsl::id.ne(exclude_plan_id))
            .filter(
                pv_dsl::currency
                    .eq(currency)
                    .or(pv_dsl::id.eq_any(offering)),
            )
            .order((
                p_dsl::self_service_rank.asc(),
                p_dsl::created_at.asc(),
                p_dsl::id.asc(),
            ))
            .select((
                p_dsl::id,
                p_dsl::name,
                p_dsl::description,
                pv_dsl::id,
                p_dsl::self_service_rank.assume_not_null(),
            ));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing self-service plans")
            .into_db_result()
    }
}

impl PlanMrrRow {
    /// The MRR of each plan's active subscriptions, whatever their version, per currency.
    pub async fn list_by_plan_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_ids: &[PlanId],
    ) -> DbResult<Vec<PlanMrrRow>> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        if plan_ids.is_empty() {
            return Ok(vec![]);
        }

        diesel::sql_query(
            r#"
            SELECT pv.plan_id, s.currency, SUM(s.mrr_cents)::bigint AS mrr_cents
            FROM subscription s
            JOIN plan_version pv ON pv.id = s.plan_version_id
            WHERE s.tenant_id = $1
              AND pv.plan_id = ANY($2)
              AND s.status = 'ACTIVE'
            GROUP BY pv.plan_id, s.currency
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids)
        .get_results(conn)
        .await
        .attach("Error while reading plans MRR")
        .into_db_result()
    }
}

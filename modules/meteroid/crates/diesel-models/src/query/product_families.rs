use crate::errors::IntoDbResult;
use crate::product_families::{ProductFamilyRow, ProductFamilyRowNew};

use crate::{DbResult, PgConn};

use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{ProductFamilyId, TenantId};
use diesel::{ExpressionMethods, PgTextExpressionMethods, QueryDsl, debug_query};
use error_stack::ResultExt;

impl ProductFamilyRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProductFamilyRow> {
        use crate::schema::product_family::dsl::product_family;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product_family).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting product family")
            .into_db_result()
    }
}

impl ProductFamilyRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<&str>,
        param_query: Option<String>,
    ) -> DbResult<PaginatedVec<ProductFamilyRow>> {
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = pf_dsl::product_family
            .filter(pf_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(param_query) = param_query {
            query = query.filter(pf_dsl::name.ilike(format!("%{param_query}%")));
        }

        let order = OrderByParam::parse(order_by, "name.asc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => {
                query = query.order((pf_dsl::name.asc(), pf_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((pf_dsl::name.desc(), pf_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((pf_dsl::created_at.asc(), pf_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((pf_dsl::created_at.desc(), pf_dsl::id.desc()))
            }
            _ => query = query.order((pf_dsl::name.asc(), pf_dsl::id.asc())),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while fetching product families")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        id: ProductFamilyId,
        tenant_id: TenantId,
    ) -> DbResult<ProductFamilyRow> {
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let query = pf_dsl::product_family
            .filter(pf_dsl::id.eq(id))
            .filter(pf_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding product family by id and tenant_id")
            .into_db_result()
    }
}

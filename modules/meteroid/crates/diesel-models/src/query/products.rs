use crate::errors::IntoDbResult;
use crate::products::{ProductRow, ProductRowNew};

use crate::{DbResult, PgConn};

use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{ProductFamilyId, ProductId, TenantId};
use diesel::{
    ExpressionMethods, JoinOnDsl, PgTextExpressionMethods, QueryDsl, SelectableHelper, debug_query,
};
use error_stack::ResultExt;

impl ProductRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProductRow> {
        use crate::schema::product::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting product")
            .into_db_result()
    }
}

impl ProductRow {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: ProductId,
        tenant_id: TenantId,
    ) -> DbResult<ProductRow> {
        use crate::schema::product::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::product
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding product by id and tenant id")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::product
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(family_id) = family_id {
            query = query.filter(pf_dsl::id.eq(family_id))
        }

        let mut query = query.select(ProductRow::as_select());

        match order_by {
            OrderByRequest::IdAsc => query = query.order(p_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(p_dsl::id.desc()),
            OrderByRequest::NameAsc => query = query.order(p_dsl::name.asc()),
            OrderByRequest::NameDesc => query = query.order(p_dsl::name.desc()),
            OrderByRequest::DateAsc => query = query.order(p_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(p_dsl::created_at.desc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching products")
            .into_db_result()
    }

    pub async fn search(
        conn: &mut PgConn,
        tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<ProductRow>> {
        use crate::schema::product::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::product
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::name.ilike(format!("%{}%", query)))
            .into_boxed();

        if let Some(family_id) = family_id {
            query = query.filter(pf_dsl::id.eq(family_id))
        }

        let mut query = query.select(ProductRow::as_select());

        match order_by {
            OrderByRequest::IdAsc => query = query.order(p_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(p_dsl::id.desc()),
            OrderByRequest::NameAsc => query = query.order(p_dsl::name.asc()),
            OrderByRequest::NameDesc => query = query.order(p_dsl::name.desc()),
            OrderByRequest::DateAsc => query = query.order(p_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(p_dsl::created_at.desc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching products")
            .into_db_result()
    }
}

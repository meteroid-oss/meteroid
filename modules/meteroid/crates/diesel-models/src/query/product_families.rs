use crate::errors::IntoDbResult;
use crate::product_families::{ProductFamilyRow, ProductFamilyRowNew};

use crate::{DbResult, PgConn};

use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use diesel::{debug_query, ExpressionMethods, PgTextExpressionMethods, QueryDsl};
use error_stack::ResultExt;
use uuid::Uuid;

impl ProductFamilyRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProductFamilyRow> {
        use crate::schema::product_family::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(product_family).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting product family")
            .into_db_result()
    }
}

impl ProductFamilyRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        param_query: Option<String>,
    ) -> DbResult<PaginatedVec<ProductFamilyRow>> {
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = pf_dsl::product_family
            .filter(pf_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(param_query) = param_query {
            query = query.filter(pf_dsl::name.ilike(format!("%{}%", param_query)));
        }

        match order_by {
            OrderByRequest::IdAsc => query = query.order(pf_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(pf_dsl::id.desc()),
            OrderByRequest::DateAsc => query = query.order(pf_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(pf_dsl::created_at.desc()),
            _ => query = query.order(pf_dsl::id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching customers")
            .into_db_result()
    }

    pub async fn find_by_local_id_and_tenant_id(
        conn: &mut PgConn,
        local_id: &str,
        tenant_id: Uuid,
    ) -> DbResult<ProductFamilyRow> {
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let query = pf_dsl::product_family
            .filter(pf_dsl::local_id.eq(local_id))
            .filter(pf_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding product family by local_id and tenant_id")
            .into_db_result()
    }
}

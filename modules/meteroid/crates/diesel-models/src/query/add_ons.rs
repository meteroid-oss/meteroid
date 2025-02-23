use crate::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{
    debug_query, BoolExpressionMethods, ExpressionMethods, PgTextExpressionMethods, QueryDsl,
    SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use tap::TapFallible;

impl AddOnRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::insert_into(ao_dsl::add_on).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting add-on")
            .into_db_result()
    }
}

impl AddOnRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: uuid::Uuid,
    ) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on
            .filter(ao_dsl::id.eq(id))
            .filter(ao_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting add-on")
            .into_db_result()
    }

    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
    ) -> DbResult<PaginatedVec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let mut query = ao_dsl::add_on
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(search) = search {
            query = query.filter(
                ao_dsl::name
                    .ilike(format!("%{}%", search))
                    .or(ao_dsl::local_id.ilike(format!("%{}%", search))),
            );
        }

        let query = query.select(AddOnRow::as_select());

        let query = query.paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .load_and_count_pages(conn)
            .await
            .tap_err(|e| log::error!("Error while listing add-ons: {:?}", e))
            .attach_printable("Error while listing add-ons")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, id: uuid::Uuid, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::delete(ao_dsl::add_on)
            .filter(ao_dsl::id.eq(id))
            .filter(ao_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while deleting add-on: {:?}", e))
            .attach_printable("Error while deleting add-on")
            .into_db_result()?;

        Ok(())
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[uuid::Uuid],
        tenant_id: TenantId,
    ) -> DbResult<Vec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on
            .filter(ao_dsl::id.eq_any(ids))
            .filter(ao_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching add-ons: {:?}", e))
            .attach_printable("Error while fetching add-ons")
            .into_db_result()
    }
}

impl AddOnRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::update(ao_dsl::add_on)
            .filter(ao_dsl::id.eq(self.id))
            .filter(ao_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating add-on")
            .into_db_result()
    }
}

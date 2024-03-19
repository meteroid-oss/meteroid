use crate::errors::IntoDbResult;
use crate::tenants::{Tenant, TenantNew};
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl TenantNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Tenant> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(tenant).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .into_db_result()
            .attach_printable("Error while inserting tenant")
    }

    // if sync mode, we keep all repositories sync and distrib to async conn pool in service
    // pub fn insert<C>(&self, conn: &mut C) -> DbResult<Tenant>
    //     where
    //         C: diesel::Connection,
    // {
    //     use diesel::prelude::*;
    //
    //     let query = diesel::insert_into(tenant::table).values(self);
    //     query
    //         .get_result(conn)
    //         .into_db_result()
    //         .attach_printable("Error while inserting tenant")
    // }
}

impl Tenant {
    pub async fn find_by_id(conn: &mut PgConn, tenant_id: uuid::Uuid) -> DbResult<Tenant> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .into_db_result()
            .attach_printable("Error while finding tenant by id")
    }
}

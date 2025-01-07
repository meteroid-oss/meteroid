use crate::errors::IntoDbResult;
use crate::product_families::{ProductFamilyRow, ProductFamilyRowNew};

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, QueryDsl};
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
    pub async fn list(conn: &mut PgConn, tenant_id: Uuid) -> DbResult<Vec<ProductFamilyRow>> {
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let query = pf_dsl::product_family.filter(pf_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing product families")
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

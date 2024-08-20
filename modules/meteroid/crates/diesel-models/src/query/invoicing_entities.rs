use crate::errors::IntoDbResult;
use crate::invoicing_entities::{InvoicingEntityRow};

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use tap::TapFallible;

impl InvoicingEntityRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoicing_entity).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization")
            .into_db_result()
    }
    pub async fn list_by_tenant_id(conn: &mut PgConn, tenant_id: &uuid::Uuid) -> DbResult<Vec<InvoicingEntityRow>> {
        use diesel_async::RunQueryDsl;
        use crate::schema::invoicing_entity::dsl as dsl;

        let query = dsl::invoicing_entity
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching invoicing entities by tenant")
            .into_db_result()
    }
    
    pub async fn get_default_invoicing_entity_for_tenant(conn: &mut PgConn, tenant_id: &uuid::Uuid) -> DbResult<InvoicingEntityRow> {
        use diesel_async::RunQueryDsl;
        use crate::schema::invoicing_entity::dsl as dsl;

        let query = dsl::invoicing_entity
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::is_default.eq(true))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching default invoicing entity by tenant")
            .into_db_result()
    }
    
    pub async fn get_invoicing_entity_by_id_and_tenant_id(conn: &mut PgConn, id: &uuid::Uuid, tenant_id: &uuid::Uuid) -> DbResult<InvoicingEntityRow> {
        use diesel_async::RunQueryDsl;
        use crate::schema::invoicing_entity::dsl as dsl;

        let query = dsl::invoicing_entity
            .filter(dsl::id.eq(id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching invoicing entity by id and tenant")
            .into_db_result()
    }
}

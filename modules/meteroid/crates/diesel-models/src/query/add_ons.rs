use crate::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, QueryDsl};
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
            .tap_err(|e| log::error!("Error while inserting add-on: {:?}", e))
            .attach_printable("Error while inserting add-on")
            .into_db_result()
    }
}

impl AddOnRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
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
        tenant_id: uuid::Uuid,
    ) -> DbResult<Vec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on.filter(ao_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while listing add-ons: {:?}", e))
            .attach_printable("Error while listing add-ons")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, id: uuid::Uuid, tenant_id: uuid::Uuid) -> DbResult<()> {
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

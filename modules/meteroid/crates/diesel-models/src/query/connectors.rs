use crate::connectors::{ConnectorRow, ConnectorRowNew, ConnectorRowPatch};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use common_domain::ids::{ConnectorId, TenantId};
use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl ConnectorRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(connector).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting connector")
            .into_db_result()
    }
}

impl ConnectorRow {
    pub async fn delete_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(
            c_dsl::connector
                .filter(c_dsl::id.eq(id))
                .filter(c_dsl::tenant_id.eq(tenant_uid)),
        );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting connector")
            .into_db_result()
    }

    pub async fn get_connector_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding connector")
            .into_db_result()
    }

    pub async fn get_connector_by_alias(
        conn: &mut PgConn,
        connector_alias: String,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = connector
            .filter(alias.eq(connector_alias))
            .filter(tenant_id.eq(tenant_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding connector")
            .into_db_result()
    }

    pub async fn list_connectors(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        connector_type_filter: Option<crate::enums::ConnectorTypeEnum>,
        provider_filter: Option<crate::enums::ConnectorProviderEnum>,
    ) -> DbResult<Vec<ConnectorRow>> {
        use crate::schema::connector::dsl::*;
        use diesel_async::RunQueryDsl;

        let mut query = connector.filter(tenant_id.eq(tenant_uid)).into_boxed();

        if let Some(ct) = connector_type_filter {
            query = query.filter(connector_type.eq(ct));
        }

        if let Some(cp) = provider_filter {
            query = query.filter(provider.eq(cp));
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing connectors")
            .into_db_result()
    }
}

impl ConnectorRowPatch {
    pub async fn patch(&self, conn: &mut PgConn, tenant_id: TenantId) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::connector)
            .filter(c_dsl::id.eq(self.id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while patching connector")
            .into_db_result()
    }
}

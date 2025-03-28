use crate::connectors::{ConnectorRow, ConnectorRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use common_domain::ids::TenantId;
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
        connector_uid: uuid::Uuid,
        tenant_uid: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::connector::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(
            connector
                .filter(id.eq(connector_uid))
                .filter(tenant_id.eq(tenant_uid)),
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
        connector_uid: uuid::Uuid,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = connector
            .filter(id.eq(connector_uid))
            .filter(tenant_id.eq(tenant_uid));

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

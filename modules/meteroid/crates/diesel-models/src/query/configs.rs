use crate::configs::{InvoicingConfig, ProviderConfig, ProviderConfigNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl ProviderConfigNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProviderConfig> {
        use crate::schema::provider_config::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(provider_config).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting provider configuration")
            .into_db_result()
    }
}

impl InvoicingConfig {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoicingConfig> {
        use crate::schema::invoicing_config::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoicing_config).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting invoicing configuration")
            .into_db_result()
    }
}

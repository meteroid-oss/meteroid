use crate::configs::{InvoicingConfigRow, ProviderConfigRow, ProviderConfigRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use crate::enums::InvoicingProviderEnum;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{debug_query, DecoratableTarget};
use error_stack::ResultExt;

impl ProviderConfigRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ProviderConfigRow> {
        use crate::schema::provider_config::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(provider_config)
            .values(self)
            .on_conflict((tenant_id, invoicing_provider))
            .filter_target(enabled.eq(true))
            .do_update()
            .set((
                enabled.eq(self.enabled),
                webhook_security.eq(&self.webhook_security),
                api_security.eq(&self.api_security),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting provider configuration")
            .into_db_result()
    }
}

impl InvoicingConfigRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoicingConfigRow> {
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

impl ProviderConfigRow {
    pub async fn find_provider_config(
        conn: &mut PgConn,
        tenant_uid: uuid::Uuid,
        provider: InvoicingProviderEnum,
    ) -> DbResult<ProviderConfigRow> {
        use crate::schema::provider_config::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = provider_config
            .filter(tenant_id.eq(tenant_uid))
            .filter(invoicing_provider.eq(provider))
            .filter(enabled.eq(true));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding provider config")
            .into_db_result()
    }
}

use crate::errors::IntoDbResult;
use crate::quote_contract_values::QuoteContractValueRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{QuoteId, TenantId};
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteContractValueRow {
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::quote_contract_value::dsl as d;

        let query = diesel::insert_into(d::quote_contract_value)
            .values(self)
            .on_conflict(d::quote_id)
            .do_update()
            .set((
                d::value.eq(excluded(d::value)),
                d::computed_at.eq(excluded(d::computed_at)),
            ));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while storing a quote's contract value")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId, quote_id: QuoteId) -> DbResult<()> {
        use crate::schema::quote_contract_value::dsl as d;

        diesel::delete(d::quote_contract_value)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while deleting a quote's contract value")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Option<QuoteContractValueRow>> {
        use crate::schema::quote_contract_value::dsl as d;

        d::quote_contract_value
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .select(QuoteContractValueRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a quote's contract value")
            .into_db_result()
    }
}

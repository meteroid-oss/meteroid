use crate::errors::IntoDbResult;
use crate::invoices::{Invoice, InvoiceNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl InvoiceNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Invoice> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }
}
impl Invoice {
    pub async fn insert_invoice_batch(
        conn: &mut PgConn,
        invoices: Vec<InvoiceNew>,
    ) -> DbResult<Vec<Invoice>> {
        use crate::schema::invoice::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoice).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting invoice")
            .into_db_result()
    }
}

use crate::customers::{Customer, CustomerNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl CustomerNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<Customer> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting customer")
            .into_db_result()
    }
}

impl Customer {
    pub async fn insert_customer_batch(
        conn: &mut PgConn,
        batch: Vec<CustomerNew>,
    ) -> DbResult<Vec<Customer>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&batch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting customer batch")
            .into_db_result()
    }
}

use deadpool_postgres::{Object, Pool, Transaction};
use std::sync::Arc;
use tonic::Status;

#[tracing::instrument(skip(pool))]
pub async fn get_connection(pool: &Pool) -> Result<Object, Status> {
    match pool.get().await {
        Ok(client) => Ok(client),
        Err(e) => {
            log::error!("Unable to get database connection : {}", e);
            Err(Status::unavailable("Unable to get database connection"))
        }
    }
}

#[tracing::instrument(skip(conn))]
pub async fn get_transaction(conn: &mut Object) -> Result<Transaction, Status> {
    let transaction = conn.transaction().await.map_err(|e| {
        Status::internal("Unable to start transaction")
            .set_source(Arc::new(e))
            .clone()
    })?;
    Ok(transaction)
}

#[derive(Debug, Clone)]
pub struct DbService {
    pub pool: Pool,
}

impl DbService {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
    pub async fn get_connection(&self) -> Result<Object, Status> {
        get_connection(&self.pool).await
    }
    pub async fn get_transaction<'a>(
        &'a self,
        client: &'a mut Object,
    ) -> Result<Transaction<'a>, Status> {
        get_transaction(client).await
    }
}

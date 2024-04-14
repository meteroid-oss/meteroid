use crate::errors::StoreError;
use diesel_async::pooled_connection::deadpool::Object;
use diesel_async::pooled_connection::deadpool::Pool;

use crate::StoreResult;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::scoped_futures::{ScopedBoxFuture, ScopedFutureExt};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use error_stack::{Report, ResultExt};

pub type PgPool = Pool<AsyncPgConnection>;
pub type PgConn = Object<AsyncPgConnection>;

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

pub fn diesel_make_pg_pool(db_url: String) -> StoreResult<PgPool> {
    // let db_url = format!(
    //     "postgres://{}:{}@{}:{}/{}",
    //     database.username,
    //     database.password.peek(),
    //     database.host,
    //     database.port,
    //     database.dbname
    // );
    let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url);
    let builder = Pool::builder(manager);
    // .max_size(database.pool_size)
    // .min_idle(database.min_idle)
    // .queue_strategy(database.queue_strategy.into())
    // .connection_timeout(std::time::Duration::from_secs(database.connection_timeout))
    // .max_lifetime(database.max_lifetime.map(std::time::Duration::from_secs))

    builder
        .build()
        .map_err(Report::from)
        .change_context(StoreError::InitializationError)
        .attach_printable("Failed to create PostgreSQL connection pool")
}

impl Store {
    pub fn from_pool(pool: PgPool) -> Self {
        Store { pool }
    }

    pub fn new(database_url: String) -> StoreResult<Self> {
        let pool: PgPool = diesel_make_pg_pool(database_url)?;

        Ok(Store { pool })
    }

    pub async fn get_conn(&self) -> StoreResult<PgConn> {
        self.pool
            .get()
            .await
            .map_err(Report::from)
            .change_context(StoreError::DatabaseConnectionError)
            .attach_printable("Failed to get a connection from the pool")
    }
    pub(crate) async fn transaction<'a, R, F>(&self, callback: F) -> StoreResult<R>
    where
        F: for<'r> FnOnce(
                &'r mut PgConn,
            )
                -> ScopedBoxFuture<'a, 'r, error_stack::Result<R, StoreError>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, callback).await
    }

    pub(crate) async fn transaction_with<'a, R, F>(
        &self,
        conn: &mut PgConn,
        callback: F,
    ) -> StoreResult<R>
    where
        F: for<'r> FnOnce(
                &'r mut PgConn,
            )
                -> ScopedBoxFuture<'a, 'r, error_stack::Result<R, StoreError>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let result = conn
            .transaction(|conn| {
                async move {
                    let res = callback(conn);
                    res.await.map_err(StoreError::TransactionStoreError)
                }
                .scope_boxed()
            })
            .await?;

        Ok(result)
    }
}

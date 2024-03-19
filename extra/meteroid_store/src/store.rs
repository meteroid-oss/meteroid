use crate::errors::{StorageResult, StoreError};
use diesel_async::pooled_connection::deadpool::Object;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::pooled_connection::deadpool::PoolError;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncPgConnection;
use error_stack::{IntoReport, Report, ResultExt};

pub type PgPool = Pool<AsyncPgConnection>;
pub type PgConn = Object<AsyncPgConnection>;

pub struct Store {
    pub pool: PgPool,
}

pub async fn diesel_make_pg_pool(db_url: String) -> StorageResult<PgPool> {
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
    pub async fn new(database_url: String) -> StorageResult<Self> {
        let pool: PgPool = diesel_make_pg_pool(database_url).await?;

        Ok(Store { pool })
    }

    pub async fn get_conn(&self) -> StorageResult<PgConn> {
        self.pool
            .get()
            .await
            .map_err(Report::from)
            .change_context(StoreError::DatabaseConnectionError)
            .attach_printable("Failed to get a connection from the pool")
    }
}

use crate::errors::StoreError;
use diesel_async::pooled_connection::deadpool::Object;
use diesel_async::pooled_connection::deadpool::Pool;
use std::sync::Arc;

use crate::compute::clients::usage::UsageClient;
use crate::StoreResult;
use common_eventbus::{Event, EventBus};
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::scoped_futures::{ScopedBoxFuture, ScopedFutureExt};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use error_stack::{Report, ResultExt};

pub type PgPool = Pool<AsyncPgConnection>;
pub type PgConn = Object<AsyncPgConnection>;


#[derive(Clone)]
pub struct Settings {
    pub crypt_key: secrecy::SecretString,
    pub jwt_secret: secrecy::SecretString,
    pub multi_organization_enabled: bool,
}

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
    pub eventbus: Arc<dyn EventBus<Event>>,
    pub usage_client: Arc<dyn UsageClient>,
    pub settings: Settings,
}

pub fn diesel_make_pg_pool(db_url: String) -> StoreResult<PgPool> {
    let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url);
    let builder = Pool::builder(manager);

    builder
        .build()
        .map_err(Report::from)
        .change_context(StoreError::InitializationError)
        .attach_printable("Failed to create PostgreSQL connection pool")
}

impl Store {
    pub fn new(
        database_url: String,
        crypt_key: secrecy::SecretString,
        jwt_secret: secrecy::SecretString,
        multi_organization_enabled: bool,
        eventbus: Arc<dyn EventBus<Event>>,
        usage_client: Arc<dyn UsageClient>,
    ) -> StoreResult<Self> {
        let pool: PgPool = diesel_make_pg_pool(database_url)?;

        Ok(Store {
            pool,
            eventbus,
            usage_client,
            settings: Settings {
                crypt_key,
                jwt_secret,
                multi_organization_enabled,
            },
        })
    }

    pub async fn get_conn(&self) -> StoreResult<PgConn> {
        self.pool
            .get()
            .await
            .map_err(Report::from)
            .change_context(StoreError::DatabaseConnectionError)
            .attach_printable("Failed to get a connection from the pool")
    }

    // Temporary, evaluating if this simplifies the handling of store + diesel interations within a transaction

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

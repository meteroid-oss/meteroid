use crate::StoreResult;
use crate::errors::{StoreError, StoreErrorReport};
use crate::services::clients::usage::UsageClient;
use common_domain::ids::{OrganizationId, TenantId};
use common_eventbus::{Event, EventBus};
use diesel::{ConnectionError, ConnectionResult};
use diesel_async::pooled_connection::deadpool::{Object, Pool};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use envconfig::Envconfig;
use error_stack::{Report, ResultExt};
use futures::FutureExt;
use futures::future::BoxFuture;
use meteroid_mailer::service::MailerService;
use meteroid_oauth::service::OauthServices;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use scoped_futures::{ScopedBoxFuture, ScopedFutureExt};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio_postgres_rustls::MakeRustlsConnect;

pub type PgPool = Pool<AsyncPgConnection>;
pub type PgConn = Object<AsyncPgConnection>;

#[derive(Clone)]
pub struct Settings {
    pub crypt_key: secrecy::SecretString,
    pub jwt_secret: secrecy::SecretString,
    pub multi_organization_enabled: bool,
    pub public_url: String,
    /// External base URL of the REST API (e.g. `http://127.0.0.1:8080`). Used
    /// for callback/return URLs handed to third parties that must hit the
    /// backend REST endpoints directly rather than the frontend `public_url`
    /// (e.g. the GoCardless Billing Request Flow return URL).
    pub rest_api_external_url: String,
    /// Public base for inbound provider webhooks; `None` → `rest_api_external_url`.
    pub webhook_external_url: Option<String>,
    pub mailer_enabled: bool,
    pub domains_whitelist: Vec<String>,
    pub admin_organization: Option<OrganizationId>,
    pub invite_ttl_days: u32,
    pub mfa_issuer: String,
}

impl Settings {
    /// Base URL for provider webhook callbacks (Mollie per-payment webhook URLs).
    pub fn webhook_base_url(&self) -> &str {
        webhook_base(
            self.webhook_external_url.as_deref(),
            &self.rest_api_external_url,
        )
    }
}

/// Blank (e.g. `METEROID_WEBHOOK_EXTERNAL_URL=` in compose) means unset.
fn webhook_base<'a>(override_url: Option<&'a str>, rest_api_external_url: &'a str) -> &'a str {
    override_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(rest_api_external_url)
}

#[cfg(test)]
mod webhook_base_tests {
    use super::webhook_base;

    #[test]
    fn blank_override_falls_back_to_the_rest_url() {
        let rest = "https://api.example.com";
        assert_eq!(webhook_base(None, rest), rest);
        assert_eq!(webhook_base(Some(""), rest), rest);
        assert_eq!(webhook_base(Some("  "), rest), rest);
        assert_eq!(
            webhook_base(Some("https://hooks.example.com"), rest),
            "https://hooks.example.com"
        );
    }
}

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
    pub eventbus: Arc<dyn EventBus<Event>>,
    pub settings: Settings,
    pub(crate) internal: StoreInternal,
    pub(crate) oauth: OauthServices,
    pub mailer: Arc<dyn MailerService>,
    pub organization_hooks: Arc<dyn crate::hooks::OrganizationHooks>,
    pub instance: Arc<dyn crate::hooks::InstanceEdition>,
    pub hooks: crate::hooks::StoreHooks,
    pub usage_client: Arc<dyn UsageClient>,
    pub keep_last_events: crate::repositories::tenants::KeepLastEventsLookup,
    pub slug_ids: crate::repositories::tenants::SlugIds,
}

pub struct StoreConfig {
    pub pg: PgConfig,
    pub crypt_key: secrecy::SecretString,
    pub jwt_secret: secrecy::SecretString,
    pub multi_organization_enabled: bool,
    pub mailer_enabled: bool,
    pub public_url: String,
    pub rest_api_external_url: String,
    pub webhook_external_url: Option<String>,
    pub eventbus: Arc<dyn EventBus<Event>>,
    pub mailer: Arc<dyn MailerService>,
    pub oauth: OauthServices,
    pub domains_whitelist: Vec<String>,
    pub admin_organization_id: Option<OrganizationId>,
    pub organization_hooks: Arc<dyn crate::hooks::OrganizationHooks>,
    pub instance: Arc<dyn crate::hooks::InstanceEdition>,
    pub hooks: crate::hooks::StoreHooks,
    pub usage_client: Arc<dyn UsageClient>,
    pub invite_ttl_days: u32,
    pub mfa_issuer: String,
}

/**
 * Share store logic while allowing cross-service transactions
 * TODO divide between Service & Repository instead ?
 * Service => Exact mapping of the API, + validations, setup conn, call repository
 * Repository is often pass-through to `diesel_models` after mapping, but not always (can multiple queries, insert multiple entities, etc)
 */
#[derive(Clone, Default)]
pub struct StoreInternal {
    pub(crate) custom_property_definitions:
        Arc<crate::repositories::custom_properties::CustomPropertyDefinitionsCache>,
}

pub(crate) async fn pool_conn(pool: &PgPool) -> StoreResult<PgConn> {
    pool.get()
        .await
        .map_err(Report::from)
        .change_context(StoreError::DatabaseConnectionError)
        .attach("Failed to get a connection from the pool")
}

pub fn diesel_make_pg_pool(pg_config: PgConfig) -> StoreResult<PgPool> {
    let database_url = pg_config.database_url.as_str();
    let config = tokio_postgres::Config::from_str(database_url).unwrap();

    let mgr: AsyncDieselConnectionManager<AsyncPgConnection> = if config.get_ssl_mode()
        == tokio_postgres::config::SslMode::Disable
    {
        AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url)
    } else {
        let mut mgr_config = ManagerConfig::default();
        // First we have to construct a connection manager with our custom `establish_connection`
        // function
        mgr_config.custom_setup = Box::new(establish_secure_connection);

        // From that connection we can then create a pool, here given with some example settings.
        //
        // This creates a TLS configuration that's equivalent to `libpq` `sslmode=verify-full`, which
        // means this will check whether the provided certificate is valid for the given database host.
        //
        // `libpq` does not perform these checks by default (https://www.postgresql.org/docs/current/libpq-connect.html)
        // If you hit a TLS error while connecting to the database double-check your certificates

        AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(database_url, mgr_config)
    };

    Pool::builder(mgr)
        .max_size(pg_config.pool.max_size)
        .runtime(deadpool_runtime::Runtime::Tokio1)
        // Timeout for creating a new connection
        .create_timeout(Some(Duration::from_secs(
            pg_config.pool.create_timeout_secs,
        )))
        // Timeout for waiting to get a connection from the pool
        .wait_timeout(Some(Duration::from_secs(pg_config.pool.wait_timeout_secs)))
        // Timeout for the health check run when a connection is handed out again (not a max age)
        .recycle_timeout(Some(Duration::from_secs(
            pg_config.pool.recycle_timeout_secs,
        )))
        .build()
        .map_err(Report::from)
        .change_context(StoreError::InitializationError(
            "Database connection pool".into(),
        ))
        .attach("Failed to create PostgreSQL connection pool")
}

fn establish_secure_connection(db_url: &str) -> BoxFuture<'_, ConnectionResult<AsyncPgConnection>> {
    let fut = async {
        let tls = get_tls(db_url).unwrap();
        let (client, conn) = tokio_postgres::connect(db_url, tls)
            .await
            .map_err(|e| ConnectionError::BadConnection(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Database connection: {e}");
            }
        });
        AsyncPgConnection::try_from(client).await
    };
    fut.boxed()
}

impl Store {
    pub fn new(config: StoreConfig) -> StoreResult<Self> {
        // The usage client reads `keep_last_events` while its caller may hold every pooled
        // connection: on connections of its own, with short timeouts, it never waits on them.
        let lookup_pool = diesel_make_pg_pool(PgConfig {
            database_url: config.pg.database_url.clone(),
            pool: PgPoolConfig {
                max_size: 2,
                create_timeout_secs: 3,
                wait_timeout_secs: 3,
                recycle_timeout_secs: 3,
            },
        })?;
        let pool: PgPool = diesel_make_pg_pool(config.pg)?;

        Ok(Store {
            keep_last_events: crate::repositories::tenants::KeepLastEventsLookup::new(lookup_pool),
            slug_ids: crate::repositories::tenants::SlugIds::new(),
            pool,
            eventbus: config.eventbus,
            settings: Settings {
                crypt_key: config.crypt_key,
                jwt_secret: config.jwt_secret,
                multi_organization_enabled: config.multi_organization_enabled,
                public_url: config.public_url,
                rest_api_external_url: config.rest_api_external_url,
                webhook_external_url: config.webhook_external_url,
                mailer_enabled: config.mailer_enabled,
                domains_whitelist: config.domains_whitelist,
                admin_organization: config.admin_organization_id,
                invite_ttl_days: config.invite_ttl_days,
                mfa_issuer: config.mfa_issuer,
            },
            internal: StoreInternal::default(),
            mailer: config.mailer,
            oauth: config.oauth,
            organization_hooks: config.organization_hooks,
            instance: config.instance,
            hooks: config.hooks,
            usage_client: config.usage_client,
        })
    }

    /// `ENABLE_MULTI_ORGANIZATION`, where the edition allows several organizations.
    pub fn multi_organization_enabled(&self) -> bool {
        self.settings.multi_organization_enabled && self.instance.allows_multiple_organizations()
    }

    pub async fn get_conn(&self) -> StoreResult<PgConn> {
        pool_conn(&self.pool).await
    }

    pub fn oauth_egress(&self) -> &egress_http::EgressClient {
        self.oauth.egress()
    }

    pub async fn keep_last_events(&self, tenant_id: TenantId) -> StoreResult<bool> {
        self.keep_last_events.get(tenant_id).await
    }

    /// The store's lookup, for a client it holds (sharing its cache and its invalidation).
    pub fn keep_last_events_lookup(&self) -> crate::repositories::tenants::KeepLastEventsLookup {
        self.keep_last_events.clone()
    }

    pub async fn transaction<'a, R, F>(&self, callback: F) -> StoreResult<R>
    where
        F: for<'r> FnOnce(&'r mut PgConn) -> ScopedBoxFuture<'a, 'r, Result<R, StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, callback).await
    }

    pub async fn transaction_with<'a, R, F>(&self, conn: &mut PgConn, callback: F) -> StoreResult<R>
    where
        F: for<'r> FnOnce(&'r mut PgConn) -> ScopedBoxFuture<'a, 'r, Result<R, StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        self.internal.transaction_with(conn, callback).await
    }

    /// `transaction`, ending with the tenant's input bump (`credit_inputs::bump_tenant`).
    pub async fn transaction_bumping_tenant<'a, R, F>(
        &self,
        tenant_id: TenantId,
        callback: F,
    ) -> StoreResult<R>
    where
        F: for<'r> FnOnce(&'r mut PgConn) -> ScopedBoxFuture<'a, 'r, Result<R, StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let mut conn = self.get_conn().await?;
        self.internal
            .transaction_bumping_tenants(&mut conn, move |conn| {
                async move { Ok((callback(conn).await?, vec![tenant_id])) }.scope_boxed()
            })
            .await
    }

    /// `transaction`, ending with the input bump of the tenants the callback returns.
    pub async fn transaction_bumping_tenants<'a, R, F>(&self, callback: F) -> StoreResult<R>
    where
        F: for<'r> FnOnce(
                &'r mut PgConn,
            )
                -> ScopedBoxFuture<'a, 'r, Result<(R, Vec<TenantId>), StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let mut conn = self.get_conn().await?;
        self.internal
            .transaction_bumping_tenants(&mut conn, callback)
            .await
    }
}

impl StoreInternal {
    pub(crate) async fn transaction_with<'a, R, F>(
        &self,
        conn: &mut PgConn,
        callback: F,
    ) -> StoreResult<R>
    where
        F: for<'r> FnOnce(&'r mut PgConn) -> ScopedBoxFuture<'a, 'r, Result<R, StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let result = conn
            .transaction(async |conn| {
                let res = callback(conn);
                res.await.map_err(crate::errors::StoreErrorContainer::from)
            })
            .await?;

        Ok(result)
    }
}

impl StoreInternal {
    /// The only caller of `bump_tenants`: the bump is the transaction's last statement, since no
    /// lock protects the tenant rows and anything after it could wait while holding them.
    async fn transaction_bumping_tenants<'a, R, F>(
        &self,
        conn: &mut PgConn,
        callback: F,
    ) -> StoreResult<R>
    where
        F: for<'r> FnOnce(
                &'r mut PgConn,
            )
                -> ScopedBoxFuture<'a, 'r, Result<(R, Vec<TenantId>), StoreErrorReport>>
            + Send
            + 'a,
        R: Send + 'a,
    {
        let result = conn
            .transaction(async |conn| {
                let (result, tenants) = callback(conn)
                    .await
                    .map_err(crate::errors::StoreErrorContainer::from)?;
                diesel_models::query::credit_inputs::bump_tenants(conn, &tenants)
                    .await
                    .map_err(|e| {
                        crate::errors::StoreErrorContainer::from(Report::<StoreError>::from(e))
                    })?;
                Ok::<_, crate::errors::StoreErrorContainer>(result)
            })
            .await?;
        Ok(result)
    }
}

#[derive(Debug)]
// this is to ignore certificates for some providers like DO
struct DummyTlsVerifier;

impl ServerCertVerifier for DummyTlsVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer,
        _intermediates: &[CertificateDer],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn get_tls(database_url: &str) -> Option<MakeRustlsConnect> {
    let config = tokio_postgres::Config::from_str(database_url).unwrap();
    if config.get_ssl_mode() == tokio_postgres::config::SslMode::Disable {
        None
    } else {
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DummyTlsVerifier))
            .with_no_client_auth();

        Some(MakeRustlsConnect::new(tls_config))
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct PgConfig {
    #[envconfig(from = "DATABASE_URL")]
    pub database_url: String,
    #[envconfig(nested)]
    pub pool: PgPoolConfig,
}

impl PgConfig {
    pub fn new(database_url: String) -> Self {
        Self {
            database_url,
            pool: PgPoolConfig::default(),
        }
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct PgPoolConfig {
    #[envconfig(from = "PG_POOL_MAX_SIZE", default = "10")]
    pub max_size: usize,
    #[envconfig(from = "PG_POOL_CREATE_TIMEOUT_SECS", default = "30")]
    pub create_timeout_secs: u64,
    #[envconfig(from = "PG_POOL_WAIT_TIMEOUT_SECS", default = "30")]
    pub wait_timeout_secs: u64,
    #[envconfig(from = "PG_POOL_RECYCLE_TIMEOUT_SECS", default = "600")]
    pub recycle_timeout_secs: u64,
}

impl Default for PgPoolConfig {
    fn default() -> Self {
        Self {
            max_size: 10,
            create_timeout_secs: 30,
            wait_timeout_secs: 30,
            recycle_timeout_secs: 600,
        }
    }
}

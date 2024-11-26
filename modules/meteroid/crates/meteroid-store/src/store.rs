use crate::compute::clients::usage::UsageClient;
use crate::errors::StoreError;
use crate::StoreResult;
use common_eventbus::{Event, EventBus};
use diesel::{ConnectionError, ConnectionResult};
use diesel_async::pooled_connection::deadpool::Object;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::scoped_futures::{ScopedBoxFuture, ScopedFutureExt};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use error_stack::{Report, ResultExt};
use futures::future::BoxFuture;
use futures::FutureExt;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use std::str::FromStr;
use std::sync::Arc;
use svix::api::{ApplicationIn, ApplicationOut, Svix};
use tokio_postgres_rustls::MakeRustlsConnect;
use uuid::Uuid;

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
    pub(crate) usage_client: Arc<dyn UsageClient>,
    pub(crate) settings: Settings,
    pub(crate) internal: StoreInternal,
    pub(crate) svix: Option<Arc<Svix>>,
}

pub struct StoreConfig {
    pub database_url: String,
    pub crypt_key: secrecy::SecretString,
    pub jwt_secret: secrecy::SecretString,
    pub multi_organization_enabled: bool,
    pub eventbus: Arc<dyn EventBus<Event>>,
    pub usage_client: Arc<dyn UsageClient>,
    pub svix: Option<Arc<Svix>>,
}

/**
 * Share store logic while allowing cross-service transactions
 * TODO divide between Service & Repository instead ?
 * Service => Exact mapping of the API, + validations, setup conn, call repository
 * Repository is often pass-through to diesel_models after mapping, but not always (can multiple queries, insert multiple entities, etc)
 */
#[derive(Clone)]
pub struct StoreInternal {}

pub fn diesel_make_pg_pool(db_url: String) -> StoreResult<PgPool> {
    let config = tokio_postgres::Config::from_str(db_url.as_str()).unwrap();

    let mgr: AsyncDieselConnectionManager<AsyncPgConnection> =
        if config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable {
            let mut config = ManagerConfig::default();
            // First we have to construct a connection manager with our custom `establish_connection`
            // function
            config.custom_setup = Box::new(establish_secure_connection);

            // From that connection we can then create a pool, here given with some example settings.
            //
            // This creates a TLS configuration that's equivalent to `libpq` `sslmode=verify-full`, which
            // means this will check whether the provided certificate is valid for the given database host.
            //
            // `libpq` does not perform these checks by default (https://www.postgresql.org/docs/current/libpq-connect.html)
            // If you hit a TLS error while connecting to the database double-check your certificates

            AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(db_url, config)
        } else {
            AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url)
        };

    Pool::builder(mgr)
        .build()
        .map_err(Report::from)
        .change_context(StoreError::InitializationError)
        .attach_printable("Failed to create PostgreSQL connection pool")
}

fn establish_secure_connection(db_url: &str) -> BoxFuture<ConnectionResult<AsyncPgConnection>> {
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
        let pool: PgPool = diesel_make_pg_pool(config.database_url)?;

        Ok(Store {
            pool,
            eventbus: config.eventbus,
            usage_client: config.usage_client,
            settings: Settings {
                crypt_key: config.crypt_key,
                jwt_secret: config.jwt_secret,
                multi_organization_enabled: config.multi_organization_enabled,
            },
            internal: StoreInternal {},
            svix: config.svix,
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

    // Temporary, evaluating if this simplifies the handling of store + diesel interactions within a transaction

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

    pub(crate) fn svix(&self) -> StoreResult<Arc<Svix>> {
        self.svix
            .clone()
            .ok_or(StoreError::InitializationError.into())
    }

    pub(crate) async fn svix_application(&self, tenant_id: Uuid) -> StoreResult<ApplicationOut> {
        let svix = self.svix()?;
        let app_name = format!("tenant-{}", tenant_id);
        svix.application()
            .get_or_create(
                ApplicationIn {
                    metadata: None,
                    name: app_name,
                    rate_limit: None,
                    uid: Some(tenant_id.to_string()),
                },
                None,
            )
            .await
            .change_context(StoreError::WebhookServiceError(
                "Failed to get or create svix application".into(),
            ))
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
    if config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable {
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DummyTlsVerifier))
            .with_no_client_auth();

        Some(MakeRustlsConnect::new(tls_config))
    } else {
        None
    }
}

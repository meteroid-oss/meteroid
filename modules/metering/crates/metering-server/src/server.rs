use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use common_grpc::middleware::client::{LayeredClientService, build_layered_client_service};
use common_grpc::middleware::common::filters::Filter;
use common_grpc::middleware::server as common_middleware;
use metering::auth::ExternalApiAuthLayer;
use metering::config::Config;
use metering::connectors::clickhouse::ClickhouseConnector;
use metering::ingest;
use metering::ingest::sinks::kafka::KafkaSink;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use tonic::service::{Routes, RoutesBuilder};
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Endpoint, Server};
use tonic_tracing_opentelemetry::middleware as otel_middleware;

/// Which planes a metering process serves. `Combined` (the default) keeps ingest and
/// query in one process; the split modes let ingest (Kafka consumer + EventsService +
/// InternalEventsService) and query (UsageQueryService) run as separate deployables.
#[derive(Debug, Clone, Copy)]
pub enum ServerMode {
    Combined,
    Ingest,
    Query,
}

impl ServerMode {
    fn serves_ingest(self) -> bool {
        matches!(self, ServerMode::Combined | ServerMode::Ingest)
    }
    fn serves_query(self) -> bool {
        matches!(self, ServerMode::Combined | ServerMode::Query)
    }
}

fn only_internal(path: &str) -> bool {
    path.starts_with("/meteroid.metering.v1.UsageQueryService")
        || path.starts_with("/meteroid.metering.v1.InternalEventsService")
}

fn only_api(path: &str) -> bool {
    path.starts_with("/meteroid.metering.v1.EventsService")
}

fn reject_all(_path: &str) -> bool {
    false
}

pub type ExtraRoutesInit =
    Pin<Box<dyn Future<Output = Result<Routes, Box<dyn Error + Send + Sync>>> + Send>>;

/// gRPC services an edition serves on the metering port, next to the metering planes.
#[derive(Default)]
pub struct ExtraRoutes {
    /// Awaited once the port is bound, after the metering services are initialised.
    pub routes: Option<ExtraRoutesInit>,
    /// Encoded file descriptor sets added to the reflection service.
    pub descriptor_sets: Vec<&'static [u8]>,
    /// Paths that require internal (HMAC) auth, on top of the metering internal services.
    pub internal_paths: Option<Filter>,
}

/// Run both planes in one process (ingest + query). Default entry point.
pub async fn start_server(config: Config) -> Result<(), Box<dyn Error>> {
    start_server_with(config, ServerMode::Combined, ExtraRoutes::default()).await
}

pub async fn start_server_with(
    config: Config,
    mode: ServerMode,
    extra: ExtraRoutes,
) -> Result<(), Box<dyn Error>> {
    let incoming = bind(&config)?;
    start_server_on_with(config, mode, extra, incoming).await
}

/// Serves on a socket bound by the caller, before the migrations run: connections wait in its
/// backlog until the services (health included) are served.
pub async fn start_server_on(
    config: Config,
    mode: ServerMode,
    incoming: TcpIncoming,
) -> Result<(), Box<dyn Error>> {
    start_server_on_with(config, mode, ExtraRoutes::default(), incoming).await
}

pub async fn start_server_on_with(
    config: Config,
    mode: ServerMode,
    extra: ExtraRoutes,
    incoming: TcpIncoming,
) -> Result<(), Box<dyn Error>> {
    let internal_client = create_meteroid_internal_client(&config).await;

    // The ingest plane owns the Kafka -> ClickHouse consumer. It writes to tables the query
    // plane migrates: it waits for those migrations when this server runs them.
    let (migrated, wait_migrated) = tokio::sync::oneshot::channel::<()>();
    if mode.serves_ingest() {
        let kafka_cfg = config.kafka.clone();
        let ch_cfg = config.clickhouse.clone();
        let consumer_cfg = config.consumer.clone();
        tokio::spawn(async move {
            // A closed channel means the start failed: the tables may not be there.
            if wait_migrated.await.is_ok() {
                ingest::consumer::run(&kafka_cfg, &ch_cfg, &consumer_cfg).await;
            }
        });
    }

    start_api_server(
        config.clone(),
        internal_client,
        mode,
        extra,
        incoming,
        migrated,
    )
    .await
    .inspect_err(|e| log::error!("Error starting API server: {e:?}"))
}

async fn start_api_server(
    config: Config,
    internal_client: InternalServiceClient<LayeredClientService>,
    mode: ServerMode,
    extra: ExtraRoutes,
    incoming: TcpIncoming,
    migrated: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Box<dyn Error>> {
    log::info!(
        "Starting Metering API grpc server ({:?}) on port {}",
        mode,
        config.listen_addr.port()
    );

    let ExtraRoutes {
        routes: extra_routes,
        descriptor_sets,
        internal_paths,
    } = extra;

    let (_, health_service) = tonic_health::server::health_reporter();

    let reflection_service = descriptor_sets
        .into_iter()
        .fold(
            tonic_reflection::server::Builder::configure().register_encoded_file_descriptor_set(
                meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET,
            ),
            |builder, set| builder.register_encoded_file_descriptor_set(set),
        )
        .build_v1()
        .unwrap();

    let admin_auth_layer =
        common_middleware::auth::create_admin(&config.internal_auth).filter(only_internal);
    let extra_admin_auth_layer = common_middleware::auth::create_admin(&config.internal_auth)
        .filter(internal_paths.unwrap_or(reject_all));

    // Path-filtered, so it is inert in query-only mode (no EventsService registered).
    let api_key_auth_layer = ExternalApiAuthLayer::new(internal_client.clone()).filter(only_api);

    let query_service = if mode.serves_query() {
        let connector =
            Arc::new(ClickhouseConnector::init(&config.clickhouse, &config.kafka, vec![]).await?);
        Some(crate::query::service(connector))
    } else {
        None
    };
    let _ = migrated.send(());

    let ingest_services = if mode.serves_ingest() {
        let sink = Arc::new(KafkaSink::new(&config.kafka)?);
        Some((
            ingest::service(internal_client.clone(), sink.clone()),
            ingest::internal_service(internal_client.clone(), sink),
        ))
    } else {
        None
    };

    let extra_routes = match extra_routes {
        Some(init) => init.await.map_err(|e| -> Box<dyn Error> { e })?,
        None => RoutesBuilder::default().routes(),
    };

    let mut router = Server::builder()
        .layer(common_middleware::metric::create())
        .layer(api_key_auth_layer)
        .layer(admin_auth_layer)
        .layer(extra_admin_auth_layer)
        .layer(otel_middleware::server::OtelGrpcLayer::default().filter(
            if config.common.telemetry.tracing_enabled {
                otel_middleware::filters::reject_healthcheck
            } else {
                reject_all
            },
        ))
        // Inside the OTel layer and the auth layer, so the failure log of a call carries its
        // trace and its tenant.
        .layer(common_middleware::context::RequestContextLayer)
        .layer(common_middleware::error_logger::create())
        .add_routes(extra_routes)
        .add_service(health_service)
        .add_service(reflection_service);

    if let Some(query_service) = query_service {
        router = router.add_service(query_service);
    }

    if let Some((event_service, internal_event_service)) = ingest_services {
        router = router
            .add_service(event_service)
            .add_service(internal_event_service);
    }

    router.serve_with_incoming(incoming).await?;

    Ok(())
}

// Before the migrations, so a taken port fails the start at once.
fn bind(config: &Config) -> Result<TcpIncoming, Box<dyn Error>> {
    TcpIncoming::bind(config.listen_addr)
        .inspect_err(|e| log::error!("Could not bind {}: {e:?}", config.listen_addr))
        .map_err(Into::into)
}

async fn create_meteroid_internal_client(
    config: &Config,
) -> InternalServiceClient<LayeredClientService> {
    let channel = Endpoint::from_shared(config.meteroid_endpoint.clone())
        .expect("Failed to create channel to meteroid from shared endpoint");

    let channel = channel
        .connect()
        .await
        .or_else(|e| {
            log::warn!("Failed to connect to the meteroid GRPC channel for endpoint {}: {}. Starting in lazy mode.", config.meteroid_endpoint.clone(), e);
            Ok::<Channel, tonic::transport::Error>(channel.connect_lazy())
        }).expect("Failed to connect to the meteroid GRPC channel");

    let service = build_layered_client_service(channel, &config.internal_auth);

    InternalServiceClient::new(service)
}

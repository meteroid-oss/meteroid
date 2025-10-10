use crate::auth::ExternalApiAuthLayer;
use crate::config::{Config, KafkaConfig};

use crate::ingest;

#[cfg(feature = "kafka")]
use crate::ingest::sinks::kafka::KafkaSink;

use common_grpc::middleware::server as common_middleware;

#[cfg(not(feature = "kafka"))]
use crate::ingest::sinks::print::PrintSink;
use common_grpc::middleware::client::{LayeredClientService, build_layered_client_service};
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use std::sync::Arc;
use tonic::transport::{Channel, Endpoint, Server};
use tonic_tracing_opentelemetry::middleware as otel_middleware;

#[cfg(feature = "clickhouse")]
use crate::connectors::clickhouse::ClickhouseConnector;
#[cfg(feature = "openstack")]
use crate::connectors::clickhouse::extensions::openstack_ext::OpenstackClickhouseExtension;

#[cfg(not(feature = "clickhouse"))]
use crate::connectors::PrintConnector;
#[cfg(feature = "kafka")]
use crate::preprocessor::run_raw_preprocessor;

fn only_internal(path: &str) -> bool {
    path.starts_with("/meteroid.metering.v1.UsageQueryService")
        || path.starts_with("/meteroid.metering.v1.MetersService")
        || path.starts_with("/meteroid.metering.v1.InternalEventsService")
}

fn only_api(path: &str) -> bool {
    path.starts_with("/meteroid.metering.v1.EventsService")
}

pub async fn start_server(config: Config) {
    let internal_client = create_meteroid_internal_client(&config).await;
    #[cfg(feature = "kafka")]
    let internal_client_clone = internal_client.clone();
    let api_server = start_api_server(config.clone(), internal_client);
    #[cfg(feature = "kafka")]
    let kafka_workers = create_kafka_workers(&config.kafka, internal_client_clone);

    #[cfg(feature = "kafka")]
    tokio::select! {
          result = api_server => {
            if let Err(e) = result {
                log::error!("Error starting API server: {e:?}");
            }
        },
        () = kafka_workers => {
              log::warn!("Workers terminated");
        }
    }

    #[cfg(not(feature = "kafka"))]
    if let Err(e) = api_server.await {
        log::error!("Error starting API server: {}", e);
    }
}

pub async fn start_api_server(
    config: Config,
    internal_client: InternalServiceClient<LayeredClientService>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!(
        "Starting Metering API grpc server on port {}",
        config.listen_addr.port()
    );

    #[cfg(feature = "clickhouse")]
    let connector = {
        log::info!("Clickhouse connector enabled");
        let conn = ClickhouseConnector::init(
            &config.clickhouse,
            &config.kafka,
            vec![
                #[cfg(feature = "openstack")]
                Arc::new(OpenstackClickhouseExtension {}),
            ],
        )
        .await?;

        Arc::new(conn)
    };
    #[cfg(not(feature = "clickhouse"))]
    let connector = Arc::new(PrintConnector {});

    #[cfg(feature = "kafka")]
    let sink = Arc::new(KafkaSink::new(&config.kafka)?);

    #[cfg(not(feature = "kafka"))]
    let sink = Arc::new(PrintSink {});

    let (_, health_service) = tonic_health::server::health_reporter();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let admin_auth_layer =
        common_middleware::auth::create_admin(&config.internal_auth).filter(only_internal);

    let api_key_auth_layer = ExternalApiAuthLayer::new(internal_client.clone()).filter(only_api);

    // Ingest for Api key
    let event_service = ingest::service(internal_client.clone(), sink.clone());

    // Meters & queries & ingest => Admin
    let internal_event_service = ingest::internal_service(internal_client.clone(), sink.clone());
    let meter_service = crate::meters::service(connector.clone());
    let query_service = crate::query::service(connector.clone());

    Server::builder()
        .layer(common_middleware::metric::create())
        .layer(api_key_auth_layer)
        .layer(admin_auth_layer)
        .layer(common_middleware::error_logger::create())
        .layer(
            otel_middleware::server::OtelGrpcLayer::default()
                .filter(otel_middleware::filters::reject_healthcheck),
        )
        .add_service(health_service)
        .add_service(reflection_service)
        .add_service(meter_service)
        .add_service(query_service)
        .add_service(event_service)
        .add_service(internal_event_service)
        .serve(config.listen_addr)
        .await?;

    Ok(())
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

async fn create_kafka_workers(
    config: &KafkaConfig,
    internal_client: InternalServiceClient<LayeredClientService>,
) {
    run_raw_preprocessor(config, internal_client).await;
}

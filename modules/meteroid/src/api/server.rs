use std::sync::Arc;

use deadpool_postgres::Pool;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware as otel_middleware;
use tonic_web::GrpcWebLayer;

use common_grpc::middleware::client::build_layered_client_service;
use common_grpc::middleware::common::filters as common_filters;
use common_grpc::middleware::server as common_middleware;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

use crate::api;
use crate::api::cors::cors;
use crate::compute::InvoiceEngine;
use crate::eventbus::analytics_handler::AnalyticsHandler;
use crate::eventbus::webhook_handler::WebhookHandler;
use crate::eventbus::{Event, EventBus};
use crate::repo::provider_config::ProviderConfigRepo;

use super::super::config::Config;

pub async fn start_api_server(
    config: Config,
    pool: Pool,
    provider_config_repo: Arc<dyn ProviderConfigRepo>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!(
        "Starting Billing API grpc server on port {}",
        config.listen_addr.port()
    );

    let (_, health_service) = tonic_health::server::health_reporter();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .build()
        .unwrap();

    let metering_channel = tonic::transport::Channel::from_shared(config.metering_endpoint.clone())
        .expect("Invalid metering_endpoint")
        .connect_lazy();
    let metering_layered_channel =
        build_layered_client_service(metering_channel, &config.internal_auth);

    let query_service_client = UsageQueryServiceClient::new(metering_layered_channel.clone());
    let metering_service = MetersServiceClient::new(metering_layered_channel);

    let compute_service = Arc::new(InvoiceEngine::new(query_service_client));

    let eventbus: Arc<dyn EventBus<Event>> = Arc::new(crate::eventbus::memory::InMemory::new());

    eventbus
        .subscribe(Arc::new(WebhookHandler::new(
            pool.clone(),
            config.secrets_crypt_key.clone(),
            true,
        )))
        .await;

    if config.analytics.enabled {
        let country = match crate::eventbus::analytics_handler::get_geoip().await {
            Ok(geoip) => Some(geoip.country),
            Err(err) => {
                log::warn!("Failed to obtain data for analytics: {}", err);
                None
            }
        };

        eventbus
            .subscribe(Arc::new(AnalyticsHandler::new(
                config.analytics.clone(),
                pool.clone(),
                country,
            )))
            .await;
    } else {
        log::info!("Analytics is disabled");
    }

    Server::builder()
        .accept_http1(true)
        .layer(cors())
        .layer(GrpcWebLayer::new())
        .layer(common_middleware::metric::create())
        .layer(
            common_middleware::auth::create(config.jwt_secret.clone(), pool.clone())
                .filter(common_filters::only_api),
        )
        .layer(
            common_middleware::auth::create_admin(&config.internal_auth)
                .filter(common_filters::only_internal),
        )
        .layer(
            otel_middleware::server::OtelGrpcLayer::default()
                .filter(otel_middleware::filters::reject_healthcheck),
        )
        .layer(common_middleware::error_logger::create())
        .add_service(health_service)
        .add_service(reflection_service)
        .add_service(api::billablemetrics::service(
            pool.clone(),
            eventbus.clone(),
            metering_service,
        ))
        .add_service(api::customers::service(pool.clone(), eventbus.clone()))
        .add_service(api::tenants::service(pool.clone(), provider_config_repo))
        .add_service(api::apitokens::service(pool.clone(), eventbus.clone()))
        .add_service(api::pricecomponents::service(
            pool.clone(),
            eventbus.clone(),
        ))
        .add_service(api::plans::service(pool.clone(), eventbus.clone()))
        .add_service(api::schedules::service(pool.clone()))
        .add_service(api::productitems::service(pool.clone()))
        .add_service(api::productfamilies::service(
            pool.clone(),
            eventbus.clone(),
        ))
        .add_service(api::instance::service(pool.clone(), eventbus.clone()))
        .add_service(api::invoices::service(pool.clone()))
        .add_service(api::stats::service(pool.clone()))
        .add_service(api::users::service(
            pool.clone(),
            eventbus.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::subscriptions::service(
            pool.clone(),
            compute_service,
            eventbus.clone(),
        ))
        .add_service(api::webhooksout::service(
            pool.clone(),
            config.secrets_crypt_key.clone(),
        ))
        .add_service(api::internal::service(pool.clone()))
        .serve(config.listen_addr)
        .await?;

    Ok(())
}

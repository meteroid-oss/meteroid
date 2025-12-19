use std::sync::Arc;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware as otel_middleware;
use tonic_web::GrpcWebLayer;

use common_grpc::middleware::common::filters as common_filters;
use common_grpc::middleware::server as common_middleware;
use meteroid_store::{Services, Store};

use crate::api;
use crate::api::cors::cors;
use crate::services::customer_ingest::CustomerIngestService;
use crate::services::invoice_rendering::InvoicePreviewRenderingService;
use crate::services::storage::ObjectStoreService;

use super::super::config::Config;

fn reject_all(_path: &str) -> bool {
    false
}

pub async fn start_api_server(
    config: Config,
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    svix: Option<Arc<svix::api::Svix>>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Starting GRPC API on {}", config.grpc_listen_addr);

    let preview_rendering = InvoicePreviewRenderingService::try_new(
        Arc::new(store.clone()),
        object_store.clone(),
        config.public_url.clone(),
        config.jwt_secret.clone(),
    )?;

    let (_, health_service) = tonic_health::server::health_reporter();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    // Default max message size is 4 MB, override on per service layer (ie CustomerIngestService) as needed
    Server::builder()
        .accept_http1(true)
        .layer(cors())
        .layer(GrpcWebLayer::new())
        .layer(common_middleware::metric::create())
        .layer(
            meteroid_middleware::server::auth::create(config.jwt_secret.clone(), store.clone())
                .filter(|path| common_filters::only_api(path) || common_filters::only_portal(path)),
        )
        .layer(
            common_middleware::auth::create_admin(&config.internal_auth)
                .filter(common_filters::only_internal),
        )
        .layer(common_middleware::error_logger::create())
        .layer(otel_middleware::server::OtelGrpcLayer::default().filter(
            if config.common.telemetry.tracing_enabled {
                otel_middleware::filters::reject_healthcheck
            } else {
                reject_all
            },
        ))
        .add_service(health_service)
        .add_service(reflection_service)
        .add_service(api::addons::service(store.clone()))
        .add_service(api::billablemetrics::service(store.clone()))
        .add_service(api::bankaccounts::service(store.clone()))
        .add_service(api::organizations::service(store.clone(), services.clone()))
        .add_service(api::invoicingentities::service(
            store.clone(),
            object_store.clone(),
        ))
        .add_service(api::connectors::service(store.clone(), services.clone()))
        .add_service(api::coupons::service(store.clone()))
        .add_service(api::customers::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::customers::ingest_service(CustomerIngestService::new(
            Arc::new(store.clone()),
        )))
        .add_service(api::events::service(store.clone(), services.clone()))
        .add_service(api::events::ingest_service(services.clone()))
        .add_service(api::tenants::service(store.clone(), services.clone()))
        .add_service(api::apitokens::service(store.clone()))
        .add_service(api::pricecomponents::service(store.clone()))
        .add_service(api::plans::service(store.clone()))
        .add_service(api::schedules::service(store.clone()))
        .add_service(api::productitems::service(store.clone()))
        .add_service(api::productfamilies::service(store.clone()))
        .add_service(api::quotes::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::instance::service(
            store.clone(),
            config.svix.server_url.is_some(),
        ))
        .add_service(api::invoices::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            preview_rendering,
        ))
        .add_service(api::stats::service(store.clone()))
        .add_service(api::users::service(store.clone()))
        .add_service(api::subscriptions::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::taxes::service(store.clone()))
        .add_service(api::webhooksout::service(svix))
        .add_service(api::internal::service(store.clone()))
        .add_service(api::portal::checkout::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::portal::customer::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
            config.rest_api_external_url.clone(),
        ))
        .add_service(api::portal::invoice::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::portal::shared::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::portal::quotes::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
        ))
        .serve(config.grpc_listen_addr)
        .await?;

    Ok(())
}

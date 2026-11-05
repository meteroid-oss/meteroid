use std::sync::Arc;

use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic_tracing_opentelemetry::middleware as otel_middleware;
use tonic_web::GrpcWebLayer;

use common_domain::ids::BaseId;
use common_grpc::middleware::common::filters as common_filters;
use common_grpc::middleware::server as common_middleware;
use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use meteroid_store::{Services, Store};
use rate_limiter::middleware::tonic::RateLimitLayer;
use rate_limiter::{RateLimitConfig, RateLimiter};

use crate as api;
use meteroid_app::cors::cors;
use meteroid_app::services::credit_note_rendering::CreditNotePreviewRenderingService;
use meteroid_app::services::egress::{Egress, EgressPurpose};
use meteroid_app::services::invoice_rendering::InvoicePreviewRenderingService;
use meteroid_app::services::storage::ObjectStoreService;
use meteroid_kit::{ApiContext, Modules};
use meteroid_rest_common::webhook_catalog::WebhookCatalog;

use meteroid_app::config::Config;

fn reject_all(_path: &str) -> bool {
    false
}

const PURCHASE_SERVICE_PREFIX: &str = "/meteroid.portal.purchase.v1.PurchaseService/";

pub async fn start_api_server(
    config: Config,
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    rate_limiter: RateLimiter,
    redis_client: Option<fred::prelude::Client>,
    modules: Modules,
    egress: Egress,
    webhook_catalog: Arc<dyn WebhookCatalog>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Bind eagerly so a port conflict fails fast here rather than after startup.
    let incoming = TcpIncoming::bind(config.api.grpc_listen_addr)?;
    start_api_server_with_incoming(
        config,
        store,
        services,
        object_store,
        rate_limiter,
        redis_client,
        incoming,
        modules,
        egress,
        webhook_catalog,
    )
    .await
}

// Serves on an already-bound socket. Tests bind the listener up-front (holding
// the port through `serve`) to avoid a TOCTOU race on ephemeral test ports.
pub async fn start_api_server_with_incoming(
    config: Config,
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    rate_limiter: RateLimiter,
    redis_client: Option<fred::prelude::Client>,
    incoming: TcpIncoming,
    modules: Modules,
    egress: Egress,
    webhook_catalog: Arc<dyn WebhookCatalog>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Starting GRPC API on {}", config.api.grpc_listen_addr);

    let preview_rendering = InvoicePreviewRenderingService::try_new(
        Arc::new(store.clone()),
        object_store.clone(),
        config.public_url.clone(),
        config.jwt_secret.clone(),
    )?;

    let credit_note_preview_rendering =
        CreditNotePreviewRenderingService::try_new(Arc::new(store.clone()), object_store.clone())?;

    // Same SSRF guards as the delivery worker, so an endpoint the worker would refuse
    // is rejected at create/update time instead.
    let webhook_egress = egress.client(EgressPurpose::Webhooks)?;

    let (_, health_service) = tonic_health::server::health_reporter();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let feature_resolver = modules.entitlements(&store);
    let feature_gates: Arc<[_]> = modules.feature_gates().into();

    let analytics = modules.analytics_hook(&store, &config, &egress);

    let grpc_limiter = rate_limiter.with_config(
        RateLimitConfig::per_minute(config.api.rate_limit.grpc.rpm).with_bucket("grpc"),
    );

    // Per-IP limiter used as the fallthrough for unauthenticated paths
    // (Login, VerifyMfa, InitResetPassword, …). The per-email / per-user
    // layer is enforced in-handler via `AuthThrottle` — see `auth_throttle.rs`.
    let auth_throttle = meteroid_app::auth_throttle::AuthThrottle::from_base(&rate_limiter);

    let purchase_links = meteroid_app::services::purchase_links::PurchaseLinks::new(
        store.clone(),
        config.jwt_secret.clone(),
        config.public_url.clone(),
        &rate_limiter,
    );
    let trusted_proxy_hops = config.api.trusted_proxy_hops;

    let module_ctx = ApiContext {
        config: config.clone(),
        store: store.clone(),
        services: services.clone(),
        feature_resolver: feature_resolver.clone(),
        feature_gates: feature_gates.clone(),
        rate_limiter: rate_limiter.clone(),
        redis: redis_client,
        analytics,
        egress,
    };
    let mut module_routes = tonic::service::RoutesBuilder::default();
    for module in &modules.api {
        module.grpc(&module_ctx, &mut module_routes);
    }
    let support_grants = modules.support_grants(&module_ctx);
    let sandboxes = modules.sandboxes(&module_ctx);
    let revenue_stats = modules.revenue_stats(&module_ctx);
    let portal_licenses = modules.portal_licenses(&module_ctx);

    // Default max message size is 4 MB, override on per service layer as needed
    Server::builder()
        .accept_http1(true)
        .layer(cors())
        .layer(GrpcWebLayer::new())
        .layer(common_middleware::metric::create())
        .layer(tower_http::add_extension::AddExtensionLayer::new(
            meteroid_app::security_context::TrustedProxyHops(config.api.trusted_proxy_hops),
        ))
        .layer(
            meteroid_middleware::server::auth::create(
                config.jwt_secret.clone(),
                store.clone(),
                support_grants.clone(),
            )
            .module_policies(modules.route_policies())
            .filter(|path| {
                common_filters::only_api(path)
                    || common_filters::only_portal(path)
                    || common_filters::only_admin(path)
            }),
        )
        .layer(
            RateLimitLayer::new(move |req: &http::Request<_>| {
                rate_limit_subject(req, trusted_proxy_hops).map(|subject| {
                    let key = format!("{}:{}", subject, req.uri().path());
                    (key, grpc_limiter.clone())
                })
            })
            .filter(|path| common_filters::only_api(path) || common_filters::only_portal(path)),
        )
        .layer(
            common_middleware::auth::create_admin(&config.internal_auth)
                .filter(common_filters::only_internal),
        )
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
        .add_routes(module_routes.routes())
        .add_service(health_service)
        .add_service(reflection_service)
        .add_service(api::activity::service(
            store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::rbac::service(store.clone(), feature_resolver.clone()))
        .add_service(api::addons::service(
            store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::billablemetrics::service(
            store.clone(),
            services.usage_clients(),
        ))
        .add_service(api::bankaccounts::service(store.clone()))
        .add_service(api::customproperties::service(store.clone()))
        .add_service(api::batchjobs::service(
            store.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::organizations::service(
            store.clone(),
            feature_resolver.clone(),
            support_grants,
            sandboxes.clone(),
        ))
        .add_service(api::invoicingentities::service(
            store.clone(),
            object_store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::connectors::service(store.clone(), services.clone()))
        .add_service(api::coupons::service(store.clone()))
        .add_service(api::creditnotes::service(
            store.clone(),
            services.clone(),
            credit_note_preview_rendering,
            config.jwt_secret.clone(),
        ))
        .add_service(api::customers::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::customeremails::service(
            store.clone(),
            services.clone(),
            feature_resolver.clone(),
        ))
        .add_service(meteroid_app::featuregates::service(
            store.clone(),
            feature_resolver.clone(),
            feature_gates,
        ))
        .add_service(api::events::service(store.clone(), services.clone()))
        .add_service(api::eventinsights::service(store.clone(), services.clone()))
        .add_service(api::entitlements::entitlements_service(
            store.clone(),
            services.clone(),
        ))
        .add_service(api::tenants::service(
            store.clone(),
            services.clone(),
            feature_resolver.clone(),
            sandboxes,
        ))
        .add_service(api::apitokens::service(store.clone()))
        .add_service(api::pricecomponents::service(
            store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::plans::service(store.clone(), feature_resolver.clone()))
        .add_service(api::schedules::service(store.clone()))
        .add_service(api::prices::service(store.clone(), services.clone()))
        .add_service(api::productitems::service(
            store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::productfamilies::service(store.clone()))
        .add_service(api::purchaselinks::service(
            store.clone(),
            config.public_url.clone(),
        ))
        .add_service(api::quotes::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::instance::service(store.clone()))
        .add_service(api::invoices::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            preview_rendering,
            feature_resolver.clone(),
        ))
        .add_service(api::stats::service(store.clone(), revenue_stats))
        .add_service(api::listviews::service(store.clone()))
        .add_service(api::users::service(
            store.clone(),
            auth_throttle.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::subscriptions::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::taxes::service(
            store.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
        ))
        .add_service(api::webhooksout::service(
            store.clone(),
            webhook_egress,
            config.webhook_out.clone(),
            modules.webhook_events(),
            webhook_catalog,
        ))
        .add_service(api::internal::service(store.clone()))
        .add_service(api::portal::checkout::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
            &rate_limiter,
            feature_resolver.clone(),
        ))
        .add_service(api::portal::customer::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
            config.rest_api_external_url.clone(),
            feature_resolver.clone(),
            portal_licenses,
        ))
        .add_service(api::portal::invoice::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::wallets::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::creditunits::service(
            store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::portal::wallets::service(
            store.clone(),
            services.clone(),
            &rate_limiter,
            feature_resolver.clone(),
        ))
        .add_service(api::portal::shared::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            config.jwt_secret.clone(),
            &rate_limiter,
        ))
        .add_service(api::portal::quotes::service(
            store.clone(),
            services.clone(),
            object_store.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::portal::subscription::service(
            store.clone(),
            services.clone(),
            config.jwt_secret.clone(),
            feature_resolver.clone(),
        ))
        .add_service(api::portal::purchase::service(
            store.clone(),
            purchase_links,
            object_store.clone(),
        ))
        .serve_with_incoming(incoming)
        .await?;

    Ok(())
}

/// Who a call is rate limited as: its actor, or for a portal link without one (checkout, invoice,
/// quote, dashboard) the resource the link opens, else the client IP for anonymous purchases.
fn rate_limit_subject<B>(req: &http::Request<B>, trusted_proxy_hops: usize) -> Option<String> {
    let subject = req.actor().ok().or_else(|| {
        req.portal_resource()
            .ok()
            .map(|portal| match portal.resource_access {
                ResourceAccess::CheckoutSession(id) => id.as_uuid(),
                ResourceAccess::CustomerPortal(id) => id.as_uuid(),
                ResourceAccess::InvoicePortal(id) => id.as_uuid(),
                ResourceAccess::QuotePortal { quote_id, .. } => quote_id.as_uuid(),
                ResourceAccess::DashboardPortal(id) => id.as_uuid(),
            })
    });
    if let Some(subject) = subject {
        return Some(subject.simple().to_string());
    }
    if req.uri().path().starts_with(PURCHASE_SERVICE_PREFIX) {
        return meteroid_app::security_context::client_ip_from_http(req, trusted_proxy_hops)
            .map(|ip| format!("ip:{ip}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::rate_limit_subject;
    use common_domain::ids::{BaseId, CheckoutSessionId, QuoteId, TenantId};
    use common_grpc::middleware::server::AuthorizedState;
    use common_grpc::middleware::server::auth::{AuthorizedAsPortalUser, ResourceAccess};

    fn portal_request(resource_access: ResourceAccess) -> http::Request<()> {
        let mut request = http::Request::new(());
        request
            .extensions_mut()
            .insert(AuthorizedState::Shared(AuthorizedAsPortalUser {
                tenant_id: TenantId::new(),
                resource_access,
                scopes: None,
                delegated: false,
            }));
        request
    }

    #[test]
    fn portal_links_without_an_actor_are_rate_limited_by_their_resource() {
        let session = CheckoutSessionId::new();
        assert_eq!(
            rate_limit_subject(&portal_request(ResourceAccess::CheckoutSession(session)), 0),
            Some(session.as_uuid().simple().to_string())
        );
        let quote_id = QuoteId::new();
        assert_eq!(
            rate_limit_subject(
                &portal_request(ResourceAccess::QuotePortal {
                    quote_id,
                    recipient_email: "buyer@example.com".to_owned(),
                }),
                0
            ),
            Some(quote_id.as_uuid().simple().to_string())
        );
        assert_eq!(rate_limit_subject(&http::Request::new(()), 0), None);
    }
}

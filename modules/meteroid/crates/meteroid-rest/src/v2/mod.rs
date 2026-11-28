//! The public REST API v2 (ADR-0013), served under `/api/v2` beside the frozen v1 by the same
//! binary, with the same API keys and tenants. Its own OpenAPI document is
//! `spec/api/v2/openapi.json`.
//!
//! The conventions every v2 route shares live here: the dated `Meteroid-Version` and the problem
//! responses (`envelope`, outside authentication), `Idempotency-Key` on every POST and the
//! PATCHes listing changes (`idempotency`, inside it), cursor pagination (`pagination`) and `{id_or_code}` paths (`ids`).
//! A new resource adds its router to [`resource_routes`]; an edition's module adds its own
//! (`RestRoutes::v2`), mounted by [`module_routes`] under the same middlewares and published in
//! the edition's v2 document, finished as JSON by [`spec_json`] (`document`).
use crate::AppState;
use meteroid_rest_common::spec::{EditionSpec, OperationEdition, mark_operations};
use std::collections::BTreeMap;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

pub mod add_ons;
pub mod bounds;
pub mod charges;
pub mod checkout_sessions;
pub mod coupons;
pub mod credit_notes;
pub mod credit_units;
pub mod custom_properties;
pub mod customers;
pub mod document;
pub mod documents;
pub mod entitlements;
pub mod envelope;
pub mod events;
pub mod examples;
pub mod extract;
pub mod features;
pub mod idempotency;
pub mod ids;
pub mod invoices;
pub mod managed_quantities;
pub mod me;
pub mod metrics;
pub mod offers;
pub mod pagination;
pub mod plans;
pub mod pricing_pages;
pub mod problem;
pub mod product_families;
pub mod products;
pub mod rate_cards;
pub mod references;
pub mod refs;
pub mod resource_types;
pub mod subscriptions;
pub mod trials;
pub mod usage;
pub mod version;
pub mod webhook_payloads;
pub mod webhooks;

/// What the v2 renderers read: the store and its services. A request's `AppState` holds them,
/// and so does the webhook worker, which renders payloads in v2's shapes with the GETs' own
/// renderers (`webhook_payloads::Renderer`).
pub trait Reader: Sync {
    fn store(&self) -> &meteroid_store::Store;
    fn services(&self) -> &meteroid_store::Services;
}

impl Reader for AppState {
    fn store(&self) -> &meteroid_store::Store {
        &self.store
    }

    fn services(&self) -> &meteroid_store::Services {
        &self.services
    }
}

/// Core's v2 routers, one per resource, named.
fn resource_routes() -> Vec<(&'static str, OpenApiRouter<AppState>)> {
    vec![
        ("plans", plans::routes()),
        ("products", products::routes()),
        ("product_families", product_families::routes()),
        ("managed_quantities", managed_quantities::routes()),
        ("metrics", metrics::routes()),
        ("rate_cards", rate_cards::routes()),
        ("add_ons", add_ons::routes()),
        ("features", features::routes()),
        ("entitlements", entitlements::routes()),
        ("resource_types", resource_types::routes()),
        ("coupons", coupons::routes()),
        ("credit_units", credit_units::routes()),
        ("pricing_pages", pricing_pages::routes()),
        ("subscriptions", subscriptions::routes()),
        ("customers", customers::routes()),
        ("custom_properties", custom_properties::routes()),
        ("invoices", invoices::routes()),
        ("credit_notes", credit_notes::routes()),
        ("events", events::routes()),
        ("usage", usage::routes()),
        ("checkout_sessions", checkout_sessions::routes()),
        ("webhooks", webhooks::routes()),
        ("me", me::routes()),
    ]
}

pub fn api_v2_routes() -> OpenApiRouter<AppState> {
    resource_routes()
        .into_iter()
        .fold(OpenApiRouter::new(), |router, (_, routes)| {
            router.merge(routes)
        })
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Meteroid API v2",
        version = "2026-12-15",
        description = "The public REST API v2. A request may send `Meteroid-Version: YYYY-MM-DD` \
            to pin a version (the first, 2026-12-01, by default) and `X-Request-Id`; every \
            response echoes the version it was served at and its `Request-Id`. These headers are \
            documented once (`components.parameters`, `components.headers`), on no operation: \
            an SDK sends them itself. A POST other than a preview takes an `Idempotency-Key`, and \
            so does `PATCH /rate-cards/{id_or_code}/rates`, whose body lists changes: a \
            retry with the same key and request replays the first response \
            (`Idempotent-Replayed: true`), or an `IdempotentReplayMarker` naming the \
            resource's `id` when the first response was not kept (larger than 512 KB, or \
            holding a secret). Its `x-idempotency` says whether the key is `required` (the \
            writes that can invoice, credit or record money refuse a request without one, \
            `400 idempotency_key_required`) or `optional`. A PATCH takes `application/json` and \
            `application/merge-patch+json` alike: one whose body is a JSON merge patch (RFC \
            7396: a field left out is unchanged, `null` clears it) declares \
            `application/merge-patch+json`, the one whose body lists changes (`PATCH \
            /rate-cards/{id_or_code}/rates`) `application/json`. Errors are RFC 9457 problems \
            (`application/problem+json`) with a specific `code`; `403 forbidden` refuses the \
            credentials, `403 feature_not_available` a feature the organization's plan or \
            edition does not include. A `401` carries `WWW-Authenticate: Bearer`. A response \
            the request rate limit counted carries `RateLimit-Limit`, `RateLimit-Remaining` and \
            `RateLimit-Reset` (Unix seconds), and the IETF draft's `RateLimit-Policy` and \
            `RateLimit`; its `429` adds `Retry-After` (seconds). Enums in responses are open: \
            new values (problem and field error codes, event types, statuses) may appear \
            without a new `Meteroid-Version`, and a client must accept a value it does not \
            know (a problem by its HTTP status, a field error as `invalid`); their values are \
            `x-extensible-enum`. Lists are paged \
            by cursor, `?limit=&after=`, as their `x-pagination` describes; a filter taking \
            several values (`status`) is the parameter repeated, \
            `?status=active&status=trial_active` (`style: form`, `explode: true`), and \
            `created_after`, `created_before` and `updated_after` are RFC 3339 moments, \
            exclusive. Prices and rates \
            are decimal strings in the currency's major unit; amounts billed are integers in \
            minor units. Requests are rate limited per tenant environment, every API key of the \
            tenant sharing the limits: the `429` response (`components.responses.TooManyRequests`) \
            states them.",
        contact(name = "Meteroid", url = "https://meteroid.com", email = "hey@meteroid.com"),
        terms_of_service = "https://meteroid.com/terms"
    ),
    external_docs(
        url = "https://docs.meteroid.com/api/v2",
        description = "The API v2 reference and guides"
    ),
    modifiers(&SecurityAddon, &WebhookSignature),
    components(schemas(
        problem::Problem,
        problem::FieldError,
        problem::FieldErrorCode,
        problem::ProblemCode,
        idempotency::IdempotentReplayMarker,
        idempotency::IdempotentReplay,
        coupons::CouponStatus,
        checkout_sessions::CheckoutStatus,
        subscriptions::model::TrialListParam,
        webhooks::DeliveryStatus,
        usage::Granularity,
        crate::webhooks::out_model::WebhookOutEventTypeEnum
    )),
    tags(
        (name = "Plans", description = "A plan is a price sheet: versions of charges in options, written through a draft and published."),
        (name = "Products", description = "What is sold. How it is billed is each charge's."),
        (name = "Product families", description = "The product lines a catalog is sorted in, each with its pricing page."),
        (name = "Managed quantities", description = "Counts a subscription holds and changes (seats), shared by the charges naming them."),
        (name = "Metrics", description = "What usage is measured by: the usage events of an event code, aggregated. Several metrics may read one event code, each its own way."),
        (name = "Rate cards", description = "Dated rates of a metric's usage by its dimensions, and their history."),
        (name = "Add-ons", description = "A charge sold beside a plan."),
        (name = "Features", description = "What a customer may use (a flag, a limit, a value), its policy and the level it applies per, granted by its defaults, plan versions, add-ons, products, subscriptions and customer overrides."),
        (name = "Entitlements", description = "Any row giving a feature a value, whoever it belongs to, and its history: an edit keeps the row's id and adds a version."),
        (name = "Resources", description = "What a customer has several of (sites, workspaces): types with identifying keys and attributes, one level below the customer. A customer's stored resources are under `/customers/{id_or_alias}/resources`."),
        (name = "Coupons", description = "Discounts and promotional credit redeemed by code: what they take off, for how long, with which plans and add-ons, and who may redeem them."),
        (name = "Credit units", description = "Denominations usage is priced and credit granted in instead of money, each with its selling price per currency."),
        (name = "Pricing pages", description = "A product family's pricing page, priced in one currency and option."),
        (name = "Subscriptions", description = "A customer on a plan: its seats, levels, charges and add-ons, changed now or at the period's end, each change previewed first; its entitlements and usage."),
        (name = "Customers", description = "Who is billed: named by id or by alias (`ext:<alias>`), with their addresses, tax and invoicing settings and portal token, and what is theirs under the customer's path: what they may use (the snapshot, the check, usage counted by your app, overrides), their resources, credit per currency or credit unit (wallets, grants, the ledger, top-ups), usage, and budgets in the Enterprise edition."),
        (name = "Custom properties", description = "The properties a tenant defines on its customers, subscriptions, invoices, credit notes and quotes, which their `custom_properties` are checked against."),
        (name = "Invoices", description = "What a customer is billed: drafts priced as usage comes in, then issued, paid, voided or written off."),
        (name = "Credit notes", description = "What an invoice gives back: to the customer's balance, in money, or as debt cancelled."),
        (name = "Events", description = "Usage events, ingested for metering and billing: 1 to 500 a request, each naming its `customer` by id or `ext:<alias>`, its properties strings, numbers or booleans."),
        (name = "Usage", description = "The tenant's usage by metric. A customer's and a subscription's are under their own paths."),
        (name = "Checkout sessions", description = "A hosted checkout of a plan: the customer pays on its page, which creates the subscription."),
        (name = "Credentials", description = "The calling credentials: their tenant and organization, the key, and what delegated credentials may reach."),
        (name = "Webhooks", description = "Endpoints Meteroid POSTs signed event payloads to, their signing secret, and their deliveries.")
    )
)]
pub struct ApiV2Doc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
    }
}

/// The Webhooks tag's paragraph on verifying a delivery ([`webhooks::SIGNATURE_VERIFICATION`]),
/// after the tag's own text.
struct WebhookSignature;

impl Modify for WebhookSignature {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for tag in openapi.tags.iter_mut().flatten() {
            if tag.name == "Webhooks" {
                let described = match tag.description.take() {
                    Some(text) => format!("{text}\n\n{}", webhooks::SIGNATURE_VERIFICATION),
                    None => webhooks::SIGNATURE_VERIFICATION.to_owned(),
                };
                tag.description = Some(described);
            }
        }
    }
}

/// Where the Connect module forwards OAuth's protocol endpoints (`/api/v2/oauth/token`…), which
/// authenticate the client in the request, not with an API key.
pub const OAUTH_PREFIX: &str = "/api/v2/oauth/";

/// Core's v2 routes, and the document with the edition's modules' operations merged in.
fn assemble(edition: &EditionSpec) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, mut open_api) = OpenApiRouter::<AppState>::with_openapi(ApiV2Doc::openapi())
        .merge(api_v2_routes())
        .split_for_parts();
    if let Some(license) = edition.api_v2.info.license.clone() {
        open_api.info.license = Some(license);
    }
    open_api.merge(edition.api_v2.clone());
    // The modules' operations come marked with their edition (`Modules::spec`); core's are oss.
    mark_operations(&mut open_api, OperationEdition::Oss);
    crate::openapi::discriminate_tagged_variants(&mut open_api);
    crate::openapi::apply_sorted_tags(&mut open_api);
    (router, open_api)
}

/// The v2 router, its POSTs idempotent, and its OpenAPI document with the edition's modules'
/// operations.
pub fn api_v2(
    state: AppState,
    edition: &EditionSpec,
) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, open_api) = assemble(edition);
    let router = router.layer(axum::middleware::from_fn_with_state(
        state,
        idempotency::idempotency,
    ));
    (router, open_api)
}

/// The v2 routes of the edition's modules, their state provided, their POSTs idempotent as
/// core's. The envelope (version, request id, problems) wraps the whole server.
pub fn module_routes(router: axum::Router, state: AppState) -> axum::Router {
    router.layer(axum::middleware::from_fn_with_state(
        state,
        idempotency::idempotency,
    ))
}

/// The v2 document of an edition, as the routes give it; [`spec_json`] finishes it.
pub fn spec(edition: &EditionSpec) -> utoipa::openapi::OpenApi {
    assemble(edition).1
}

/// A v2 document as published and served: beta operations marked as v1's, then finished
/// (`document::finish`).
pub fn spec_json(openapi: &utoipa::openapi::OpenApi) -> serde_json::Value {
    let mut value = crate::openapi::spec_json(openapi);
    if let Some(root) = value.as_object_mut() {
        document::finish(root);
    }
    value
}

/// The v2 document of an edition as published and served: [`spec_json`], with the edition's
/// webhooks in v2's payloads (`document::attach_webhooks`), the entitlement alerts v1 keeps beta
/// included.
pub fn published_spec_json(
    openapi: &utoipa::openapi::OpenApi,
    edition: &EditionSpec,
) -> serde_json::Value {
    let mut value = spec_json(openapi);
    if let Some(root) = value.as_object_mut() {
        document::attach_webhooks(root, &crate::openapi::v2_webhooks_root(edition));
    }
    value
}

/// The component names two documents give different schemas: two Rust types named alike for
/// utoipa (`ToSchema::name`), of which a merge keeps one and drops the other silently. Within one
/// document the last type registered wins unseen, so each resource's router is compared apart.
pub fn component_name_collisions(documents: &[(&str, &utoipa::openapi::OpenApi)]) -> Vec<String> {
    let mut seen: BTreeMap<String, (String, serde_json::Value)> = BTreeMap::new();
    let mut collisions = Vec::new();
    for (document, openapi) in documents {
        let Some(components) = openapi.components.as_ref() else {
            continue;
        };
        for (name, schema) in &components.schemas {
            let schema = serde_json::to_value(schema).expect("a schema serializes");
            match seen.get(name) {
                Some((first, first_schema)) if *first_schema != schema => {
                    collisions.push(format!("{name}: {first} and {document}"));
                }
                Some(_) => {}
                None => {
                    seen.insert(name.clone(), ((*document).to_owned(), schema));
                }
            }
        }
    }
    collisions
}

/// Core's v2 documents, one per resource and the shared one, compared by
/// [`component_name_collisions`] with an edition's modules'.
pub fn resource_documents() -> Vec<(&'static str, utoipa::openapi::OpenApi)> {
    let mut documents: Vec<(&'static str, utoipa::openapi::OpenApi)> = resource_routes()
        .into_iter()
        .map(|(name, routes)| (name, routes.split_for_parts().1))
        .collect();
    documents.push(("ApiV2Doc", ApiV2Doc::openapi()));
    documents.push((
        "webhook_payloads",
        webhook_payloads::WebhookPayloadsDoc::openapi(),
    ));
    documents
}

/// Refuses to publish an edition's v2 document where two types share a component name: the
/// generators (`openapi-generate`, `openapi-generate-enterprise`) fail instead.
pub fn assert_one_type_per_component(edition: &EditionSpec) {
    let mut documents = resource_documents();
    documents.push(("the edition's modules", edition.api_v2.clone()));
    let documents: Vec<(&str, &utoipa::openapi::OpenApi)> = documents
        .iter()
        .map(|(name, openapi)| (*name, openapi))
        .collect();
    let collisions = component_name_collisions(&documents);
    assert!(
        collisions.is_empty(),
        "two types share a component name of the v2 document (rename one, or \
         `#[schema(as = …)]`): {collisions:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_name_is_one_type() {
        assert_one_type_per_component(&EditionSpec::core());
    }

    #[test]
    fn a_name_two_types_give_is_a_collision() {
        use utoipa::openapi::{ComponentsBuilder, ObjectBuilder, OpenApiBuilder, Type};
        let address = |field: &str| {
            let schema = ObjectBuilder::new()
                .property(field, ObjectBuilder::new().schema_type(Type::String));
            OpenApiBuilder::new()
                .components(Some(
                    ComponentsBuilder::new().schema("Address", schema).build(),
                ))
                .build()
        };
        let (customers, documents, subscriptions) = (
            address("postal_code"),
            address("zip_code"),
            address("postal_code"),
        );
        assert_eq!(
            component_name_collisions(&[("customers", &customers), ("invoices", &documents)]),
            vec!["Address: customers and invoices".to_owned()]
        );
        assert!(
            component_name_collisions(&[
                ("customers", &customers),
                ("subscriptions", &subscriptions)
            ])
            .is_empty()
        );
    }
}

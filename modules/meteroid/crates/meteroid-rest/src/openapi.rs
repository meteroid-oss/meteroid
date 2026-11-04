use crate::invoices::model::InvoiceStatus;
use crate::metrics::model::BillingMetricAggregateEnum;
use crate::products::model::ProductFeeTypeEnum;
use crate::subscriptions::model::SubscriptionStatusEnum;
use crate::webhooks::out_model::{
    CreditGrantSource, CreditGrantType, WebhookOutAlertPeriod, WebhookOutAutoTopUpCapReachedEvent,
    WebhookOutAutoTopUpCapReachedEventData, WebhookOutAutoTopUpConsentChangedEvent,
    WebhookOutAutoTopUpConsentChangedEventData, WebhookOutAutoTopUpEvent,
    WebhookOutAutoTopUpEventData, WebhookOutBudgetAlertEvent, WebhookOutBudgetAlertEventData,
    WebhookOutCreditGrantExpiredEvent, WebhookOutCreditGrantExpiredEventData,
    WebhookOutEntitlementAlertEvent, WebhookOutEntitlementAlertEventData,
    WebhookOutTooManyGroupsEvent, WebhookOutTooManyGroupsEventData,
    WebhookOutWalletBalanceLowEvent, WebhookOutWalletBalanceLowEventData,
    WebhookOutWalletCreditRecoveredEvent, WebhookOutWalletCreditRecoveredEventData,
    WebhookOutWalletCreditUnrecoveredEvent, WebhookOutWalletCreditUnrecoveredEventData,
    WebhookOutWalletDebitedEvent, WebhookOutWalletDebitedEventData, WebhookOutWalletDepletedEvent,
    WebhookOutWalletDepletedEventData, WebhookOutWalletFundedEvent,
    WebhookOutWalletFundedEventData, WebhookOutWalletRecoveredEvent,
    WebhookOutWalletRecoveredEventData,
};
use crate::webhooks::out_model::{
    CreditNoteStatus, WebhookOutAddOnEvent, WebhookOutAddOnEventData, WebhookOutCouponEvent,
    WebhookOutCouponEventData, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEInvoicingFinding,
    WebhookOutEventGroupEnum, WebhookOutEventTypeEnum, WebhookOutInvoiceDocumentsEvent,
    WebhookOutInvoiceDocumentsEventData, WebhookOutInvoiceEvent, WebhookOutInvoiceEventData,
    WebhookOutMetricEvent, WebhookOutMetricEventData, WebhookOutPaymentEvent, WebhookOutPlanEvent,
    WebhookOutPlanEventData, WebhookOutProductEvent, WebhookOutProductEventData,
    WebhookOutQuoteEvent, WebhookOutQuoteEventData, WebhookOutRefundEventData,
    WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use crate::{AppState, api_routes};
use common_domain::ids::{
    AddOnId, BaseId, BillableMetricId, BudgetId, CouponId, CreditGrantId, CreditNoteId, CustomerId,
    EventId, FeatureId, InvoiceId, LedgerEntryId, PaymentTransactionId, PlanId, ProductFamilyId,
    ProductId, QuoteId, SubscriptionId,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::BillingPeriodEnum;
pub use meteroid_rest_common::spec::EditionSpec;
use strum::IntoEnumIterator;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

/// Builds the merged spec. Beta webhooks (and their `EventType` values) are left out; beta REST
/// operations are published, labelled as beta.
fn build_spec(edition: &EditionSpec) -> utoipa::openapi::OpenApi {
    let (_router, mut open_api) = assemble(api_routes());
    let schemas_before = referenced_schemas(&open_api);
    merge_edition(&mut open_api, edition);
    remove_beta_webhooks(&mut open_api);
    prune_schemas(&mut open_api, &schemas_before);
    add_rate_limit_responses(&mut open_api);
    discriminate_tagged_variants(&mut open_api);
    apply_sorted_tags(&mut open_api);
    label_beta_operations(&mut open_api);

    // Sort tags alphabetically by name
    if let Some(tags) = open_api.tags.as_mut() {
        tags.sort_by(|a, b| a.name.cmp(&b.name));
    }
    open_api
}

/// The REST API of an edition: the core and module routes.
pub fn assemble(
    routes: OpenApiRouter<AppState>,
) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(routes)
        .split_for_parts()
}

/// Adds the modules' operations and leaves out the webhooks no module of the edition emits; an
/// edition that names its own license publishes under it. Leaves the unreferenced schemas.
pub fn merge_edition(open_api: &mut utoipa::openapi::OpenApi, edition: &EditionSpec) {
    if let Some(license) = edition.api.info.license.clone() {
        open_api.info.license = Some(license);
    }
    open_api.merge(edition.api.clone());
    let unemitted = WebhookOutEventTypeEnum::iter()
        .filter(|event| event.needs_module())
        .map(|event| event.to_string())
        .filter(|name| !edition.webhook_events.contains(&name.as_str()))
        .collect::<Vec<_>>();
    remove_webhooks(open_api, &unemitted);
}

/// The served spec of an edition, with the schemas only the webhooks it leaves out referenced.
pub fn edition_spec(edition: &EditionSpec) -> (axum::Router<AppState>, utoipa::openapi::OpenApi) {
    let (router, mut open_api) = assemble(api_routes());
    let schemas_before = referenced_schemas(&open_api);
    merge_edition(&mut open_api, edition);
    prune_schemas(&mut open_api, &schemas_before);
    (router, open_api)
}

/// Promotes `webhooks` out of utoipa's `extensions` bucket into the OpenAPI 3.1 top-level key,
/// which is where the published file's readers expect to find it.
fn promote_webhooks_to_top_level(
    open_api: &utoipa::openapi::OpenApi,
) -> serde_json::Map<String, serde_json::Value> {
    let mut json_value: serde_json::Value = serde_json::from_str(
        &open_api
            .to_pretty_json()
            .expect("Failed to serialize OpenAPI"),
    )
    .expect("Failed to parse OpenAPI JSON");
    let webhooks = json_value
        .get_mut("extensions")
        .and_then(|ext| ext.as_object_mut())
        .and_then(|ext| ext.remove("webhooks"));
    let mut root = match json_value.take() {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("OpenApi always serializes to a JSON object"),
    };
    root.remove("extensions");
    if let Some(webhooks) = webhooks {
        root.insert("webhooks".to_string(), webhooks);
    }
    root
}

/// The published spec as a JSON object: webhooks at the top level, beta operations marked.
pub fn published_spec_root(edition: &EditionSpec) -> serde_json::Map<String, serde_json::Value> {
    let mut root = promote_webhooks_to_top_level(&build_spec(edition));
    mark_beta_operations(&mut root);
    root
}

/// Writes the specs of an edition, whose modules add `edition`, under `dir`.
pub fn generate_spec(edition: EditionSpec, dir: &str) {
    let path = format!("{dir}/api/v1/openapi.json");

    println!("Generating OpenAPI spec {path:?}");

    let mut root = published_spec_root(&edition);

    // The published key order (webhooks last) is what CI diffs against: keep it stable.
    let mut ordered = serde_json::Map::new();
    let key_order = ["openapi", "info", "paths", "components", "tags", "webhooks"];

    for key in key_order {
        if let Some(value) = root.remove(key) {
            ordered.insert(key.to_string(), value);
        }
    }

    // Add any remaining keys not in our order list
    for (key, value) in root.iter() {
        ordered.insert(key.clone(), value.clone());
    }

    let output = serde_json::to_string_pretty(&serde_json::Value::Object(ordered))
        .expect("Failed to serialize OpenAPI");

    std::fs::create_dir_all(format!("{dir}/api/v1")).expect("Unable to create the spec directory");
    std::fs::write(path, output).expect("Unable to write openapi.json file");

    let client_path = format!("{dir}/client/v1/openapi.json");
    println!("Generating OpenAPI spec {client_path:?}");
    let (_router, client_api) = crate::client::client_api(edition.client);
    std::fs::create_dir_all(format!("{dir}/client/v1")).expect("Unable to create spec/client/v1");
    std::fs::write(
        client_path,
        serde_json::to_string_pretty(&spec_json(&client_api))
            .expect("Failed to serialize client OpenAPI"),
    )
    .expect("Unable to write client openapi.json file");
}

#[derive(OpenApi)]
#[openapi(
    // The published title, set here since this crate is no longer `meteroid`.
    info(title = "meteroid"),
    modifiers(&SecurityAddon, &WebhooksAddon),
    components(
        schemas(
            WebhookOutCustomerEvent,
        WebhookOutInvoiceEvent,
        WebhookOutInvoiceDocumentsEvent,
        WebhookOutSubscriptionEvent,
        WebhookOutMetricEvent,
        WebhookOutQuoteEvent,
        WebhookOutCreditNoteEvent,
        WebhookOutPlanEvent,
        WebhookOutProductEvent,
        WebhookOutCouponEvent,
        WebhookOutAddOnEvent,
        WebhookOutPaymentEvent,
        WebhookOutWalletFundedEvent,
        WebhookOutWalletDebitedEvent,
        WebhookOutCreditGrantExpiredEvent,
        WebhookOutWalletBalanceLowEvent,
        WebhookOutWalletDepletedEvent,
        WebhookOutWalletRecoveredEvent,
        WebhookOutWalletCreditUnrecoveredEvent,
        WebhookOutWalletCreditRecoveredEvent,
        WebhookOutAutoTopUpEvent,
        WebhookOutAutoTopUpConsentChangedEvent,
        WebhookOutAutoTopUpCapReachedEvent,
        WebhookOutEntitlementAlertEvent,
        WebhookOutBudgetAlertEvent,
        WebhookOutTooManyGroupsEvent,
      )
    ),
    tags(
        (name = "Meteroid", description = "Meteroid API"),
        (name = "Webhook Endpoints", description = "Manage outbound webhook endpoints and inspect their deliveries"),
        (name = "Credits", description = "Prepaid credit: a customer's wallets, credit grants, ledger and top-ups. Promotional credit applies before tax like a discount; paid credit applies after tax like a payment."),
        (name = "Credit units", description = "Custom denominations, such as \"credits\", that usage can be priced in and credit granted or sold in, instead of money."),
        (name = "Licenses", description = "Licenses for your customers' self-hosted installations: signed tokens carrying their plan's entitlements, activated and refreshed by the installation over the public licensing endpoints.")
    )
)]
pub struct ApiDoc;

/// The browser-facing API under `/api/client/v1`, published as its own document.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Meteroid Client API",
        description = "Browser-facing API, authenticated with a customer portal token."
    ),
    modifiers(&ClientSecurityAddon)
)]
pub struct ClientApiDoc;

struct SecurityAddon;

struct ClientSecurityAddon;

struct WebhooksAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
        }
    }
}

impl Modify for ClientSecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "customer_token",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some(
                            "Customer portal token, minted by your backend with \
                             `POST /api/v1/customers/{id_or_alias}/portal-token`. \
                             It needs the `read` scope.",
                        ))
                        .build(),
                ),
            );
    }
}

/// Tags of routes published as beta, with their tag description: their shape may still change,
/// so each operation says so and carries `x-beta: true` for SDK generators. Their webhooks stay
/// out of the spec (`WebhookOutEventGroupEnum::is_beta`).
const BETA_TAGS: &[(&str, &str)] = &[
    (
        "Budgets",
        "Beta: may change before general availability. Spending limits on a customer's usage, or one subscription's, per currency, with alerts at percentages of the amount.",
    ),
    (
        "Checks",
        "Beta: may change before general availability. Whether a customer may consume more of a feature right now.",
    ),
    (
        "Usage costs",
        "Beta: may change before general availability. A customer's or a subscription's usage events and usage over time, with what they cost.",
    ),
];

const BETA_NOTICE: &str = "Beta: the request and response shapes of this endpoint may change before general availability.";

fn is_beta_tag(name: &str) -> bool {
    BETA_TAGS.iter().any(|(beta, _)| *beta == name)
}

/// Says in each beta operation's description, and in its tag's, that its shape may change. Runs
/// after `apply_sorted_tags`, which lists the tags the operations use.
pub fn label_beta_operations(openapi: &mut utoipa::openapi::OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        for op in [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.options.as_mut(),
            item.head.as_mut(),
            item.patch.as_mut(),
            item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let beta = op
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| is_beta_tag(tag)));
            if beta
                && !op
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .contains(BETA_NOTICE)
            {
                op.description = Some(match op.description.take() {
                    Some(description) if !description.trim().is_empty() => {
                        format!("{description}\n\n{BETA_NOTICE}")
                    }
                    _ => BETA_NOTICE.to_owned(),
                });
            }
        }
    }
    for tag in openapi.tags.iter_mut().flatten() {
        if let Some((_, description)) = BETA_TAGS.iter().find(|(name, _)| *name == tag.name) {
            tag.description = Some((*description).to_owned());
        }
    }
}

/// Marks every beta operation, and its tag, with the `x-beta: true` extension.
fn mark_beta_operations(root: &mut serde_json::Map<String, serde_json::Value>) {
    let is_beta = |op: &serde_json::Map<String, serde_json::Value>| {
        op.get("tags")
            .and_then(|tags| tags.as_array())
            .is_some_and(|tags| tags.iter().filter_map(|t| t.as_str()).any(is_beta_tag))
    };
    if let Some(paths) = root.get_mut("paths").and_then(|p| p.as_object_mut()) {
        for item in paths.values_mut().filter_map(|item| item.as_object_mut()) {
            for op in item.values_mut().filter_map(|op| op.as_object_mut()) {
                if is_beta(op) {
                    op.insert("x-beta".to_owned(), serde_json::Value::Bool(true));
                }
            }
        }
    }
    if let Some(tags) = root.get_mut("tags").and_then(|t| t.as_array_mut()) {
        for tag in tags.iter_mut().filter_map(|tag| tag.as_object_mut()) {
            if tag
                .get("name")
                .and_then(|n| n.as_str())
                .is_some_and(is_beta_tag)
            {
                tag.insert("x-beta".to_owned(), serde_json::Value::Bool(true));
            }
        }
    }
}

/// A served spec as JSON, beta operations marked like the published one.
pub fn spec_json(openapi: &utoipa::openapi::OpenApi) -> serde_json::Value {
    let mut value = serde_json::to_value(openapi).expect("Failed to serialize OpenAPI");
    if let Some(root) = value.as_object_mut() {
        mark_beta_operations(root);
    }
    value
}

fn remove_beta_webhooks(openapi: &mut utoipa::openapi::OpenApi) {
    let beta = WebhookOutEventTypeEnum::iter()
        .filter(|event| event.group().is_beta())
        .map(|event| event.to_string())
        .collect::<Vec<_>>();
    remove_webhooks(openapi, &beta);
}

/// Drops the named webhooks and their `EventType` values.
fn remove_webhooks(openapi: &mut utoipa::openapi::OpenApi, names: &[String]) {
    use utoipa::openapi::{RefOr, Schema};

    if names.is_empty() {
        return;
    }
    if let Some(webhooks) = openapi
        .extensions
        .as_mut()
        .and_then(|extensions| extensions.get_mut("webhooks"))
        .and_then(serde_json::Value::as_object_mut)
    {
        webhooks.retain(|name, _| !names.contains(name));
    }
    if let Some(RefOr::T(Schema::Object(event_type))) = openapi
        .components
        .as_mut()
        .and_then(|components| components.schemas.get_mut("EventType"))
        && let Some(values) = event_type.enum_values.as_mut()
    {
        values.retain(|value| {
            !value
                .as_str()
                .is_some_and(|name| names.iter().any(|n| n == name))
        });
    }
}

/// Schema names reachable through `$ref` from anything outside `components.schemas`.
fn referenced_schemas(openapi: &utoipa::openapi::OpenApi) -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;

    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, v) in map {
                    match (key.as_str(), v) {
                        ("$ref", serde_json::Value::String(r)) => {
                            if let Some(name) = r.strip_prefix("#/components/schemas/") {
                                out.push(name.to_string());
                            }
                        }
                        _ => collect(v, out),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
            _ => {}
        }
    }

    let mut root = serde_json::to_value(openapi).expect("Failed to serialize OpenAPI");
    let schemas = root
        .get_mut("components")
        .and_then(|c| c.as_object_mut())
        .and_then(|c| c.remove("schemas"))
        .unwrap_or_default();

    let mut pending = Vec::new();
    collect(&root, &mut pending);
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if seen.insert(name.clone())
            && let Some(schema) = schemas.get(&name)
        {
            collect(schema, &mut pending);
        }
    }
    seen
}

/// Drops the schemas that were referenced before beta operations were removed and no longer are.
fn prune_schemas(
    openapi: &mut utoipa::openapi::OpenApi,
    before: &std::collections::BTreeSet<String>,
) {
    let after = referenced_schemas(openapi);
    if let Some(components) = openapi.components.as_mut() {
        components
            .schemas
            .retain(|name, _| after.contains(name) || !before.contains(name));
    }
}

/// Collects all tags used across path operations and sets the top-level `tags`
/// array to a sorted, deduplicated list. Call this **after** routes have been merged.
pub fn apply_sorted_tags(openapi: &mut utoipa::openapi::OpenApi) {
    use std::collections::BTreeSet;
    use utoipa::openapi::tag::TagBuilder;

    // Index existing tags by name so we can preserve their descriptions
    let mut existing: std::collections::HashMap<String, utoipa::openapi::Tag> = openapi
        .tags
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.name.clone(), t))
        .collect();

    let mut tag_names = BTreeSet::new();
    for tag in existing.keys() {
        tag_names.insert(tag.clone());
    }
    for (_, path_item) in openapi.paths.paths.iter() {
        for op in [
            path_item.get.as_ref(),
            path_item.put.as_ref(),
            path_item.post.as_ref(),
            path_item.delete.as_ref(),
            path_item.options.as_ref(),
            path_item.head.as_ref(),
            path_item.patch.as_ref(),
            path_item.trace.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(tags) = &op.tags {
                for tag in tags {
                    tag_names.insert(tag.clone());
                }
            }
        }
    }

    let mut sorted_tags: Vec<utoipa::openapi::Tag> = tag_names
        .into_iter()
        .map(|name| {
            existing
                .remove(&name)
                .unwrap_or_else(|| TagBuilder::new().name(name).build())
        })
        .collect();

    // todo do we need Meteroid tag?
    // Pin "Meteroid" as the first entry
    if let Some(pos) = sorted_tags.iter().position(|t| t.name == "Meteroid") {
        let meteroid = sorted_tags.remove(pos);
        sorted_tags.insert(0, meteroid);
    }

    openapi.tags = Some(sorted_tags);
}

/// Schemas `$ref`'d anywhere other than as a member of a discriminated union's `oneOf`.
fn standalone_schemas(spec: &serde_json::Value) -> std::collections::BTreeSet<String> {
    fn collect(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                let discriminated = map.contains_key("discriminator");
                for (key, v) in map {
                    match (key.as_str(), v) {
                        ("$ref", serde_json::Value::String(r)) => {
                            if let Some(name) = r.strip_prefix("#/components/schemas/") {
                                out.insert(name.to_owned());
                            }
                        }
                        ("oneOf", serde_json::Value::Array(members)) if discriminated => {
                            for member in members.iter().filter_map(|m| m.as_object()) {
                                member
                                    .iter()
                                    .filter(|(k, _)| k.as_str() != "$ref")
                                    .for_each(|(_, v)| collect(v, out));
                            }
                        }
                        _ => collect(v, out),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
            _ => {}
        }
    }
    let mut out = std::collections::BTreeSet::new();
    collect(spec, &mut out);
    out
}

/// Declares the tag of every mapped discriminated union on each variant as a required
/// single-value enum, so a payload matches one variant only, as JSON Schema `oneOf` requires.
/// A variant also used outside its union is serialized there without the tag, so it gets none.
pub fn discriminate_tagged_variants(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::RefOr;
    use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};

    let standalone =
        standalone_schemas(&serde_json::to_value(&*openapi).expect("the spec serializes"));
    let Some(components) = openapi.components.as_mut() else {
        return;
    };
    let tags: Vec<(String, String, String)> = components
        .schemas
        .values()
        .filter_map(|schema| match schema {
            RefOr::T(Schema::OneOf(one_of)) => one_of.discriminator.as_ref(),
            _ => None,
        })
        .flat_map(|discriminator| {
            discriminator.mapping.iter().map(|(tag, reference)| {
                let variant = reference.rsplit('/').next().unwrap_or(reference);
                (
                    variant.to_owned(),
                    discriminator.property_name.clone(),
                    tag.clone(),
                )
            })
        })
        .filter(|(variant, _, _)| !standalone.contains(variant))
        .collect();
    for (variant, property, tag) in tags {
        if let Some(RefOr::T(Schema::Object(object))) = components.schemas.get_mut(&variant) {
            if !object.required.contains(&property) {
                object.required.push(property.clone());
            }
            object.properties.entry(property).or_insert_with(|| {
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .enum_values(Some([tag]))
                    .into()
            });
        }
    }
}

/// Adds a `429 Too Many Requests` response to every operation whose path starts
/// with `/api/`. Call this **after** routes have been merged into the `OpenApi`
/// object (i.e. after `split_for_parts()`), because utoipa modifiers run before
/// routes are incorporated and would see an empty path list.
pub fn add_rate_limit_responses(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::content::ContentBuilder;
    use utoipa::openapi::response::ResponseBuilder;
    use utoipa::openapi::{Ref, RefOr};

    let rate_limited_response = RefOr::T(
        ResponseBuilder::new()
            .description("Too many requests")
            .content(
                "application/json",
                ContentBuilder::new()
                    .schema(Some(RefOr::Ref(Ref::from_schema_name("RestErrorResponse"))))
                    .build(),
            )
            .build(),
    );

    for (path, path_item) in openapi.paths.paths.iter_mut() {
        if !path.starts_with("/api/") {
            continue;
        }

        for operation in [
            path_item.get.as_mut(),
            path_item.put.as_mut(),
            path_item.post.as_mut(),
            path_item.delete.as_mut(),
            path_item.options.as_mut(),
            path_item.head.as_mut(),
            path_item.patch.as_mut(),
            path_item.trace.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            operation
                .responses
                .responses
                .entry("429".to_string())
                .or_insert_with(|| rate_limited_response.clone());
        }
    }
}

impl Modify for WebhooksAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use serde_json::json;
        use std::collections::BTreeMap;

        // Build webhook definitions
        let mut webhooks = BTreeMap::new();

        for event in WebhookOutEventTypeEnum::iter() {
            let event_name = event.to_string();
            let description = event.description();
            let group = event.group();

            let example: serde_json::Value = match group {
                WebhookOutEventGroupEnum::Customer => {
                    let event = WebhookOutCustomerEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCustomerEventData {
                            customer_id: CustomerId::default(),
                            name: "Acme Corporation".to_string(),
                            alias: Some("ACME".to_string()),
                            billing_email: Some("billing@acme.example".to_string()),
                            invoicing_emails: vec!["invoices@acme.example".to_string()],
                            phone: Some("+1-555-0123".to_string()),
                            currency: "USD".to_string(),
                            custom_properties: serde_json::json!({ "account_tier": "enterprise" }),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Subscription => {
                    let event = WebhookOutSubscriptionEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutSubscriptionEventData {
                            subscription_id: SubscriptionId::default(),
                            customer_id: Default::default(),
                            customer_alias: Some("ACME".to_string()),
                            customer_name: "Acme Corporation".to_string(),
                            billing_day_anchor: 1,
                            currency: "EUR".to_string(),
                            trial_duration: None,
                            start_date: Default::default(),
                            end_date: None,
                            billing_start_date: None,
                            plan_name: "default".to_string(),
                            version: 0,
                            created_at: Default::default(),
                            net_terms: 0,
                            invoice_memo: None,
                            invoice_threshold: None,
                            activated_at: None,
                            mrr_cents: 0,
                            period: BillingPeriodEnum::Monthly,
                            status: SubscriptionStatusEnum::PendingActivation,
                            auto_advance_invoices: true,
                            charge_automatically: true,
                            purchase_order: None,
                            change_type: None,
                            cancellation_reason: None,
                            custom_properties: serde_json::json!({ "pdp_external_status": "approved" }),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Invoice => {
                    let event = WebhookOutInvoiceEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutInvoiceEventData {
                            invoice_id: InvoiceId::default(),
                            invoice_number: Some("INV-0001".to_string()),
                            customer_id: Default::default(),
                            status: InvoiceStatus::Draft,
                            currency: "EUR".to_string(),
                            total: 10000,
                            tax_amount: 2000,
                            created_at: Default::default(),
                            consolidated_into_invoice_id: None,
                            parent_invoice_id: None,
                            custom_properties: serde_json::json!({}),
                            settled_by_credit_notes: false,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::InvoiceDocuments => {
                    let event = WebhookOutInvoiceDocumentsEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutInvoiceDocumentsEventData {
                            invoice_id: InvoiceId::default(),
                            customer_id: CustomerId::default(),
                            pdf_document_id: "doc_01HXYZ".to_string(),
                            xml_document_id: None,
                            einvoicing_status: Some(
                                crate::invoices::model::EInvoicingStatus::Failed,
                            ),
                            einvoicing_profile: Some("EN 16931".to_string()),
                            einvoicing_findings: vec![WebhookOutEInvoicingFinding {
                                rule: "BR-11".to_string(),
                                term: "BG-8/BT-55".to_string(),
                                message:
                                    "The Buyer postal address shall contain a Buyer country code."
                                        .to_string(),
                                hint: None,
                            }],
                            einvoicing_error: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::BillableMetric => {
                    let event = WebhookOutMetricEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutMetricEventData {
                            metric_id: BillableMetricId::default(),
                            name: "Api Calls".to_string(),
                            description: None,
                            code: "api_calls".to_string(),
                            aggregation_type: BillingMetricAggregateEnum::Count,
                            aggregation_key: None,
                            unit_conversion_factor: None,
                            unit_conversion_rounding: None,
                            segmentation_matrix: None,
                            usage_group_key: None,
                            unit_singular: None,
                            unit_plural: None,
                            created_at: Default::default(),
                            product_family_id: Default::default(),
                            product_id: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Quote => {
                    let event = WebhookOutQuoteEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutQuoteEventData {
                            quote_id: QuoteId::default(),
                            customer_id: Default::default(),
                            subscription_id: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::CreditNote => {
                    let event = WebhookOutCreditNoteEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCreditNoteEventData {
                            credit_note_id: CreditNoteId::default(),
                            credit_note_number: Some("CN-000001".to_string()),
                            customer_id: Default::default(),
                            invoice_id: Default::default(),
                            invoice_number: Some("INV-0001".to_string()),
                            status: CreditNoteStatus::Draft,
                            currency: "EUR".to_string(),
                            subtotal: 4000,
                            total: 5000,
                            tax_amount: 1000,
                            refunded_amount_cents: 0,
                            credited_amount_cents: 5000,
                            line_items: vec![],
                            tax_breakdown: vec![],
                            reason: Some("Partial refund".to_string()),
                            memo: None,
                            custom_properties: serde_json::json!({}),
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Plan => {
                    let event = WebhookOutPlanEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutPlanEventData {
                            plan_id: PlanId::default(),
                            name: "Enterprise Plan".to_string(),
                            description: Some("Our enterprise tier".to_string()),
                            plan_type: crate::plans::model::PlanTypeEnum::Standard,
                            status: crate::plans::model::PlanStatusEnum::Active,
                            currency: "USD".to_string(),
                            version: 1,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Product => {
                    let event = WebhookOutProductEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutProductEventData {
                            product_id: ProductId::default(),
                            name: "API Calls Product".to_string(),
                            description: Some("Usage-based API calls product".to_string()),
                            fee_type: ProductFeeTypeEnum::Usage,
                            product_family_id: ProductFamilyId::default(),
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Coupon => {
                    let event = WebhookOutCouponEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutCouponEventData {
                            coupon_id: CouponId::default(),
                            code: "WELCOME20".to_string(),
                            description: "20% off for new customers".to_string(),
                            discount: crate::coupons::model::CouponDiscount::Percentage(
                                crate::coupons::model::PercentageDiscount {
                                    percentage: "20".to_string(),
                                },
                            ),
                            expires_at: None,
                            redemption_limit: Some(100),
                            recurring_value: Some(3),
                            reusable: false,
                            disabled: false,
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::AddOn => {
                    let event = WebhookOutAddOnEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutAddOnEventData {
                            add_on_id: AddOnId::default(),
                            name: "Extra Storage".to_string(),
                            description: Some("Additional storage capacity".to_string()),
                            product_id: ProductId::default(),
                            price_id: common_domain::ids::PriceId::default(),
                            fee_type: Some(ProductFeeTypeEnum::Usage),
                            self_serviceable: true,
                            max_instances_per_subscription: Some(5),
                            created_at: Default::default(),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Payment
                    if matches!(event, WebhookOutEventTypeEnum::PaymentFailed) =>
                {
                    let event = WebhookOutPaymentEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutRefundEventData {
                            transaction_id: PaymentTransactionId::default(),
                            invoice_id: Some(InvoiceId::default()),
                            provider_transaction_id: Some("pi_3Q1abcDEF".to_string()),
                            amount: 2500,
                            currency: "EUR".to_string(),
                            status: crate::invoices::model::PaymentStatusEnum::Failed,
                            payment_type: crate::invoices::model::PaymentTypeEnum::Payment,
                            error: Some("Your card has expired.".to_string()),
                            decline_kind: Some(crate::invoices::model::DeclineKind::CardExpired),
                            processed_at: None,
                            amount_refunded: 0,
                            amount_reversed: 0,
                            amount_disputed: 0,
                            amount_reversed_excess: 0,
                            reversal_kind: None,
                            parent_transaction_id: None,
                            credit_note_id: None,
                            refund_mode: None,
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Payment => {
                    let event = WebhookOutPaymentEvent {
                        id: EventId::default(),
                        event_type: event,
                        data: WebhookOutRefundEventData {
                            transaction_id: PaymentTransactionId::default(),
                            invoice_id: Some(InvoiceId::default()),
                            provider_transaction_id: Some("re_3Q1abcDEF".to_string()),
                            amount: 2500,
                            currency: "EUR".to_string(),
                            status: crate::invoices::model::PaymentStatusEnum::Settled,
                            payment_type: crate::invoices::model::PaymentTypeEnum::Refund,
                            error: None,
                            decline_kind: None,
                            processed_at: Some(Default::default()),
                            amount_refunded: 0,
                            amount_reversed: 0,
                            amount_disputed: 0,
                            amount_reversed_excess: 0,
                            reversal_kind: None,
                            parent_transaction_id: Some(PaymentTransactionId::default()),
                            credit_note_id: Some(CreditNoteId::default()),
                            refund_mode: Some(crate::invoices::model::RefundMode::Online),
                        },
                        timestamp: Default::default(),
                    };
                    serde_json::to_value(&event).expect("Failed to serialize webhook example")
                }
                WebhookOutEventGroupEnum::Wallet | WebhookOutEventGroupEnum::CreditGrant => {
                    wallet_example(event)
                }
                WebhookOutEventGroupEnum::Entitlement | WebhookOutEventGroupEnum::Budget => {
                    alert_example(event)
                }
            };

            let webhook_def = json!({
                "post": {
                    "summary": format!("{} webhook", event_name),
                    "description": description,
                    "operationId": format!("webhook_{}", event_name.replace('.', "_")),
                    "tags": ["webhooks"],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}", event.schema_name())
                                },
                                "example": example
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Webhook received successfully"
                        }
                    }
                }
            });

            webhooks.insert(event_name, webhook_def);
        }

        // Add webhooks extension
        if openapi.extensions.is_none() {
            openapi.extensions = Some(Default::default());
        }

        if let Some(extensions) = openapi.extensions.as_mut() {
            extensions.insert("webhooks".to_string(), json!(webhooks));
        }
    }
}

fn wallet_example(event: WebhookOutEventTypeEnum) -> serde_json::Value {
    let changed_at = ApiDateTime::default();
    let example = match event {
        WebhookOutEventTypeEnum::WalletFunded => {
            serde_json::to_value(WebhookOutWalletFundedEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletFundedEventData {
                    customer_id: CustomerId::default(),
                    applies_to: crate::credits::model::CreditAppliesTo::All,
                    subscription_id: None,
                    applicable_product_ids: Vec::new(),
                    ledger_entry_id: LedgerEntryId::default(),
                    credit_grant_id: CreditGrantId::default(),
                    invoice_id: Some(InvoiceId::default()),
                    credit_note_id: None,
                    amount: 5000,
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    source: CreditGrantSource::Purchase,
                    grant_type: CreditGrantType::Paid,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletDebited => {
            serde_json::to_value(WebhookOutWalletDebitedEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletDebitedEventData {
                    customer_id: CustomerId::default(),
                    applies_to: crate::credits::model::CreditAppliesTo::All,
                    subscription_id: None,
                    applicable_product_ids: Vec::new(),
                    ledger_entry_id: LedgerEntryId::default(),
                    credit_grant_id: CreditGrantId::default(),
                    invoice_id: None,
                    credit_note_id: None,
                    amount: 1250,
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    reason: "Credit removed by hand".to_string(),
                    actor: Some(crate::webhooks::out_model::WebhookOutActor {
                        actor_type: crate::webhooks::out_model::WebhookOutActorType::User,
                        id: Some(common_domain::ids::UserId::default().as_base62()),
                    }),
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletCreditUnrecovered => {
            serde_json::to_value(WebhookOutWalletCreditUnrecoveredEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletCreditUnrecoveredEventData {
                    customer_id: CustomerId::default(),
                    invoice_id: Some(InvoiceId::default()),
                    payment_id: Some(PaymentTransactionId::default()),
                    credit_note_id: None,
                    cause: "PAYMENT_DISPUTED".to_string(),
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    credit_taken_back: 3500,
                    credit_unrecovered: 1500,
                    credit_unrecovered_total: 1500,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletCreditRecovered => {
            serde_json::to_value(WebhookOutWalletCreditRecoveredEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletCreditRecoveredEventData {
                    customer_id: CustomerId::default(),
                    invoice_id: Some(InvoiceId::default()),
                    payment_id: Some(PaymentTransactionId::default()),
                    credit_note_id: None,
                    cause: "PAYMENT_DISPUTED".to_string(),
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    credit_recovered: 1500,
                    credit_unrecovered_total: 0,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletBalanceLow => {
            serde_json::to_value(WebhookOutWalletBalanceLowEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletBalanceLowEventData {
                    customer_id: CustomerId::default(),
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    balance: 800,
                    threshold: 1000,
                    shared_balance: Some(300),
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletDepleted => {
            serde_json::to_value(WebhookOutWalletDepletedEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletDepletedEventData {
                    customer_id: CustomerId::default(),
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::WalletRecovered => {
            serde_json::to_value(WebhookOutWalletRecoveredEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutWalletRecoveredEventData {
                    customer_id: CustomerId::default(),
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    balance: 500,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::AutoTopUpCapReached => {
            serde_json::to_value(WebhookOutAutoTopUpCapReachedEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutAutoTopUpCapReachedEventData {
                    customer_id: CustomerId::default(),
                    currency: "EUR".to_string(),
                    period: "MONTHLY".to_string(),
                    cap: 150_000,
                    attempted_amount: 50_000,
                    period_start: changed_at,
                    period_end: changed_at,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::AutoTopUpConsentChanged => {
            serde_json::to_value(WebhookOutAutoTopUpConsentChangedEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutAutoTopUpConsentChangedEventData {
                    currency: "EUR".to_owned(),
                    customer_id: CustomerId::default(),
                    enabled: true,
                    previous_configuration_revision: None,
                    configuration_revision: Some(
                        "3f0c9a4be1d27f5c8e6a0b9d4c2f1e7a5b8d6c3e9f0a1b2c4d5e6f7a8b9c0d1e"
                            .to_string(),
                    ),
                    actor: crate::webhooks::out_model::WebhookOutActor {
                        actor_type: crate::webhooks::out_model::WebhookOutActorType::Customer,
                        id: Some(CustomerId::default().as_base62()),
                    },
                    changed_at,
                    consent_attested_by: None,
                    consent_attested_at: None,
                },
                timestamp: Default::default(),
            })
        }
        WebhookOutEventTypeEnum::CreditGrantExpired => {
            serde_json::to_value(WebhookOutCreditGrantExpiredEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutCreditGrantExpiredEventData {
                    customer_id: CustomerId::default(),
                    applies_to: crate::credits::model::CreditAppliesTo::Usage,
                    subscription_id: Some(common_domain::ids::SubscriptionId::default()),
                    applicable_product_ids: Vec::new(),
                    reason: crate::webhooks::out_model::CreditGrantLapseReason::SubscriptionEnded,
                    credit_grant_id: CreditGrantId::default(),
                    ledger_entry_id: LedgerEntryId::default(),
                    expired_amount: 300,
                    currency: Some("EUR".to_string()),
                    credit_unit: None,
                    credit_unit_precision: None,
                    expired_at: changed_at,
                },
                timestamp: Default::default(),
            })
        }
        _ => {
            let suspended = event == WebhookOutEventTypeEnum::AutoTopUpSuspended;
            serde_json::to_value(WebhookOutAutoTopUpEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutAutoTopUpEventData {
                    customer_id: CustomerId::default(),
                    currency: "EUR".to_string(),
                    amount: 5000,
                    consecutive_failures: if suspended { 3 } else { 1 },
                    suspended_at: suspended.then_some(changed_at),
                    error:
                        "stripe rejected: Your card was declined. (code=Some(\"card_declined\"))"
                            .to_string(),
                },
                timestamp: Default::default(),
            })
        }
    };
    example.expect("Failed to serialize webhook example")
}

fn alert_example(event: WebhookOutEventTypeEnum) -> serde_json::Value {
    let period = || WebhookOutAlertPeriod {
        start: Default::default(),
        end: Some(Default::default()),
    };
    let as_of = ApiDateTime::default();
    let reached = matches!(
        event,
        WebhookOutEventTypeEnum::EntitlementLimitReached | WebhookOutEventTypeEnum::BudgetExceeded
    );
    if event == WebhookOutEventTypeEnum::EntitlementTooManyGroups {
        return serde_json::to_value(WebhookOutTooManyGroupsEvent {
            id: EventId::default(),
            event_type: event,
            data: WebhookOutTooManyGroupsEventData {
                customer_id: CustomerId::default(),
                customer_alias: Some("acme".to_string()),
                feature_id: Some(FeatureId::default()),
                subscription_id: Some(SubscriptionId::default()),
                metric_code: "api_calls".to_string(),
                group_by: "api_key".to_string(),
                max_groups: 50_000,
                period: period(),
                as_of,
            },
            timestamp: Default::default(),
        })
        .expect("Failed to serialize webhook example");
    }
    let threshold_pct = if reached { 100 } else { 80 };
    let used = rust_decimal::Decimal::new(if reached { 1000 } else { 800 }, 0);
    let example = match event {
        WebhookOutEventTypeEnum::EntitlementUsageThresholdReached
        | WebhookOutEventTypeEnum::EntitlementLimitReached => {
            serde_json::to_value(WebhookOutEntitlementAlertEvent {
                id: EventId::default(),
                event_type: event,
                data: WebhookOutEntitlementAlertEventData {
                    customer_id: CustomerId::default(),
                    customer_alias: Some("acme".to_string()),
                    feature_id: FeatureId::default(),
                    subscription_id: Some(SubscriptionId::default()),
                    metric_code: "api_calls".to_string(),
                    group: Some([("region".to_string(), "eu".to_string())].into()),
                    threshold_pct,
                    used,
                    limit: rust_decimal::Decimal::new(1000, 0),
                    period: period(),
                    as_of,
                    exact: false,
                },
                timestamp: Default::default(),
            })
        }
        _ => serde_json::to_value(WebhookOutBudgetAlertEvent {
            id: EventId::default(),
            event_type: event,
            data: WebhookOutBudgetAlertEventData {
                customer_id: CustomerId::default(),
                customer_alias: Some("acme".to_string()),
                budget_id: BudgetId::default(),
                subscription_id: None,
                currency: Some("EUR".to_string()),
                credit_unit: None,
                credit_unit_precision: None,
                threshold_pct,
                used: if reached { 100_000 } else { 80_000 },
                amount: 100_000,
                period: period(),
                as_of,
                exact: false,
            },
            timestamp: Default::default(),
        }),
    };
    example.expect("Failed to serialize webhook example")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A union's schema name, its serde tag, and, when it deserializes, how it refuses `{}`.
    type SerdeTag = (&'static str, &'static str, Option<fn() -> Option<String>>);

    /// The serde tag of every mapped discriminated union of both specs. A union that also
    /// deserializes is checked to refuse an untagged payload; the others are response-only, and an
    /// internally tagged enum always writes its tag.
    fn serde_tags() -> Vec<SerdeTag> {
        use crate::{coupons, entitlements, metrics, plans, products, subscriptions};
        fn untagged<T: serde::de::DeserializeOwned>() -> Option<String> {
            serde_json::from_value::<T>(serde_json::json!({}))
                .err()
                .map(|error| error.to_string())
        }
        vec![
            (
                "ConfigValue",
                "kind",
                Some(untagged::<entitlements::model::ConfigValue>),
            ),
            (
                "CouponDiscount",
                "type",
                Some(untagged::<coupons::model::CouponDiscount>),
            ),
            (
                "CreditExhaustedRule",
                "type",
                Some(untagged::<plans::model::CreditExhaustedRule>),
            ),
            ("EffectiveEntitlementValue", "type", None),
            (
                "EntitlementValue",
                "type",
                Some(untagged::<entitlements::model::EntitlementValue>),
            ),
            (
                "FeatureType",
                "type",
                Some(untagged::<entitlements::model::FeatureType>),
            ),
            ("Fee", "type", Some(untagged::<plans::model::Fee>)),
            (
                "MetricSegmentationMatrix",
                "type",
                Some(untagged::<metrics::model::MetricSegmentationMatrix>),
            ),
            (
                "MinimumCommitmentInputScope",
                "type",
                Some(untagged::<plans::model::MinimumCommitmentInputScope>),
            ),
            (
                "MinimumCommitmentScope",
                "type",
                Some(untagged::<plans::model::MinimumCommitmentScope>),
            ),
            (
                "PaymentMethodsConfig",
                "type",
                Some(untagged::<subscriptions::model::PaymentMethodsConfig>),
            ),
            (
                "PlanUsagePricingModel",
                "type",
                Some(untagged::<plans::model::UsagePricingModel>),
            ),
            (
                "PriceEntry",
                "type",
                Some(untagged::<subscriptions::model::PriceEntry>),
            ),
            (
                "Pricing",
                "type",
                Some(untagged::<subscriptions::model::Pricing>),
            ),
            (
                "ProductFeeStructure",
                "type",
                Some(untagged::<products::model::ProductFeeStructure>),
            ),
            (
                "ProductRef",
                "type",
                Some(untagged::<subscriptions::model::ProductRef>),
            ),
            (
                "ResetPeriod",
                "type",
                Some(untagged::<entitlements::model::ResetPeriod>),
            ),
            ("ResolvedEntitlementValue", "type", None),
            (
                "SubscriptionAddOnCustomization",
                "type",
                Some(untagged::<subscriptions::model::SubscriptionAddOnCustomization>),
            ),
            (
                "SubscriptionFee",
                "type",
                Some(untagged::<subscriptions::model::SubscriptionFee>),
            ),
            (
                "UsagePricingModel",
                "type",
                Some(untagged::<subscriptions::model::UsagePricingModel>),
            ),
        ]
    }

    /// In every mapped discriminated union of both specs, each variant declares its tag as a
    /// single-value enum, so the variants are pairwise disjoint, and requires it as serde does.
    #[test]
    fn discriminated_union_variants_are_disjoint_and_tagged_as_serde() {
        let tags = serde_tags();
        for (union, tag, untagged) in &tags {
            if let Some(untagged) = untagged {
                let error = untagged().unwrap_or_else(|| panic!("{union} accepts no tag"));
                assert!(
                    error.contains(&format!("missing field `{tag}`")),
                    "{union}: {error}"
                );
            }
        }
        let published = serde_json::Value::Object(promote_webhooks_to_top_level(&build_spec(
            &EditionSpec::core(),
        )));
        let client = serde_json::to_value(crate::client::client_api(EditionSpec::core().client).1)
            .expect("the client spec serializes");
        for spec in [&published, &client] {
            let standalone = standalone_schemas(spec);
            let schemas = spec["components"]["schemas"]
                .as_object()
                .expect("components.schemas");
            for (union, schema) in schemas {
                let Some(discriminator) = schema.get("discriminator") else {
                    continue;
                };
                let property = discriminator["propertyName"]
                    .as_str()
                    .expect("a discriminator property");
                assert!(
                    tags.iter()
                        .any(|(name, tag, _)| name == union && *tag == property),
                    "{union}: add its serde tag to `serde_tags`"
                );
                let mapping = discriminator["mapping"].as_object().expect("a mapping");
                let mut seen = std::collections::BTreeSet::new();
                for (tag, reference) in mapping {
                    let variant = reference
                        .as_str()
                        .and_then(|r| r.strip_prefix("#/components/schemas/"))
                        .expect("a schema reference");
                    assert!(seen.insert(tag.clone()), "{union}: tag {tag} mapped twice");
                    if standalone.contains(variant) {
                        assert!(
                            schemas[variant]["properties"].get(property).is_none()
                                && !schemas[variant]["required"]
                                    .as_array()
                                    .is_some_and(|required| required.contains(&property.into())),
                            "{union}: {variant} is also used standalone, untagged"
                        );
                        continue;
                    }
                    assert_eq!(
                        &schemas[variant]["properties"][property]["enum"],
                        &serde_json::json!([tag]),
                        "{union}: {variant} must declare {property} = {tag}"
                    );
                    assert!(
                        schemas[variant]["required"]
                            .as_array()
                            .is_some_and(|required| required.contains(&property.into())),
                        "{union}: {variant} must require {property}, as serde does"
                    );
                }
            }
        }
    }

    #[test]
    fn variant_used_standalone_gets_no_tag() {
        let mut openapi: utoipa::openapi::OpenApi = serde_json::from_value(serde_json::json!({
            "openapi": "3.1.0",
            "info": {"title": "t", "version": "1"},
            "paths": {},
            "components": {"schemas": {
                "Matrix": {
                    "oneOf": [
                        {"$ref": "#/components/schemas/Dimension"},
                        {"$ref": "#/components/schemas/Double"}
                    ],
                    "discriminator": {"propertyName": "type", "mapping": {
                        "SINGLE": "#/components/schemas/Dimension",
                        "DOUBLE": "#/components/schemas/Double"
                    }}
                },
                "Dimension": {
                    "type": "object",
                    "required": ["key"],
                    "properties": {"key": {"type": "string"}}
                },
                "Double": {
                    "type": "object",
                    "required": ["dimension1"],
                    "properties": {"dimension1": {"$ref": "#/components/schemas/Dimension"}}
                }
            }}
        }))
        .expect("a valid spec");
        discriminate_tagged_variants(&mut openapi);
        let spec = serde_json::to_value(&openapi).expect("the spec serializes");
        let schemas = &spec["components"]["schemas"];
        assert_eq!(schemas["Dimension"]["required"], serde_json::json!(["key"]));
        assert!(schemas["Dimension"]["properties"].get("type").is_none());
        assert_eq!(
            schemas["Double"]["required"],
            serde_json::json!(["dimension1", "type"])
        );
        assert_eq!(
            schemas["Double"]["properties"]["type"]["enum"],
            serde_json::json!(["DOUBLE"])
        );
    }

    #[test]
    fn published_spec_hides_beta_events() {
        let root = promote_webhooks_to_top_level(&build_spec(&EditionSpec::core()));
        let webhooks = root
            .get("webhooks")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let event_type_enum_values = root
            .get("components")
            .and_then(|c| c.get("schemas"))
            .and_then(|s| s.get("EventType"))
            .and_then(|e| e.get("enum"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        for event in WebhookOutEventTypeEnum::iter().filter(|event| event.group().is_beta()) {
            let name = event.to_string();
            assert!(
                !webhooks.contains_key(&name),
                "published spec must not list beta webhook `{name}`"
            );
            assert!(
                !event_type_enum_values
                    .iter()
                    .any(|value| value.as_str() == Some(name.as_str())),
                "published EventType enum must not list beta value `{name}`"
            );
        }
    }

    /// The credit routes and their webhooks are published (GA) under their final names; the
    /// routes of `BETA_TAGS` are published too, each marked beta, in industry vocabulary.
    #[test]
    fn published_spec_has_the_credit_surface_and_labels_beta_rest_paths() {
        let root = published_spec_root(&EditionSpec::core());
        let paths = root["paths"].as_object().expect("paths");
        for (path, methods) in [
            ("/api/v1/customers/{id_or_alias}/wallets", &["get"][..]),
            (
                "/api/v1/customers/{id_or_alias}/wallets/{currency}",
                &["patch"],
            ),
            (
                "/api/v1/customers/{id_or_alias}/credit-grants",
                &["get", "post"],
            ),
            ("/api/v1/customers/{id_or_alias}/credit-ledger", &["get"]),
            ("/api/v1/customers/{id_or_alias}/top-ups", &["post"]),
            ("/api/v1/credit-units", &["get", "post"]),
            ("/api/v1/credit-units/{id}", &["get", "patch"]),
            ("/api/v1/credit-units/{id}/prices", &["get"]),
            (
                "/api/v1/credit-units/{id}/prices/{currency}",
                &["put", "delete"],
            ),
        ] {
            for method in methods {
                assert!(
                    paths.get(path).and_then(|item| item.get(*method)).is_some(),
                    "{method} {path} must be published"
                );
            }
        }
        let path = "/api/v1/customers/{id_or_alias}/wallets";
        let parameters = paths[path]["get"]["parameters"]
            .as_array()
            .expect("parameters");
        let subscription = parameters
            .iter()
            .find(|parameter| parameter["name"] == "subscription_id")
            .unwrap_or_else(|| panic!("GET {path} must take subscription_id"));
        assert_eq!(subscription["in"], "query", "GET {path}");
        assert_ne!(subscription["required"], true, "GET {path}");
        assert_eq!(
            subscription["schema"]["$ref"], "#/components/schemas/SubscriptionId",
            "GET {path}"
        );
        for path in [
            "/api/v1/customers/{id_or_alias}/wallets/check",
            "/api/v1/customers/{id_or_alias}/entitlements/{feature_id_or_code}/check",
            "/api/v1/customers/{id_or_alias}/budgets",
        ] {
            assert!(paths.get(path).is_none(), "{path} is a module's");
        }
        for (path, method) in [
            ("/api/v1/usage/customer/{customer_id}/costs", "get"),
            ("/api/v1/usage/customer/{customer_id}/events", "get"),
            ("/api/v1/usage/subscription/{subscription_id}/costs", "get"),
            ("/api/v1/usage/subscription/{subscription_id}/events", "get"),
        ] {
            assert_eq!(
                paths[path][method]["x-beta"], true,
                "{method} {path} must be published as beta"
            );
        }
        for (path, item) in paths {
            for (method, op) in item.as_object().into_iter().flatten() {
                let Some(tags) = op.get("tags").and_then(|tags| tags.as_array()) else {
                    continue;
                };
                let beta = tags.iter().filter_map(|tag| tag.as_str()).any(is_beta_tag);
                assert_eq!(
                    op.get("x-beta").is_some(),
                    beta,
                    "{method} {path}: x-beta must match its tags"
                );
                if beta {
                    assert!(
                        op["description"]
                            .as_str()
                            .is_some_and(|d| d.ends_with(BETA_NOTICE)),
                        "{method} {path} must say its shape may change"
                    );
                }
            }
        }
        let tags = root["tags"].as_array().expect("tags");
        for (name, _) in BETA_TAGS.iter().filter(|(name, _)| *name == "Usage costs") {
            let tag = tags
                .iter()
                .find(|tag| tag["name"] == *name)
                .unwrap_or_else(|| panic!("tag {name}"));
            assert!(
                tag["description"]
                    .as_str()
                    .is_some_and(|d| d.starts_with("Beta")),
                "{name}"
            );
            assert_eq!(tag["x-beta"], true, "{name}");
        }

        let client = spec_json(&crate::client::client_api(EditionSpec::core().client).1);
        assert!(
            client["paths"]
                .get("/api/client/v1/entitlements/{feature_id_or_code}/check")
                .is_none()
        );

        let webhooks = root["webhooks"].as_object().expect("webhooks");
        let event_types = root["components"]["schemas"]["EventType"]["enum"]
            .as_array()
            .expect("EventType");
        for event in WebhookOutEventTypeEnum::iter().filter(|event| {
            matches!(
                event.group(),
                WebhookOutEventGroupEnum::Wallet | WebhookOutEventGroupEnum::CreditGrant
            )
        }) {
            let name = event.to_string();
            let listed = event_types.iter().any(|value| value == name.as_str());
            if event.needs_module() {
                assert!(!webhooks.contains_key(&name), "{name} is a module's");
                assert!(!listed, "{name} is a module's");
            } else {
                assert!(webhooks.contains_key(&name), "{name} must be published");
                assert!(listed, "{name} must be an EventType");
            }
        }

        let schemas = &root["components"]["schemas"];
        assert!(
            schemas["Wallet"]["properties"]["shared_balance"].is_object(),
            "Wallet carries shared_balance"
        );
        assert!(
            !schemas["Wallet"]["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|name| name == "shared_balance")),
            "Wallet.shared_balance is optional"
        );
        assert!(schemas["UsagePlanFee"]["properties"]["credit_unit"].is_object());
        assert!(schemas["CapacityPlanFee"]["properties"]["credit_unit"].is_object());
        assert!(schemas["PlanFeeCreditUnit"]["properties"]["rate"].is_object());
        assert!(schemas["UsageFee"]["properties"]["credit_unit"].is_object());
        assert!(schemas["FeeCreditUnit"]["properties"]["credit_unit_id"].is_object());
        assert!(schemas["CreditFee"]["properties"]["credit_unit"].is_object());
        assert!(schemas["CreditFee"]["properties"]["rollover"].is_object());
        assert!(schemas["Actor"]["properties"]["type"].is_object());

        let published = serde_json::to_string(&root).unwrap();
        for jargon in [
            "live_balance",
            "pending_usage",
            "available_balance",
            "available_usage_only",
            "upcoming_grants",
            "actor_id",
            "Freshness SLO",
            "bounded",
            "overshoot",
            "monitor",
            "projection",
            "watermark",
            "quiesce",
            "answer cache",
            "metering backend",
            "p95",
            "overflow",
        ] {
            assert!(
                !published.contains(jargon),
                "the published spec says {jargon:?}"
            );
        }
    }

    /// A dangling `$ref` (a `ToSchema` type used only as a query parameter) breaks SDK generation.
    #[test]
    fn every_component_ref_resolves() {
        fn refs<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, v) in map {
                        match (key.as_str(), v) {
                            ("$ref", serde_json::Value::String(r)) => out.push(r),
                            _ => refs(v, out),
                        }
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|v| refs(v, out)),
                _ => {}
            }
        }

        let published = serde_json::Value::Object(published_spec_root(&EditionSpec::core()));
        let client = spec_json(&crate::client::client_api(EditionSpec::core().client).1);
        for (name, spec) in [("published", &published), ("client", &client)] {
            let mut found = Vec::new();
            refs(spec, &mut found);
            assert!(!found.is_empty(), "{name} spec has refs");
            let missing = found
                .into_iter()
                .filter(|r| {
                    r.strip_prefix('#')
                        .filter(|pointer| pointer.starts_with("/components/"))
                        .and_then(|pointer| spec.pointer(pointer))
                        .is_none()
                })
                .collect::<std::collections::BTreeSet<_>>();
            assert!(missing.is_empty(), "{name} spec: unresolved {missing:?}");
        }
    }
}

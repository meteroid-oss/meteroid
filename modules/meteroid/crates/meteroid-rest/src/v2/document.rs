//! The v2 document as published and served (ADR-0013 §3), finished as JSON once the routes and
//! the edition's modules are merged: what utoipa's model cannot hold (`components.headers`, a
//! response header by reference) and what an SDK generator reads off the document to send the
//! common headers itself, page a list and require an `Idempotency-Key`, in a key order that
//! never changes from one generation to the next.
//!
//! - The headers every request may send (`Meteroid-Version`, `Idempotency-Key`,
//!   `X-Request-Id`) are documented once, in `components.parameters`, and on no operation: a
//!   generator would make each a per-method option, where the SDK sends them itself.
//! - The headers every response carries (`Meteroid-Version`, `Request-Id`, and
//!   `Idempotent-Replayed` where a key is taken) are `components.headers`, referenced by each
//!   response.
//! - Every operation answers `400`, `401` and `429` as problems (`components.responses`), the
//!   `401` with its `WWW-Authenticate`, the `429` with the rate limit's headers and limits.
//! - Every list's `after` says what a cursor is, how long it reads and what refuses one.
//! - Every POST but a preview and the entitlement check carries `x-idempotency: required |
//!   optional`, and so does a PATCH taking a key (`idempotency::KEYED_PATCHES`); every list
//!   paged by cursor carries `x-pagination`.
//! - Every object a JSON body holds is closed (`additionalProperties: false`): the body extractor
//!   refuses a field it does not have (`v2::extract`). One a response flattens (`allOf`) is left
//!   open, as closing it would refuse the response's own fields.
//! - Every enum a response holds is open: its values are `x-extensible-enum`, not `enum`, so a
//!   generated client accepts a value it does not know. A tag (a single value) stays an `enum`.
use crate::v2::envelope::{CALLER_REQUEST_ID_HEADER, REQUEST_ID_HEADER};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, KEYED_PATCHES, REPLAYED_HEADER};
use crate::v2::pagination::{DEFAULT_LIMIT, MAX_LIMIT};
use crate::v2::problem::PROBLEM_CONTENT_TYPE;
use crate::v2::version::{VERSION_HEADER, VERSIONS};
use meteroid_app::config::RestRateLimitConfig;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// Whether an operation takes an `Idempotency-Key`: `required` or `optional`. A GET, a PUT, a
/// DELETE, a preview, the entitlement check and a PATCH but those of
/// `idempotency::KEYED_PATCHES` take none, and carry no such extension.
pub const IDEMPOTENCY_EXTENSION: &str = "x-idempotency";

/// The values of an open enum, which may grow without a new version (Zalando's convention, which
/// the compatibility check reads: a value added to a response is not a break).
pub const EXTENSIBLE_ENUM: &str = "x-extensible-enum";

/// How a list is paged: the query parameter taking the cursor, the response fields giving the
/// next one and telling whether more follow, the items and the page size.
pub const PAGINATION_EXTENSION: &str = "x-pagination";

/// The operations refusing a request without an `Idempotency-Key` (`400
/// idempotency_key_required`): the writes that can invoice, credit or record money, and the usage
/// counted by the app, which a retry must never apply twice.
pub const KEY_REQUIRED: &[&str] = &[
    "create_subscription",
    "change_plan",
    "change_quantity",
    "change_level",
    "amend_subscription",
    "cancel_subscription",
    "end_trial",
    "create_invoice",
    "record_invoice_payment",
    "reverse_invoice_payment",
    "create_credit_note",
    "create_credit_grant",
    "create_top_up",
    "report_entitlement_usage",
];

/// Bodies v1 defines (and an edition's module shares with its v1 routes), published in v2 under
/// v2's names: `<Verb><Noun>` for a request, its result for a response. v1's document keeps
/// theirs.
pub const RENAMED_SCHEMAS: &[(&str, &str)] = &[("EventChangeResponse", "EventChange")];

/// The schema a retry's replay reads as when the first response was not kept.
const REPLAY_MARKER: &str = "IdempotentReplayMarker";

/// The problem responses every operation documents, by status, as `components.responses`.
const COMMON_PROBLEMS: &[(&str, &str, &str)] = &[
    (
        "400",
        "BadRequest",
        "The request could not be read: a malformed body or parameter, or an unsupported \
         `Meteroid-Version`.",
    ),
    (
        "401",
        "Unauthorized",
        "The API key is missing, unknown or revoked.",
    ),
    (
        "429",
        "TooManyRequests",
        "Too many requests: retry later, with the same `Idempotency-Key` for a POST.",
    ),
];

/// What a list's `after` says, on every list paged by cursor: what a list's own text adds follows
/// it.
pub const CURSOR_DESCRIPTION: &str = "The `next_cursor` of the previous page; the first page when \
     left out. A cursor is opaque: send it back as it was answered, with the same filters. It \
     names the last item read, never a position, so a later page neither repeats nor skips an \
     item however the list changes, and it stays valid unless this list says otherwise. A \
     cursor that is not one of this list's is refused, `422 validation_failed` with \
     `errors[0]` at `after`, code `invalid`.";

/// The text of a list's `after` the document replaces with [`CURSOR_DESCRIPTION`].
const PLAIN_CURSOR: &str = "The `next_cursor` of the previous page.";

/// The limits of the request rate and those counted within a request, at their defaults
/// (`RestRateLimitConfig`, and the Enterprise event corrections'), for the `429` response.
fn rate_limits() -> String {
    let limits = RestRateLimitConfig::default();
    let grouped = |count: u64| {
        let digits = count.to_string();
        let mut out = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                out.push(',');
            }
            out.push(digit);
        }
        out
    };
    format!(
        "Too many requests: retry later, with the same `Idempotency-Key` for a POST.\n\n\
         Limits are per tenant environment, every API key of the tenant sharing them, each a \
         token bucket refilled continuously over its window. At their defaults (a self-hosted \
         instance sets its own, `RATE_LIMIT_REST_*`), production first, then any other \
         environment:\n\n\
         - Requests: {} and {} a second, every route but the event routes; `RateLimit-Policy` \
         names the bucket `rest:api` (`rest:non_prod:api`).\n\
         - Requests to the event routes (`/api/v2/events/…`): {} and {} a second, bucket \
         `rest:metering` (`rest:non_prod:metering`).\n\
         - Events ingested: {} and {} a second, each `POST /events/ingest` counting the events \
         it holds, unless the organization's plan sets its own.\n\
         - Usage reads (`/usage/summary`, and `…/usage…` under a customer or a subscription): \
         {} and {} a minute, unless the organization's plan sets its own.\n\
         - Event corrections (`/events/{{event_id}}/void` and `…/amend`, Enterprise edition): \
         600 and 120 changes a minute.\n\n\
         The first two count requests: their `429` carries `Retry-After` and the `RateLimit` \
         headers, which every response they counted carries too. The others are counted within \
         the request, after it passed the first: their `429 rate_limited` carries `Retry-After` \
         and says the same wait in `detail` (`Retry after <n> seconds`), without the \
         `RateLimit` headers.",
        grouped(limits.prod_rps),
        grouped(limits.non_prod_rps),
        grouped(limits.prod_metering_rps),
        grouped(limits.non_prod_metering_rps),
        grouped(limits.prod_metering_eps),
        grouped(limits.non_prod_metering_eps),
        grouped(limits.prod_usage_costs_rpm),
        grouped(limits.non_prod_usage_costs_rpm),
    )
}

const METHODS: [&str; 5] = ["get", "put", "post", "delete", "patch"];

const KEY_ORDER: [&str; 6] = ["openapi", "info", "servers", "paths", "components", "tags"];

/// Finishes the JSON of the routes' document (`openapi::spec_json`).
pub fn finish(root: &mut Map<String, Value>) {
    rename_schemas(root);
    close_bodies(root);
    describe_properties(root);
    drop_retired_event_types(root);
    root.insert("servers".to_owned(), servers());
    let paged = paged_lists(root);
    if let Some(paths) = root.get_mut("paths").and_then(Value::as_object_mut) {
        for (path, item) in paths.iter_mut() {
            let Some(item) = item.as_object_mut() else {
                continue;
            };
            for method in METHODS {
                if let Some(operation) = item.get_mut(method).and_then(Value::as_object_mut) {
                    finish_operation(path, method, operation, &paged);
                }
            }
        }
    }
    let components = root
        .entry("components")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(components) = components.as_object_mut() {
        components.insert("responses".to_owned(), problem_responses());
        components.insert("parameters".to_owned(), request_headers());
        components.insert("headers".to_owned(), response_headers());
    }
    open_enums(root);
    let mut rest = std::mem::take(root);
    for key in KEY_ORDER {
        if let Some(value) = rest.remove(key) {
            root.insert(key.to_owned(), value);
        }
    }
    root.extend(rest);
}

/// Gives a description its property, where a generator reads it for the field's docstring: an
/// optional reference's (`oneOf: [{$ref, description}, {type: null}]`, as utoipa writes a
/// documented `Option<_>`) moves up to the property, and a variant's tag says which variant of
/// which union it names.
fn describe_properties(root: &mut Map<String, Value>) {
    let Some(schemas) = root
        .get_mut("components")
        .and_then(|components| components.get_mut("schemas"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut tags = Vec::new();
    for (union, schema) in schemas.iter() {
        let Some(property) = schema["discriminator"]["propertyName"].as_str() else {
            continue;
        };
        for (tag, target) in schema["discriminator"]["mapping"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if let Some(variant) = target
                .as_str()
                .and_then(|target| target.strip_prefix(SCHEMA_REF))
            {
                tags.push((
                    variant.to_owned(),
                    property.to_owned(),
                    tag.clone(),
                    union.clone(),
                ));
            }
        }
    }
    for (variant, property, tag, union) in tags {
        if let Some(declared) = schemas
            .get_mut(&variant)
            .and_then(|schema| schema.get_mut("properties"))
            .and_then(|properties| properties.get_mut(&property))
            .and_then(Value::as_object_mut)
            && !declared.contains_key("description")
        {
            declared.insert(
                "description".to_owned(),
                json!(format!("`{tag}`: names this variant of `{union}`.")),
            );
        }
    }
    for schema in schemas.values_mut() {
        hoist_descriptions(schema);
    }
}

fn hoist_descriptions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                for property in properties.values_mut() {
                    let Some(property) = property.as_object_mut() else {
                        continue;
                    };
                    if property.contains_key("description") {
                        continue;
                    }
                    let Some(members) = property.get_mut("oneOf").and_then(Value::as_array_mut)
                    else {
                        continue;
                    };
                    if members.len() != 2 || !members.iter().any(|member| member["type"] == "null")
                    {
                        continue;
                    }
                    let described = members
                        .iter_mut()
                        .filter_map(Value::as_object_mut)
                        .find_map(|member| member.remove("description"));
                    if let Some(description) = described {
                        property.insert("description".to_owned(), description);
                    }
                }
            }
            object.values_mut().for_each(hoist_descriptions);
        }
        Value::Array(items) => items.iter_mut().for_each(hoist_descriptions),
        _ => {}
    }
}

/// v1 event types v2 does not list: never emitted any more, refused on v2's writes.
pub const RETIRED_EVENT_TYPES: [&str; 1] = ["entitlement.too_many_groups"];

fn drop_retired_event_types(root: &mut Map<String, Value>) {
    if let Some(values) = root
        .get_mut("components")
        .and_then(|components| components.get_mut("schemas"))
        .and_then(|schemas| schemas.get_mut("EventType"))
        .and_then(|event_type| event_type.get_mut("enum"))
        .and_then(Value::as_array_mut)
    {
        values.retain(|value| {
            !value
                .as_str()
                .is_some_and(|name| RETIRED_EVENT_TYPES.contains(&name))
        });
    }
}

/// Where the API is served; every path starts with `/api/v2`.
fn servers() -> Value {
    json!([
        {"url": "https://api.meteroid.com", "description": "Meteroid Cloud"},
        {
            "url": "{base_url}",
            "description": "A self-hosted instance: its REST API's external URL",
            "variables": {"base_url": {"default": "http://localhost:8084"}}
        }
    ])
}

fn schema_ref(name: &str) -> String {
    format!("#/components/schemas/{name}")
}

fn references<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                out.push(reference);
            }
            object.values().for_each(|value| references(value, out));
        }
        Value::Array(items) => items.iter().for_each(|value| references(value, out)),
        _ => {}
    }
}

/// The schemas a JSON request body holds, by name, followed through their references.
fn body_schemas(root: &Map<String, Value>) -> BTreeSet<String> {
    let mut found = Vec::new();
    for operation in operations(root) {
        let content = operation
            .pointer("/requestBody/content")
            .and_then(Value::as_object);
        for (content_type, media) in content.into_iter().flatten() {
            if content_type.ends_with("json") {
                references(media, &mut found);
            }
        }
    }
    held_through(root, found)
}

/// The schemas a response holds, an operation's or a shared one's, by name, followed through
/// their references.
fn response_schemas(root: &Map<String, Value>) -> BTreeSet<String> {
    let mut found = Vec::new();
    for operation in operations(root) {
        if let Some(responses) = operation.get("responses") {
            references(responses, &mut found);
        }
    }
    if let Some(shared) = root.get("components").and_then(|c| c.get("responses")) {
        references(shared, &mut found);
    }
    held_through(root, found)
}

fn operations(root: &Map<String, Value>) -> impl Iterator<Item = &Value> {
    root.get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::values)
        .flat_map(|item| METHODS.into_iter().filter_map(|method| item.get(method)))
}

/// The schemas `found` references, and those they reference in turn, by name.
fn held_through(root: &Map<String, Value>, found: Vec<&str>) -> BTreeSet<String> {
    let schemas = root
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object);
    let mut held = BTreeSet::new();
    let mut next: Vec<String> = found.into_iter().map(str::to_owned).collect();
    while let Some(reference) = next.pop() {
        let Some(name) = reference.strip_prefix("#/components/schemas/") else {
            continue;
        };
        if !held.insert(name.to_owned()) {
            continue;
        }
        if let Some(schema) = schemas.and_then(|schemas| schemas.get(name)) {
            let mut inner = Vec::new();
            references(schema, &mut inner);
            next.extend(inner.into_iter().map(str::to_owned));
        }
    }
    held
}

/// Opens every enum a response holds (ADR-0013 §3), in the responses and the schemas they hold.
fn open_enums(root: &mut Map<String, Value>) {
    let held = response_schemas(root);
    if let Some(paths) = root.get_mut("paths").and_then(Value::as_object_mut) {
        for item in paths.values_mut() {
            for method in METHODS {
                if let Some(responses) = item.pointer_mut(&format!("/{method}/responses")) {
                    open_enum(responses);
                }
            }
        }
    }
    let Some(components) = root.get_mut("components").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(shared) = components.get_mut("responses") {
        open_enum(shared);
    }
    if let Some(schemas) = components.get_mut("schemas").and_then(Value::as_object_mut) {
        for (name, schema) in schemas.iter_mut() {
            if held.contains(name) {
                open_enum(schema);
            }
        }
    }
}

fn open_enum(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|values| values.len() > 1)
                && let Some(values) = object.remove("enum")
            {
                object.insert(EXTENSIBLE_ENUM.to_owned(), values);
            }
            object.values_mut().for_each(open_enum);
        }
        Value::Array(items) => items.iter_mut().for_each(open_enum),
        _ => {}
    }
}

/// Closes every object a JSON request body holds but those a response flattens (`allOf`).
fn close_bodies(root: &mut Map<String, Value>) {
    let held = body_schemas(root);
    let Some(schemas) = root
        .get_mut("components")
        .and_then(|components| components.get_mut("schemas"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut flattened = Vec::new();
    for schema in schemas.values() {
        schema_all_of(schema, &mut flattened);
    }
    for name in held {
        if flattened
            .iter()
            .any(|reference| reference == &schema_ref(&name))
        {
            continue;
        }
        if let Some(schema) = schemas.get_mut(&name).and_then(Value::as_object_mut)
            && schema.contains_key("properties")
            && !schema.contains_key("additionalProperties")
        {
            schema.insert("additionalProperties".to_owned(), Value::Bool(false));
        }
    }
}

/// The schemas `value` composes with `allOf`.
fn schema_all_of(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(parts) = object.get("allOf") {
                let mut found = Vec::new();
                references(parts, &mut found);
                out.extend(found.into_iter().map(str::to_owned));
            }
            object.values().for_each(|value| schema_all_of(value, out));
        }
        Value::Array(items) => items.iter().for_each(|value| schema_all_of(value, out)),
        _ => {}
    }
}

/// Renames [`RENAMED_SCHEMAS`] whose new name is free, and every reference to them (`$ref`s and
/// discriminator mappings); the schemas stay sorted by name, as utoipa writes them.
fn rename_schemas(root: &mut Map<String, Value>) {
    let Some(schemas) = root
        .get_mut("components")
        .and_then(|components| components.get_mut("schemas"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut renamed = Vec::new();
    for (from, to) in RENAMED_SCHEMAS {
        if schemas.contains_key(*to) {
            continue;
        }
        if let Some(schema) = schemas.remove(*from) {
            schemas.insert((*to).to_owned(), schema);
            renamed.push((schema_ref(from), schema_ref(to)));
        }
    }
    if renamed.is_empty() {
        return;
    }
    let mut sorted: Vec<(String, Value)> = std::mem::take(schemas).into_iter().collect();
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
    schemas.extend(sorted);
    for (_, value) in root.iter_mut() {
        rewrite_refs(value, &renamed);
    }
}

fn rewrite_refs(value: &mut Value, renamed: &[(String, String)]) {
    match value {
        Value::String(text) => {
            if let Some((_, to)) = renamed
                .iter()
                .find(|(from, _)| from.as_str() == text.as_str())
            {
                *text = to.clone();
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| rewrite_refs(item, renamed)),
        Value::Object(map) => map
            .iter_mut()
            .for_each(|(_, item)| rewrite_refs(item, renamed)),
        _ => {}
    }
}

/// The schemas of a page of a list: `{data, next_cursor, has_more}`.
fn paged_lists(root: &Map<String, Value>) -> BTreeSet<String> {
    root.get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, schema)| {
            ["data", "next_cursor", "has_more"]
                .iter()
                .all(|field| schema["properties"].get(*field).is_some())
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn finish_operation(
    path: &str,
    method: &str,
    operation: &mut Map<String, Value>,
    paged: &BTreeSet<String>,
) {
    // OAuth's endpoints authenticate the client in the request: no key claims a key there.
    let keyed = (method == "post"
        && !path.ends_with("/preview")
        && !crate::v2::idempotency::reads_only(path)
        && !path.starts_with(crate::v2::OAUTH_PREFIX))
        || (method == "patch" && KEYED_PATCHES.contains(&path));
    let idempotency = keyed.then(|| {
        let id = operation
            .get("operationId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if KEY_REQUIRED.contains(&id) {
            "required"
        } else {
            "optional"
        }
    });
    let pages = method == "get" && page_parameters(operation) && answers_a_page(operation, paged);
    if let Some(responses) = operation
        .get_mut("responses")
        .and_then(Value::as_object_mut)
    {
        for (status, name, _) in COMMON_PROBLEMS {
            let reference = json!({"$ref": format!("#/components/responses/{name}")});
            // An API key's refusal is the common one; OAuth's endpoints keep RFC 6749's.
            if *status == "401" && !path.starts_with(crate::v2::OAUTH_PREFIX) {
                responses.insert((*status).to_owned(), reference);
            } else {
                responses.entry(*status).or_insert(reference);
            }
        }
        for (_, response) in responses.iter_mut() {
            if let Some(response) = response.as_object_mut()
                && !response.contains_key("$ref")
            {
                response.insert("headers".to_owned(), header_refs(idempotency.is_some()));
            }
        }
        let mut sorted: Vec<(String, Value)> = std::mem::take(responses).into_iter().collect();
        sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
        responses.extend(sorted);
    }
    if let Some(idempotency) = idempotency {
        operation.insert(IDEMPOTENCY_EXTENSION.to_owned(), json!(idempotency));
    }
    if pages {
        operation.insert(
            PAGINATION_EXTENSION.to_owned(),
            json!({
                "cursor": "after",
                "next_cursor": "next_cursor",
                "has_more": "has_more",
                "items": "data",
                "limit": "limit"
            }),
        );
    }
}

/// Declares `limit` (1 to 100, 20 by default) and `after` alike on every list; whether the
/// operation takes both.
fn page_parameters(operation: &mut Map<String, Value>) -> bool {
    let mut limit = false;
    let mut after = false;
    for parameter in operation
        .get_mut("parameters")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(parameter) = parameter.as_object_mut() else {
            continue;
        };
        if parameter.get("in").and_then(Value::as_str) != Some("query") {
            continue;
        }
        let name = parameter
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match name.as_deref() {
            Some("limit") => {
                limit = true;
                parameter.entry("description").or_insert_with(|| {
                    json!(format!(
                        "How many items, 1 to {MAX_LIMIT}; {DEFAULT_LIMIT} by default."
                    ))
                });
                parameter.insert(
                    "schema".to_owned(),
                    json!({
                        "type": "integer",
                        "format": "int32",
                        "minimum": 1,
                        "maximum": MAX_LIMIT,
                        "default": DEFAULT_LIMIT
                    }),
                );
            }
            Some("after") => {
                after = true;
                let own = parameter
                    .get("description")
                    .and_then(Value::as_str)
                    .map(|text| text.strip_prefix(PLAIN_CURSOR).unwrap_or(text).trim())
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned);
                let described = match own {
                    Some(own) => format!("{CURSOR_DESCRIPTION} {own}"),
                    None => CURSOR_DESCRIPTION.to_owned(),
                };
                parameter.insert("description".to_owned(), json!(described));
            }
            _ => {}
        }
    }
    limit && after
}

fn answers_a_page(operation: &Map<String, Value>, paged: &BTreeSet<String>) -> bool {
    operation
        .get("responses")
        .and_then(|responses| {
            responses["200"]["content"]["application/json"]["schema"]["$ref"].as_str()
        })
        .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
        .is_some_and(|name| paged.contains(name))
}

fn header_refs(replayable: bool) -> Value {
    let mut headers = Map::new();
    let mut names = vec![VERSION_HEADER, REQUEST_ID_HEADER];
    if replayable {
        names.push(REPLAYED_HEADER);
    }
    for name in names {
        headers.insert(
            name.to_owned(),
            json!({"$ref": format!("#/components/headers/{name}")}),
        );
    }
    Value::Object(headers)
}

fn problem_responses() -> Value {
    let mut responses = Map::new();
    for (status, name, description) in COMMON_PROBLEMS {
        let mut content = Map::new();
        content.insert(
            PROBLEM_CONTENT_TYPE.to_owned(),
            json!({"schema": {"$ref": schema_ref("Problem")}}),
        );
        let mut headers = header_refs(false);
        if let Some(headers) = headers.as_object_mut() {
            headers.extend(problem_headers(status));
        }
        let description = if *status == "429" {
            rate_limits()
        } else {
            (*description).to_owned()
        };
        responses.insert(
            (*name).to_owned(),
            json!({
                "description": description,
                "headers": headers,
                "content": content
            }),
        );
    }
    Value::Object(responses)
}

/// The headers a common problem carries beside every response's: a `401` its challenge, a `429`
/// the rate limit's state (`rate_limiter::middleware::axum`).
fn problem_headers(status: &str) -> Map<String, Value> {
    let header =
        |description: &str, schema: Value| json!({"description": description, "schema": schema});
    let integer = || json!({"type": "integer", "format": "int64", "minimum": 0});
    let mut headers = Map::new();
    match status {
        "401" => {
            headers.insert(
                "WWW-Authenticate".to_owned(),
                header(
                    "`Bearer`: the API key is sent as `Authorization: Bearer <key>`.",
                    json!({"type": "string"}),
                ),
            );
        }
        "429" => {
            headers.insert(
                "Retry-After".to_owned(),
                header(
                    "Seconds to wait before retrying. Sent when the request rate refused the \
                     request; a limit counted within a request (events per second, usage cost \
                     reads, event corrections) says its wait in `detail`.",
                    integer(),
                ),
            );
            headers.insert(
                "RateLimit-Limit".to_owned(),
                header("Requests the bucket holds when full.", integer()),
            );
            headers.insert(
                "RateLimit-Remaining".to_owned(),
                header("Requests left in the bucket: `0` on a refusal.", integer()),
            );
            headers.insert(
                "RateLimit-Reset".to_owned(),
                header(
                    "The Unix time, in seconds, the bucket is full again.",
                    integer(),
                ),
            );
            headers.insert(
                "RateLimit-Policy".to_owned(),
                header(
                    "The bucket as the IETF httpapi draft's structured field: \
                     `\"<policy>\";q=<requests>;w=<window seconds>`.",
                    json!({"type": "string"}),
                ),
            );
            headers.insert(
                "RateLimit".to_owned(),
                header(
                    "What is left of it as the draft's structured field: \
                     `\"<policy>\";r=<requests left>;t=<seconds until full>`.",
                    json!({"type": "string"}),
                ),
            );
        }
        _ => {}
    }
    headers
}

/// The headers a request may send, documented once: the SDK sends them itself.
fn request_headers() -> Value {
    let header = |name: &str, description: String, schema: Value| {
        json!({
            "name": name,
            "in": "header",
            "required": false,
            "description": description,
            "schema": schema
        })
    };
    let mut parameters = Map::new();
    parameters.insert(
        VERSION_HEADER.to_owned(),
        header(
            VERSION_HEADER,
            format!(
                "The API version to serve, as a date: the latest version published on or before \
                 it. The first, {}, when left out; a date before it, or not a date, is refused \
                 (`400 unsupported_version`). Published: {}.",
                VERSIONS[0],
                VERSIONS.join(", ")
            ),
            json!({"type": "string", "format": "date", "default": VERSIONS[0]}),
        ),
    );
    let mut key = header(
        IDEMPOTENCY_KEY_HEADER,
        format!(
            "Taken by an operation whose `{IDEMPOTENCY_EXTENSION}` is `required` or `optional` \
             (a POST, and a PATCH whose body lists changes), kept 24 hours; a `required` one is \
             refused without it (`400 idempotency_key_required`). Any other PATCH, a PUT and a \
             DELETE are sent again as they are. A retry \
             with the same key and request replays the first response (`{REPLAYED_HEADER}: \
             true`), or an `{REPLAY_MARKER}` (its `x-replay-marker`) when the first response was \
             not kept; the same key with another request is refused (`422 \
             idempotency_key_reused`), and while the first still runs (`409 \
             idempotency_request_in_progress`)."
        ),
        json!({"type": "string", "minLength": 1, "maxLength": 255}),
    );
    if let Some(key) = key.as_object_mut() {
        key.insert(
            "x-replay-marker".to_owned(),
            json!({"$ref": schema_ref(REPLAY_MARKER)}),
        );
    }
    parameters.insert(IDEMPOTENCY_KEY_HEADER.to_owned(), key);
    parameters.insert(
        CALLER_REQUEST_ID_HEADER.to_owned(),
        header(
            CALLER_REQUEST_ID_HEADER,
            format!(
                "Your id of the request, up to 128 visible ASCII characters: answered as \
                 `{REQUEST_ID_HEADER}` and in a problem's `request_id`. One is generated when \
                 left out."
            ),
            json!({"type": "string", "maxLength": 128}),
        ),
    );
    Value::Object(parameters)
}

/// The headers every response carries.
fn response_headers() -> Value {
    let mut headers = Map::new();
    headers.insert(
        VERSION_HEADER.to_owned(),
        json!({
            "description": "The version the request was served at.",
            "schema": {"type": "string", "format": "date"}
        }),
    );
    headers.insert(
        REQUEST_ID_HEADER.to_owned(),
        json!({
            "description": format!(
                "The id of the request: the `{CALLER_REQUEST_ID_HEADER}` sent, else a generated \
                 one. Quote it to support."
            ),
            "schema": {"type": "string"}
        }),
    );
    headers.insert(
        REPLAYED_HEADER.to_owned(),
        json!({
            "description": format!(
                "`true` on the replay of the first response to a request with the same \
                 `{IDEMPOTENCY_KEY_HEADER}`: its status and body, or an `{REPLAY_MARKER}` when \
                 the first response was not kept (read the resource by its `id`)."
            ),
            "schema": {"type": "boolean"}
        }),
    );
    Value::Object(headers)
}

/// The edition's webhooks (those its v1 document publishes) in v2's payloads (ADR-0013 §15):
/// each event type's payload is an envelope schema, `<Data>Event`, whose `data` is the resource's
/// own schema (`Invoice`) or the notification's (`WalletFundedEventData`, copied from
/// `webhook_payloads::WebhookPayloadsDoc` with what it names that v2 does not hold yet). v1's
/// payload schemas stay in v1's document. The `Webhooks` tag gains what it says of versions
/// (`webhook_payloads::VERSIONS_NOTE`), a paragraph of its own.
pub fn attach_webhooks(v2: &mut Map<String, Value>, v1: &Map<String, Value>) {
    use crate::v2::webhook_payloads::{
        VERSIONS_NOTE, WebhookPayloadsDoc, data_schema, envelope_schema, read_as,
    };
    use crate::webhooks::out_model::WebhookOutEventTypeEnum;
    use std::str::FromStr;
    use utoipa::OpenApi;

    let Some(published) = v1.get("webhooks").and_then(Value::as_object) else {
        return;
    };
    let notices = serde_json::to_value(WebhookPayloadsDoc::openapi())
        .expect("the payloads' document serializes");
    let notices = notices["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let schemas = v2
        .entry("components")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("components is an object")
        .entry("schemas")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("schemas is an object");

    let mut webhooks = Map::new();
    for name in published.keys() {
        let Ok(event) = WebhookOutEventTypeEnum::from_str(name) else {
            continue;
        };
        let data = data_schema(event);
        let mut stack = schema_refs(&envelope_of(data));
        while let Some(needed) = stack.pop() {
            if schemas.contains_key(&needed) {
                continue;
            }
            if let Some(schema) = notices.get(&needed) {
                stack.extend(schema_refs(schema));
                schemas.insert(needed, schema.clone());
            }
        }
        let envelope = envelope_schema(event);
        schemas
            .entry(envelope.clone())
            .or_insert_with(|| envelope_of(data));
        let reads = match read_as(event) {
            Some(get) => {
                format!("`data` is the resource as `{get}` reads it, when the event is processed.")
            }
            None => "`data` is what the notification observed.".to_owned(),
        };
        webhooks.insert(
            name.clone(),
            json!({
                "post": {
                    "summary": format!("{name} webhook"),
                    "description": format!("{}\n\n{reads}", event.description()),
                    "operationId": format!("webhook_{}", name.replace('.', "_")),
                    "tags": ["Webhooks"],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {"$ref": schema_ref(&envelope)}
                            }
                        }
                    },
                    "responses": {
                        "200": {"description": "Received: any 2xx answer acknowledges it"}
                    }
                }
            }),
        );
    }
    // The schemas added beside the routes' keep the document's order: by name.
    let mut sorted: Vec<(String, Value)> = std::mem::take(schemas).into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    schemas.extend(sorted);
    v2.insert("webhooks".to_owned(), Value::Object(webhooks));

    if let Some(tag) = v2
        .get_mut("tags")
        .and_then(Value::as_array_mut)
        .and_then(|tags| tags.iter_mut().find(|tag| tag["name"] == "Webhooks"))
        .and_then(Value::as_object_mut)
    {
        let described = match tag.get("description").and_then(Value::as_str) {
            Some(description) => format!("{description}\n\n{VERSIONS_NOTE}"),
            None => VERSIONS_NOTE.to_owned(),
        };
        tag.insert("description".to_owned(), Value::String(described));
    }
}

/// An event's payload in v2's shape, `data` of the schema `data`.
fn envelope_of(data: &str) -> Value {
    let mut envelope = json!({
        "type": "object",
        "description": format!(
            "An event's payload in API v2's shape, sent to an endpoint whose `api_version` is \
             `{}` or later; `data` is a `{data}`.",
            crate::v2::version::V2_WEBHOOK_PAYLOADS
        ),
        "required": ["id", "type", "created_at", "api_version", "data"],
        "properties": {
            "id": {
                "$ref": schema_ref("EventId"),
                "description": "The event's id, also its `webhook-id` header: the same on every \
                    retry and to every endpoint."
            },
            "type": {"$ref": schema_ref("EventType")},
            "created_at": {
                "type": "string",
                "format": "date-time",
                "description": "When the event was recorded."
            },
            "api_version": {
                "type": "string",
                "description": "The version the payload is rendered at: the endpoint's \
                    `api_version` when the event was processed.",
                "example": crate::v2::version::V2_WEBHOOK_PAYLOADS
            },
            "data": {"$ref": schema_ref(data)}
        }
    });
    if matches!(data, "Customer" | "Subscription") {
        envelope["properties"]["previous_attributes"] = json!({
            "$ref": schema_ref("PreviousAttributes"),
            "description": "On `customer.updated` and `subscription.updated`: what the change \
                replaced. Absent when it replaced none of the attributes listed there."
        });
    }
    envelope
}

const SCHEMA_REF: &str = "#/components/schemas/";

/// The schemas a value names, by `$ref` or in a discriminator's mapping.
fn schema_refs(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::String(text) => {
                if let Some(name) = text.strip_prefix(SCHEMA_REF) {
                    found.push(name.to_owned());
                }
            }
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => stack.extend(map.values()),
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use meteroid_rest_common::spec::EditionSpec;

    fn published() -> Value {
        crate::v2::spec_json(&crate::v2::spec(&EditionSpec::core()))
    }

    /// Every operation of the document: its method, its path and itself.
    fn operations(spec: &Value) -> Vec<(&str, &str, &Map<String, Value>)> {
        spec["paths"]
            .as_object()
            .expect("paths")
            .iter()
            .flat_map(|(path, item)| {
                METHODS.into_iter().filter_map(move |method| {
                    item.get(method)
                        .and_then(Value::as_object)
                        .map(|operation| (method, path.as_str(), operation))
                })
            })
            .collect()
    }

    fn resolves<'a>(spec: &'a Value, reference: &str) -> Option<&'a Value> {
        reference
            .strip_prefix('#')
            .and_then(|pointer| spec.pointer(pointer))
    }

    #[test]
    fn the_sdk_sends_the_common_headers_itself() {
        let spec = published();
        for (method, path, operation) in operations(&spec) {
            let headers: Vec<&Value> = operation
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|parameter| parameter["in"] == "header")
                // A conditional read's `If-None-Match` is the caller's, per call: kept on it.
                .filter(|parameter| {
                    !(parameter["name"] == "If-None-Match"
                        && operation["responses"].get("304").is_some())
                })
                .collect();
            assert!(headers.is_empty(), "{method} {path}: {headers:?}");
            for (status, response) in operation["responses"].as_object().expect("responses") {
                if response.get("$ref").is_some() {
                    continue;
                }
                for name in [VERSION_HEADER, REQUEST_ID_HEADER] {
                    let reference = response["headers"][name]["$ref"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{method} {path} {status}: {name}"));
                    assert!(resolves(&spec, reference).is_some(), "{reference}");
                }
            }
        }
        for name in [
            VERSION_HEADER,
            IDEMPOTENCY_KEY_HEADER,
            CALLER_REQUEST_ID_HEADER,
        ] {
            assert_eq!(spec["components"]["parameters"][name]["in"], "header");
        }
        assert!(
            spec["servers"]
                .as_array()
                .is_some_and(|servers| !servers.is_empty())
        );
    }

    #[test]
    fn a_post_says_whether_it_takes_a_key() {
        let spec = published();
        let operations = operations(&spec);
        for id in KEY_REQUIRED {
            let (method, path, operation) = operations
                .iter()
                .find(|(_, _, operation)| operation["operationId"] == *id)
                .unwrap_or_else(|| panic!("{id} is no operation"));
            assert_eq!(*method, "post", "{id}: {path}");
            assert_eq!(
                operation.get(IDEMPOTENCY_EXTENSION).and_then(Value::as_str),
                Some("required"),
                "{id}"
            );
        }
        for keyed in KEYED_PATCHES {
            assert!(
                operations
                    .iter()
                    .any(|(method, path, _)| *method == "patch" && path == keyed),
                "{keyed} is no PATCH"
            );
        }
        for (method, path, operation) in &operations {
            let taken = operation.get(IDEMPOTENCY_EXTENSION).and_then(Value::as_str);
            if *method == "patch" && KEYED_PATCHES.contains(path) {
                assert_eq!(taken, Some("optional"), "{method} {path}");
            } else if *method != "post"
                || path.ends_with("/preview")
                || crate::v2::idempotency::reads_only(path)
                || path.starts_with(crate::v2::OAUTH_PREFIX)
            {
                assert_eq!(taken, None, "{method} {path}");
            } else {
                assert!(
                    matches!(taken, Some("required" | "optional")),
                    "{method} {path}"
                );
                let created = operation["responses"]
                    .as_object()
                    .expect("responses")
                    .values()
                    .find(|response| response.get("$ref").is_none())
                    .expect("a response of its own");
                assert!(created["headers"][REPLAYED_HEADER].is_object(), "{path}");
            }
        }
    }

    #[test]
    fn every_list_paged_by_cursor_says_how() {
        let spec = published();
        let mut paged = 0;
        for (method, path, operation) in operations(&spec) {
            let parameters: Vec<&Value> = operation
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|parameter| parameter["in"] == "query")
                .collect();
            let named = |name: &str| {
                parameters
                    .iter()
                    .find(|parameter| parameter["name"] == name)
            };
            if let Some(limit) = named("limit") {
                assert_eq!(
                    limit["schema"],
                    json!({"type": "integer", "format": "int32", "minimum": 1, "maximum": 100, "default": 20}),
                    "{method} {path}"
                );
            }
            if named("after").is_some() {
                paged += 1;
                assert_eq!(
                    operation
                        .get(PAGINATION_EXTENSION)
                        .map(|pagination| &pagination["cursor"]),
                    Some(&json!("after")),
                    "{method} {path}"
                );
            } else {
                assert!(
                    operation.get(PAGINATION_EXTENSION).is_none(),
                    "{method} {path}"
                );
            }
        }
        assert!(paged > 20, "{paged} lists");
        let wallets = &spec["components"]["schemas"]["WalletList"]["properties"];
        assert!(
            wallets.get("next_cursor").is_none(),
            "the wallets are never paged"
        );
    }

    #[test]
    fn every_operation_answers_the_common_problems() {
        let spec = published();
        for (method, path, operation) in operations(&spec) {
            for (status, _, _) in COMMON_PROBLEMS {
                let response = &operation["responses"][*status];
                let response = response["$ref"]
                    .as_str()
                    .and_then(|reference| resolves(&spec, reference))
                    .unwrap_or(response);
                assert_eq!(
                    response["content"][PROBLEM_CONTENT_TYPE]["schema"]["$ref"],
                    "#/components/schemas/Problem",
                    "{method} {path} {status}"
                );
            }
        }
    }

    #[test]
    fn the_common_problems_document_their_headers() {
        let spec = published();
        let responses = &spec["components"]["responses"];
        assert_eq!(
            responses["Unauthorized"]["headers"]["WWW-Authenticate"]["schema"]["type"],
            "string"
        );
        for name in [
            "Retry-After",
            "RateLimit-Limit",
            "RateLimit-Remaining",
            "RateLimit-Reset",
            "RateLimit-Policy",
            "RateLimit",
            VERSION_HEADER,
        ] {
            assert!(
                responses["TooManyRequests"]["headers"][name].is_object(),
                "{name}"
            );
        }
        let marker =
            spec["components"]["parameters"][IDEMPOTENCY_KEY_HEADER]["x-replay-marker"]["$ref"]
                .as_str()
                .expect("the marker is named");
        let marker = resolves(&spec, marker).expect("the marker resolves");
        assert!(marker["properties"]["id"].is_object());
    }

    /// An SDK names its methods by the operation ids: none carries a version, and a PATCH whose
    /// body is a merge patch says so.
    #[test]
    fn operation_ids_and_patches_read_as_sdk_methods() {
        let spec = published();
        for (method, path, operation) in operations(&spec) {
            let id = operation["operationId"].as_str().expect("an operation id");
            assert!(
                !id.split('_').any(|word| word.len() > 1
                    && word.starts_with('v')
                    && word[1..].parse::<u32>().is_ok()),
                "{method} {path}: {id}"
            );
            if method == "patch" {
                let content = operation["requestBody"]["content"]
                    .as_object()
                    .expect("a body");
                let declared: Vec<&String> = content.keys().collect();
                let expected = if path.ends_with("/rates") {
                    "application/json"
                } else {
                    "application/merge-patch+json"
                };
                assert_eq!(declared, vec![expected], "{method} {path}");
            }
        }
        let finalize = &spec["paths"]["/api/v2/invoices/{id}/finalize"]["post"];
        assert_eq!(finalize["operationId"], "finalize_invoice");
        assert!(
            spec["paths"]["/api/v2/customers/{id_or_alias}"]
                .get("delete")
                .is_none()
        );
    }

    #[test]
    fn every_object_a_body_holds_is_closed() {
        let spec = published();
        let root = spec.as_object().expect("a document");
        let mut flattened = Vec::new();
        for schema in spec["components"]["schemas"]
            .as_object()
            .expect("schemas")
            .values()
        {
            schema_all_of(schema, &mut flattened);
        }
        let held = body_schemas(root);
        assert!(held.contains("CreateSubscription") && held.contains("PlanDraft"));
        for name in held {
            let schema = &spec["components"]["schemas"][&name];
            if schema.get("properties").is_none()
                || flattened
                    .iter()
                    .any(|reference| reference == &schema_ref(&name))
            {
                continue;
            }
            assert_eq!(schema["additionalProperties"], false, "{name}");
        }
    }

    #[test]
    fn every_enum_a_response_holds_is_open() {
        let spec = published();
        let root = spec.as_object().expect("a document");
        let event_types = &spec["components"]["schemas"]["EventType"];
        assert!(event_types.get("enum").is_none(), "{event_types}");
        assert!(
            event_types[EXTENSIBLE_ENUM]
                .as_array()
                .is_some_and(|values| values.contains(&json!("invoice.paid")))
        );
        let mut tags = 0;
        for name in response_schemas(root) {
            let mut enums = Vec::new();
            enums_of(&spec["components"]["schemas"][&name], &mut enums);
            for values in enums {
                assert_eq!(values.len(), 1, "{name}: {values:?}");
                tags += 1;
            }
        }
        assert!(tags > 0, "the tags of tagged unions stay closed");
        let mut parameters = Vec::new();
        for (_, _, operation) in operations(&spec) {
            if let Some(listed) = operation.get("parameters") {
                enums_of(listed, &mut parameters);
            }
        }
        assert!(
            parameters.iter().any(|values| values.len() > 1),
            "what a request sends stays closed"
        );
    }

    fn enums_of<'a>(value: &'a Value, out: &mut Vec<&'a Vec<Value>>) {
        match value {
            Value::Object(object) => {
                if let Some(values) = object.get("enum").and_then(Value::as_array) {
                    out.push(values);
                }
                object.values().for_each(|value| enums_of(value, out));
            }
            Value::Array(items) => items.iter().for_each(|value| enums_of(value, out)),
            _ => {}
        }
    }

    #[test]
    fn every_reference_resolves() {
        fn references<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
            match value {
                Value::Object(map) => {
                    for (key, value) in map {
                        match (key.as_str(), value) {
                            ("$ref", Value::String(reference)) => out.push(reference),
                            _ => references(value, out),
                        }
                    }
                }
                Value::Array(items) => items.iter().for_each(|item| references(item, out)),
                _ => {}
            }
        }
        let spec = published();
        let mut found = Vec::new();
        references(&spec, &mut found);
        let dangling: BTreeSet<&str> = found
            .into_iter()
            .filter(|reference| resolves(&spec, reference).is_none())
            .collect();
        assert!(dangling.is_empty(), "{dangling:?}");
    }

    /// A generator makes one type per variant: each tag maps to a schema of its own, which
    /// exists, declares that tag, and is a member of the union; no member is left inline.
    #[test]
    fn every_discriminator_maps_each_tag_to_its_own_variant() {
        let spec = published();
        let schemas = spec["components"]["schemas"].as_object().expect("schemas");
        for (union, schema) in schemas {
            let Some(discriminator) = schema.get("discriminator") else {
                continue;
            };
            let property = discriminator["propertyName"].as_str().expect("a property");
            let mapping = discriminator["mapping"].as_object().expect("a mapping");
            let members: BTreeSet<&str> = schema["oneOf"]
                .as_array()
                .expect("oneOf")
                .iter()
                .map(|member| {
                    member["$ref"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{union}: a member is inline: {member}"))
                })
                .collect();
            let targets: BTreeSet<&str> = mapping.values().filter_map(Value::as_str).collect();
            assert_eq!(
                targets.len(),
                mapping.len(),
                "{union}: two tags map to one schema"
            );
            assert_eq!(targets, members, "{union}: every member is mapped");
            for (tag, target) in mapping {
                let target = target.as_str().expect("a reference");
                let variant = resolves(&spec, target)
                    .unwrap_or_else(|| panic!("{union}: {tag} maps to the missing {target}"));
                if let Some(declared) = variant["properties"].get(property) {
                    assert_eq!(declared["enum"], json!([tag]), "{union}: {target}");
                }
            }
        }
        for name in [
            "CalendarReset",
            "FixedWindowReset",
            "SlidingWindowReset",
            "BillingCycleReset",
            "NeverReset",
            "OnValue",
            "OffValue",
            "LimitValue",
            "UnlimitedValue",
            "NumberValue",
            "TextValue",
            "JsonValue",
            "BooleanFeature",
            "CouponDuration",
            "CouponStatus",
        ] {
            assert!(schemas.contains_key(name), "{name}");
        }
        for gone in ["Window", "ResetUnit", "Box", "Usage"] {
            assert!(!schemas.contains_key(gone), "{gone}");
        }
        assert_eq!(
            schemas["CalendarReset"]["properties"]["interval"]["minimum"].as_f64(),
            Some(1.0)
        );
    }

    #[test]
    fn bodies_v1_shares_take_v2_names() {
        let unfinished = crate::openapi::spec_json(&crate::v2::spec(&EditionSpec::core()));
        let before = unfinished["components"]["schemas"]
            .as_object()
            .expect("schemas");
        for (from, to) in RENAMED_SCHEMAS {
            assert!(
                !before.contains_key(*to),
                "{to} is taken: {from} cannot take it"
            );
        }
        let spec = published();
        let schemas = spec["components"]["schemas"].as_object().expect("schemas");
        assert!(
            schemas.contains_key("IngestEvents") && !schemas.contains_key("IngestEventsRequest")
        );
        assert_eq!(
            spec["paths"]["/api/v2/events/ingest"]["post"]["requestBody"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/IngestEvents"
        );
        let names: Vec<&String> = schemas.keys().collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "schemas stay sorted");
    }

    /// Where each example of the document is, the schema it shows and itself: a request body's
    /// and a response's, `example` or each of `examples`.
    fn examples(spec: &Value) -> Vec<(String, &Value, &Value)> {
        let mut found = Vec::new();
        for (method, path, operation) in operations(spec) {
            let id = operation["operationId"].as_str().unwrap_or(path);
            let mut media = Vec::new();
            if let Some(content) = operation
                .get("requestBody")
                .and_then(|body| body["content"].as_object())
            {
                media.extend(
                    content
                        .iter()
                        .map(|(kind, media)| (format!("request {kind}"), media)),
                );
            }
            for (status, response) in operation["responses"].as_object().expect("responses") {
                if let Some(content) = response["content"].as_object() {
                    media.extend(
                        content
                            .iter()
                            .map(|(kind, media)| (format!("{status} {kind}"), media)),
                    );
                }
            }
            for (at, media) in media {
                let schema = &media["schema"];
                if let Some(example) = media.get("example") {
                    found.push((format!("{method} {id} {at}"), schema, example));
                }
                for (name, example) in media["examples"].as_object().into_iter().flatten() {
                    found.push((
                        format!("{method} {id} {at} {name}"),
                        schema,
                        &example["value"],
                    ));
                }
            }
        }
        found
    }

    /// The properties `schema` declares, its `allOf` parts' included, followed through
    /// references.
    fn declared<'a>(spec: &'a Value, schema: &'a Value, out: &mut BTreeSet<&'a str>) {
        let schema = schema["$ref"]
            .as_str()
            .and_then(|reference| resolves(spec, reference))
            .unwrap_or(schema);
        out.extend(
            schema["properties"]
                .as_object()
                .into_iter()
                .flat_map(|properties| properties.keys().map(String::as_str)),
        );
        for part in schema["allOf"].as_array().into_iter().flatten() {
            declared(spec, part, out);
        }
    }

    fn is_type(value: &Value, kind: &str) -> bool {
        match kind {
            "null" => value.is_null(),
            "boolean" => value.is_boolean(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "string" => value.is_string(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => true,
        }
    }

    /// Checks `value` against `schema`, structurally: references, `allOf`, `oneOf` (by its
    /// discriminator when it has one, else exactly one member) and `anyOf`, types, enums,
    /// required properties, items, bounds and the formats the document uses (dates, decimals,
    /// ids by their prefix). An example names no property its schema does not declare, unless
    /// `additionalProperties` holds a schema: stricter than JSON Schema, so a misspelled
    /// optional field is caught. `open` is set for an `allOf` part, its siblings' properties.
    fn check(
        spec: &Value,
        schema: &Value,
        value: &Value,
        at: &str,
        open: Option<&BTreeSet<&str>>,
        errors: &mut Vec<String>,
    ) {
        if let Some(reference) = schema["$ref"].as_str() {
            match resolves(spec, reference) {
                Some(target) => check(spec, target, value, at, open, errors),
                None => errors.push(format!("{at}: {reference} does not resolve")),
            }
            return;
        }
        if let Some(parts) = schema["allOf"].as_array() {
            let mut all = BTreeSet::new();
            declared(spec, schema, &mut all);
            if let Some(open) = open {
                all.extend(open.iter().copied());
            }
            for part in parts {
                check(spec, part, value, at, Some(&all), errors);
            }
        }
        if let Some(members) = schema["oneOf"].as_array() {
            let tag = schema["discriminator"]["propertyName"]
                .as_str()
                .and_then(|property| value.get(property))
                .and_then(Value::as_str);
            match (tag, schema["discriminator"]["mapping"].as_object()) {
                (Some(tag), Some(mapping)) => match mapping.get(tag) {
                    Some(target) => check(spec, &json!({"$ref": target}), value, at, open, errors),
                    None => errors.push(format!("{at}: no variant is tagged `{tag}`")),
                },
                _ => {
                    let matching: Vec<Vec<String>> = members
                        .iter()
                        .map(|member| {
                            let mut refused = Vec::new();
                            check(spec, member, value, at, open, &mut refused);
                            refused
                        })
                        .collect();
                    let matched = matching.iter().filter(|refused| refused.is_empty()).count();
                    if matched != 1 {
                        errors.push(format!(
                            "{at}: {matched} members of oneOf match {value}: {matching:?}"
                        ));
                    }
                }
            }
        }
        if let Some(members) = schema["anyOf"].as_array() {
            let mut refusals = Vec::new();
            let matched = members.iter().any(|member| {
                let mut refused = Vec::new();
                check(spec, member, value, at, open, &mut refused);
                refusals.extend(refused.iter().cloned());
                refused.is_empty()
            });
            if !matched {
                errors.push(format!("{at}: no member of anyOf matches: {refusals:?}"));
            }
        }
        let types: Vec<&str> = match &schema["type"] {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !types.is_empty() && !types.iter().any(|kind| is_type(value, kind)) {
            errors.push(format!("{at}: {value} is not {types:?}"));
            return;
        }
        // An example names a value an open enum lists, as a closed one's.
        if let Some(allowed) = schema["enum"]
            .as_array()
            .or_else(|| schema[EXTENSIBLE_ENUM].as_array())
            && !allowed.contains(value)
        {
            errors.push(format!("{at}: {value} is not one of {allowed:?}"));
        }
        if let (Some(bound), Some(number)) = (schema["minimum"].as_f64(), value.as_f64())
            && number < bound
        {
            errors.push(format!("{at}: {number} is below {bound}"));
        }
        if let (Some(bound), Some(number)) = (schema["maximum"].as_f64(), value.as_f64())
            && number > bound
        {
            errors.push(format!("{at}: {number} is above {bound}"));
        }
        if let Some(text) = value.as_str() {
            check_format(schema, text, at, errors);
        }
        match value {
            Value::Object(object) => {
                for required in schema["required"].as_array().into_iter().flatten() {
                    let required = required.as_str().unwrap_or_default();
                    if !object.contains_key(required) {
                        errors.push(format!("{at}: `{required}` is required"));
                    }
                }
                let properties = schema["properties"].as_object();
                let extra = &schema["additionalProperties"];
                for (key, item) in object {
                    let at = format!("{at}/{key}");
                    match properties.and_then(|properties| properties.get(key)) {
                        Some(property) => check(spec, property, item, &at, None, errors),
                        None if extra.is_object() => check(spec, extra, item, &at, None, errors),
                        None if extra == &Value::Bool(true) => {}
                        None if properties.is_some()
                            && !open.is_some_and(|open| open.contains(key.as_str())) =>
                        {
                            errors.push(format!("{at}: not a property of its schema"));
                        }
                        None => {}
                    }
                }
            }
            Value::Array(items) => {
                if let Some(schema) = schema.get("items") {
                    for (index, item) in items.iter().enumerate() {
                        check(spec, schema, item, &format!("{at}/{index}"), None, errors);
                    }
                }
            }
            _ => {}
        }
    }

    /// The formats the document gives its strings, checked: an id by the prefix of its schema's
    /// own example.
    fn check_format(schema: &Value, text: &str, at: &str, errors: &mut Vec<String>) {
        let read = match schema["format"].as_str() {
            Some("date") => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
            Some("date-time") => chrono::DateTime::parse_from_rfc3339(text).is_ok(),
            Some("decimal") => text.parse::<rust_decimal::Decimal>().is_ok(),
            Some("MeteroidId") => {
                let prefix = schema["examples"][0]
                    .as_str()
                    .and_then(|example| example.rsplit_once('_'))
                    .map(|(prefix, _)| format!("{prefix}_"));
                prefix.is_none_or(|prefix| {
                    text.strip_prefix(&prefix).is_some_and(|rest| {
                        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric())
                    })
                })
            }
            _ => true,
        };
        if !read {
            errors.push(format!(
                "{at}: `{text}` is not a {}",
                schema["format"].as_str().unwrap_or_default()
            ));
        }
        if let Some(most) = schema["maxLength"].as_u64()
            && text.chars().count() as u64 > most
        {
            errors.push(format!("{at}: longer than {most}"));
        }
        if let Some(least) = schema["minLength"].as_u64()
            && (text.chars().count() as u64) < least
        {
            errors.push(format!("{at}: shorter than {least}"));
        }
    }

    /// Integrators read the v2 and client documents: none of the engine's own words, the v1
    /// document's list and the entitlement engine's.
    #[test]
    fn the_documents_say_no_engine_jargon() {
        let client = crate::client::client_api(utoipa::openapi::OpenApiBuilder::new().build()).1;
        for (document, spec) in [
            ("v2", published()),
            ("client", crate::openapi::spec_json(&client)),
        ] {
            let published = serde_json::to_string(&spec).expect("a document");
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
                "fence",
                "lineage",
                "engine",
                "readout",
                "own level",
                "resource_override",
                "FeatureScope",
                "PathLevel",
                "RowLevel",
                "PathResource",
            ] {
                assert!(
                    !published.contains(jargon),
                    "the {document} document says {jargon:?}"
                );
            }
        }
    }

    #[test]
    fn every_example_reads_as_its_schema() {
        let spec = published();
        let examples = examples(&spec);
        assert!(!examples.is_empty());
        let mut errors = Vec::new();
        for (at, schema, example) in examples {
            check(&spec, schema, example, &at, None, &mut errors);
        }
        assert!(errors.is_empty(), "{errors:#?}");
    }

    /// The check refuses what an example could get wrong.
    #[test]
    fn the_example_check_refuses_a_wrong_example() {
        let spec = published();
        let customer = json!({"$ref": "#/components/schemas/Customer"});
        let refused = |value: Value| {
            let mut errors = Vec::new();
            check(&spec, &customer, &value, "customer", None, &mut errors);
            errors
        };
        let mut good = crate::v2::examples::response("get_customer");
        assert_eq!(refused(good.clone()), Vec::<String>::new());
        good["nickname"] = json!("nw");
        assert!(refused(good.clone())[0].contains("nickname"));
        good.as_object_mut().unwrap().remove("nickname");
        good["type"] = json!("partnership");
        assert!(!refused(good.clone()).is_empty());
        good["type"] = json!("company");
        good["id"] = json!("sub_2LjsL3pvKFIUjovfWsyd2S");
        assert!(!refused(good.clone()).is_empty());
        good.as_object_mut().unwrap().remove("id");
        assert!(refused(good)[0].contains("`id` is required"));
    }

    /// The first integration (customers, plans, subscriptions, invoices, events, webhook
    /// endpoints) shows a request and a response for each operation: a JSON body sent has its
    /// example, and so has its JSON success.
    #[test]
    fn the_first_integration_is_shown_by_example() {
        let spec = published();
        let mut missing = Vec::new();
        for (method, path, operation) in operations(&spec) {
            let id = operation["operationId"].as_str().unwrap_or(path);
            if !crate::v2::examples::SHOWN.contains(&id) {
                continue;
            }
            let shown =
                |media: &Value| media.get("example").is_some() || media.get("examples").is_some();
            let sent = operation
                .get("requestBody")
                .and_then(|body| body["content"].as_object());
            for (kind, media) in sent.into_iter().flatten() {
                if kind.ends_with("json") && !shown(media) {
                    missing.push(format!("{method} {id} request {kind}"));
                }
            }
            for (status, response) in operation["responses"].as_object().expect("responses") {
                if !status.starts_with('2') {
                    continue;
                }
                for (kind, media) in response["content"].as_object().into_iter().flatten() {
                    if kind.ends_with("json") && !shown(media) {
                        missing.push(format!("{method} {id} {status} {kind}"));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
        let ids: BTreeSet<&str> = operations(&spec)
            .iter()
            .filter_map(|(_, _, operation)| operation["operationId"].as_str())
            .collect();
        for id in crate::v2::examples::SHOWN {
            assert!(ids.contains(id), "{id} is no operation");
        }
    }

    /// A generator reads a field's docstring off its property: an optional reference's is
    /// there, and a variant's tag says what it names.
    #[test]
    fn properties_carry_their_descriptions() {
        let spec = published();
        let schemas = &spec["components"]["schemas"];
        let address = &schemas["Customer"]["properties"]["billing_address"];
        assert!(address["description"].is_string(), "{address}");
        assert!(
            address["oneOf"]
                .as_array()
                .expect("oneOf")
                .iter()
                .all(|member| member.get("description").is_none())
        );
        assert_eq!(
            schemas["PerUnitModel"]["properties"]["type"]["description"],
            "`per_unit`: names this variant of `PriceModel`."
        );
        assert!(schemas["CustomerList"]["description"].is_string());
        assert!(schemas["ProductRef"]["description"].is_string());
    }

    #[test]
    fn webhook_payloads_are_v2s() {
        let spec = crate::v2::published_spec_json(
            &crate::v2::spec(&EditionSpec::core()),
            &EditionSpec::core(),
        );
        let schemas = spec["components"]["schemas"].as_object().unwrap();
        let webhooks = spec["webhooks"].as_object().expect("webhooks");
        assert!(
            webhooks.contains_key("invoice.paid"),
            "{:?}",
            webhooks.keys()
        );
        for (name, item) in webhooks {
            let reference =
                item["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{name} names its payload"));
            let envelope = &schemas[reference.trim_start_matches(SCHEMA_REF)];
            assert_eq!(
                envelope["required"],
                json!(["id", "type", "created_at", "api_version", "data"]),
                "{name}"
            );
            let data = envelope["properties"]["data"]["$ref"].as_str().unwrap();
            assert!(
                schemas.contains_key(data.trim_start_matches(SCHEMA_REF)),
                "{name}: {data}"
            );
        }
        assert!(
            !schemas.keys().any(|name| name.starts_with("V1")),
            "v1's payload schemas stay in v1's document"
        );
        let names: Vec<&String> = schemas.keys().collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "schemas stay sorted");
        assert_eq!(
            schemas["InvoiceEvent"]["properties"]["data"]["$ref"], "#/components/schemas/Invoice",
            "an invoice's events carry the invoice as its GET reads it"
        );
        let tag = spec["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tag| tag["name"] == "Webhooks")
            .unwrap();
        assert!(
            tag["description"]
                .as_str()
                .unwrap()
                .ends_with(crate::v2::webhook_payloads::VERSIONS_NOTE)
        );
    }
}

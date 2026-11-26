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
//!   `401` with its `WWW-Authenticate`, the `429` with the rate limit's headers.
//! - Every POST but a preview and the entitlement check carries `x-idempotency: required |
//!   optional`; every list paged by cursor carries `x-pagination`.
//! - Every object a JSON body holds is closed (`additionalProperties: false`): the body extractor
//!   refuses a field it does not have (`v2::extract`). One a response flattens (`allOf`) is left
//!   open, as closing it would refuse the response's own fields.
use crate::v2::envelope::{CALLER_REQUEST_ID_HEADER, REQUEST_ID_HEADER};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, REPLAYED_HEADER};
use crate::v2::pagination::{DEFAULT_LIMIT, MAX_LIMIT};
use crate::v2::problem::PROBLEM_CONTENT_TYPE;
use crate::v2::version::{VERSION_HEADER, VERSIONS};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Whether an operation takes an `Idempotency-Key`: `required` or `optional`. A GET, a PATCH, a
/// PUT, a DELETE, a preview and the entitlement check take none, and carry no such extension.
pub const IDEMPOTENCY_EXTENSION: &str = "x-idempotency";

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

const METHODS: [&str; 5] = ["get", "put", "post", "delete", "patch"];

const KEY_ORDER: [&str; 6] = ["openapi", "info", "servers", "paths", "components", "tags"];

/// Finishes the JSON of the routes' document (`openapi::spec_json`).
pub fn finish(root: &mut Map<String, Value>) {
    rename_schemas(root);
    close_bodies(root);
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
    let mut rest = std::mem::take(root);
    for key in KEY_ORDER {
        if let Some(value) = rest.remove(key) {
            root.insert(key.to_owned(), value);
        }
    }
    root.extend(rest);
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
    for item in root
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::values)
    {
        for method in METHODS {
            let content = item
                .get(method)
                .and_then(|operation| operation.pointer("/requestBody/content"))
                .and_then(Value::as_object);
            for (content_type, media) in content.into_iter().flatten() {
                if content_type.ends_with("json") {
                    references(media, &mut found);
                }
            }
        }
    }
    let schemas = root
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object);
    let mut held = BTreeSet::new();
    let mut next: Vec<String> = found
        .iter()
        .map(|reference| reference.to_string())
        .collect();
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
    let keyed = method == "post"
        && !path.ends_with("/preview")
        && !crate::v2::idempotency::reads_only(path)
        && !path.starts_with(crate::v2::OAUTH_PREFIX);
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
                parameter
                    .entry("description")
                    .or_insert_with(|| json!("The `next_cursor` of the previous page."));
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
            "Taken by a POST whose `{IDEMPOTENCY_EXTENSION}` is `required` or `optional`, kept 24 \
             hours; a `required` one is refused without it (`400 idempotency_key_required`). Only \
             a POST takes one: a PATCH, a PUT and a DELETE are sent again as they are. A retry \
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

/// v1's webhook payloads, until v2 has payloads of its own (ADR-0013 §15): the `webhooks` of the
/// edition's v1 document, with every schema they reach. A schema whose name v2 gives another
/// shape is copied as `V1<Name>`, and every reference to it rewritten, so a generator sees both.
/// Each payload's description says it is v1's ([`V1_PAYLOAD_NOTE`]).
pub fn attach_v1_webhooks(v2: &mut Map<String, Value>, v1: &Map<String, Value>) {
    let Some(mut webhooks) = v1.get("webhooks").cloned() else {
        return;
    };
    let Some(v1_schemas) = v1
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
    else {
        return;
    };
    let mut needed = BTreeSet::new();
    let mut stack = schema_refs(&webhooks);
    while let Some(name) = stack.pop() {
        if needed.insert(name.clone())
            && let Some(schema) = v1_schemas.get(&name)
        {
            stack.extend(schema_refs(schema));
        }
    }
    let v2_schemas = v2
        .entry("components")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("components is an object")
        .entry("schemas")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("schemas is an object");
    let renames: BTreeMap<String, String> = needed
        .iter()
        .filter(|name| {
            v2_schemas
                .get(*name)
                .is_some_and(|existing| Some(existing) != v1_schemas.get(*name))
        })
        .map(|name| (name.clone(), format!("V1{name}")))
        .collect();
    for name in &needed {
        let Some(schema) = v1_schemas.get(name) else {
            continue;
        };
        let mut schema = schema.clone();
        rename_refs(&mut schema, &renames);
        let target = renames.get(name).cloned().unwrap_or_else(|| name.clone());
        v2_schemas.entry(target).or_insert(schema);
    }
    rename_refs(&mut webhooks, &renames);
    for operation in webhooks
        .as_object_mut()
        .into_iter()
        .flat_map(|webhooks| webhooks.values_mut())
        .filter_map(Value::as_object_mut)
        .flat_map(|item| item.values_mut())
        .filter_map(Value::as_object_mut)
    {
        let described = match operation.get("description").and_then(Value::as_str) {
            Some(description) => format!("{description}\n\n{V1_PAYLOAD_NOTE}"),
            None => V1_PAYLOAD_NOTE.to_owned(),
        };
        operation.insert("description".to_owned(), Value::String(described));
    }
    v2.insert("webhooks".to_owned(), webhooks);
}

/// What every payload in v2's document says of its shape.
pub const V1_PAYLOAD_NOTE: &str = "Rendered in API v1's shape (v1's field names and enum \
     spellings), whatever API created the endpoint. An endpoint's `api_version` will pin its \
     payloads' shape when v2 payloads ship.";

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

fn rename_refs(value: &mut Value, renames: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(renamed) = text
                .strip_prefix(SCHEMA_REF)
                .and_then(|name| renames.get(name))
            {
                *text = format!("{SCHEMA_REF}{renamed}");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rename_refs(item, renames)),
        Value::Object(map) => map.values_mut().for_each(|item| rename_refs(item, renames)),
        _ => {}
    }
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
        for (method, path, operation) in &operations {
            let taken = operation.get(IDEMPOTENCY_EXTENSION).and_then(Value::as_str);
            if *method != "post"
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
    fn webhook_payloads_say_they_are_v1s() {
        let spec = crate::v2::published_spec_json(
            &crate::v2::spec(&EditionSpec::core()),
            &EditionSpec::core(),
        );
        let webhooks = spec["webhooks"].as_object().expect("webhooks");
        assert!(!webhooks.is_empty());
        for (name, item) in webhooks {
            let description = item["post"]["description"].as_str().unwrap_or_default();
            assert!(
                description.ends_with(V1_PAYLOAD_NOTE),
                "{name}: {description}"
            );
        }
    }
}

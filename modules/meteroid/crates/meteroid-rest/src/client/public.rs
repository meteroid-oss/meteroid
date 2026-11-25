//! The public routes, read with a publishable key: what a tenant shows anyone, its pricing pages.
//! Their answers are the same for every visitor of a key, so they are cacheable: `Cache-Control:
//! public` for a minute, varying on the key and the origin, and a weak `ETag` answered `304` when
//! it still matches.
use crate::AppState;
use crate::client::AuthorizedAsPublishable;
use crate::v2::extract::{Path, Query};
use crate::v2::pricing_pages::{self, PricingPage, PricingPageQuery};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Serialize;
use std::hash::{DefaultHasher, Hasher};

const CACHE_CONTROL: &str = "public, max-age=60, stale-while-revalidate=300";
/// A shared cache keys the answer on the key (`Authorization`), so two tenants' pages of the same
/// family code never mix; the origin since the CORS headers follow it.
const VARY: &str = "authorization, origin";

/// Get a pricing page
///
/// A product family's pricing page, as API v2's `GET /api/v2/pricing-pages/{family_code}` reads
/// it: its self-serve ladder priced in one currency and one option, card by card. Only what the
/// public page shows: a sales-led plan's hidden price is left out. Authenticated with a
/// publishable key, from one of its allowed origins when it lists any.
#[utoipa::path(
    get,
    tag = "Pricing pages",
    path = "/api/client/v1/pricing-pages/{family_code}",
    params(
        ("family_code" = String, Path, description = "Product family code (or id)"),
        PricingPageQuery,
    ),
    responses(
        (status = 200, description = "The pricing page", body = PricingPage),
        (status = 304, description = "Unchanged since the `ETag` sent in `If-None-Match`"),
        (status = 401, description = "Missing or unknown publishable key, or another credential", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The key is not accepted from the request's origin", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "No such product family", body = Problem, content_type = "application/problem+json"),
    ),
    security(("publishable_key" = []))
)]
pub async fn get_pricing_page(
    Extension(auth): Extension<AuthorizedAsPublishable>,
    State(state): State<AppState>,
    Path(family): Path<String>,
    Query(query): Query<PricingPageQuery>,
    headers: HeaderMap,
) -> Result<Response, Problem> {
    let page = pricing_pages::render(&state, auth.tenant_id, &family, query).await?;
    Ok(cacheable(&headers, &page))
}

/// The JSON answer with its cache headers, or `304` when the caller's `If-None-Match` names it.
fn cacheable<T: Serialize>(request: &HeaderMap, body: &T) -> Response {
    let Ok(bytes) = serde_json::to_vec(body) else {
        return Problem::internal().into_response();
    };
    let etag = etag_of(&bytes);
    let matches = request
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|tags| {
            tags.split(',')
                .map(str::trim)
                .any(|tag| tag == "*" || tag == etag)
        });

    let mut response = if matches {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            bytes,
        )
            .into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    headers.append(header::VARY, HeaderValue::from_static(VARY));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    response
}

/// A weak validator of the body: equal bodies, equal tags, within a release.
fn etag_of(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    format!("W/\"{:016x}-{:x}\"", hasher.finish(), bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_page_answers_not_modified_with_its_tag() {
        let page = serde_json::json!({ "family": "cloud", "cards": [] });
        let first = cacheable(&HeaderMap::new(), &page);
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first.headers().get(header::ETAG).unwrap().clone();
        assert!(etag.to_str().unwrap().starts_with("W/\""));
        assert_eq!(
            first.headers().get(header::CACHE_CONTROL).unwrap(),
            CACHE_CONTROL
        );

        let mut revalidation = HeaderMap::new();
        revalidation.insert(header::IF_NONE_MATCH, etag.clone());
        let again = cacheable(&revalidation, &page);
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(again.headers().get(header::ETAG), Some(&etag));

        let changed = serde_json::json!({ "family": "cloud", "cards": [1] });
        assert_eq!(cacheable(&revalidation, &changed).status(), StatusCode::OK);
    }
}

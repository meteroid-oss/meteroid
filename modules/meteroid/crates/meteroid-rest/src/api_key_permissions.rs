//! Which resource, and whether read or write access to it, a REST request needs from a
//! restricted secret key.

use http::Method;
use meteroid_store::domain::api_key_permissions::ApiResource;

use crate::auth::is_restricted_money_route;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Any key: who the credentials are.
    AnyKey,
    Resource {
        resource: ApiResource,
        write: bool,
    },
    /// Write access to each of them: what the route hands out spans them all.
    AllWrite(&'static [ApiResource]),
    /// Not covered by any resource: only an unrestricted key reaches it.
    Unrestricted,
}

/// A portal token acts for the customer on their subscriptions and payments too.
const PORTAL_TOKEN: &[ApiResource] = &[
    ApiResource::Customers,
    ApiResource::Subscriptions,
    ApiResource::Payments,
];

const CATALOG: &[&str] = &[
    "plans",
    "plan-versions",
    "products",
    "product-families",
    "product_families",
    "addons",
    "add-ons",
    "coupons",
    "features",
    "rate-cards",
    "metrics",
    "credit-units",
    "resource-types",
    "custom-property-definitions",
    "managed-quantities",
    "pricing-pages",
    "trial-offers",
];

const CUSTOMER_MONEY: &[&str] = &[
    "top-ups",
    "credit-grants",
    "budgets",
    "wallets",
    "credit-ledger",
];

/// POSTs that compute without changing anything, matched on the whole template so a code
/// named `preview` in a write route stays a write.
fn is_read_only_post(segments: &[&str]) -> bool {
    matches!(
        segments,
        [
            "subscriptions",
            _,
            "plan-change" | "amendments" | "cancel",
            "preview"
        ] | ["subscriptions", _, "levels" | "quantities", _, "preview"]
            | ["metrics", _, "attribution", "preview"]
            | ["metrics", _, "attribution-preview"]
            | ["customers", _, "entitlements", "check"]
    )
}

/// GETs that reveal a secret: reading one is as good as holding write access.
fn reveals_secret(segments: &[&str]) -> bool {
    matches!(
        segments,
        ["webhooks", "endpoints", _, "secret"] | ["licenses", _, "activation-code"]
    )
}

pub fn requirement(method: &Method, path: &str) -> Requirement {
    let Some(rest) = path
        .strip_prefix("/api/v1/")
        .or_else(|| path.strip_prefix("/api/v2/"))
    else {
        return Requirement::Unrestricted;
    };
    let segments: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let top = segments.first().copied().unwrap_or_default();
    let sub = segments.get(2).copied().unwrap_or_default();

    let read = (method.is_safe() && !reveals_secret(&segments))
        || (*method == Method::POST && is_read_only_post(&segments));
    let write = !read;
    let resource = |resource| Requirement::Resource { resource, write };

    if is_restricted_money_route(method, path) {
        return resource(ApiResource::Payments);
    }

    match top {
        "me" => Requirement::AnyKey,
        "customers" => match sub {
            "portal-token" => Requirement::AllWrite(PORTAL_TOKEN),
            "entitlements" | "entitlement-overrides" => resource(ApiResource::Entitlements),
            "usage" => resource(ApiResource::Usage),
            s if CUSTOMER_MONEY.contains(&s) => resource(ApiResource::Payments),
            _ => resource(ApiResource::Customers),
        },
        "subscriptions" => match sub {
            "entitlements" => resource(ApiResource::Entitlements),
            "usage" => resource(ApiResource::Usage),
            _ => resource(ApiResource::Subscriptions),
        },
        "checkout-sessions" | "cancellation-sessions" => resource(ApiResource::Subscriptions),
        "invoices" | "credit-notes" => match sub {
            "payments" => resource(ApiResource::Payments),
            _ => resource(ApiResource::Invoices),
        },
        t if CATALOG.contains(&t) => resource(ApiResource::Catalog),
        "usage" | "events" => resource(ApiResource::Usage),
        "entitlements" | "licenses" | "licensing" => resource(ApiResource::Entitlements),
        "webhooks" => resource(ApiResource::Webhooks),
        "batch-jobs" => resource(ApiResource::BatchJobs),
        _ => Requirement::Unrestricted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApiResource::*;

    fn needs(method: Method, path: &str) -> (ApiResource, bool) {
        match requirement(&method, path) {
            Requirement::Resource { resource, write } => (resource, write),
            other => panic!("{method} {path}: {other:?}"),
        }
    }

    #[test]
    fn reads_and_writes_follow_the_method() {
        assert_eq!(needs(Method::GET, "/api/v2/customers"), (Customers, false));
        assert_eq!(needs(Method::POST, "/api/v2/customers"), (Customers, true));
        assert_eq!(
            needs(Method::PATCH, "/api/v1/customers/cus_1"),
            (Customers, true)
        );
        assert_eq!(needs(Method::DELETE, "/api/v2/plans/pro"), (Catalog, true));
    }

    #[test]
    fn previews_and_checks_are_reads() {
        assert_eq!(
            needs(
                Method::POST,
                "/api/v2/subscriptions/sub_1/plan-change/preview"
            ),
            (Subscriptions, false)
        );
        assert_eq!(
            needs(Method::POST, "/api/v2/customers/cus_1/entitlements/check"),
            (Entitlements, false)
        );
        assert_eq!(
            needs(Method::POST, "/api/v1/metrics/m_1/attribution-preview"),
            (Catalog, false)
        );
        assert_eq!(
            needs(
                Method::POST,
                "/api/v2/customers/cus_1/entitlements/api/usage"
            ),
            (Entitlements, true)
        );
    }

    #[test]
    fn a_write_route_with_a_code_named_preview_stays_a_write() {
        for path in [
            "/api/v2/subscriptions/sub_1/levels/preview",
            "/api/v2/subscriptions/sub_1/quantities/preview",
        ] {
            assert_eq!(needs(Method::POST, path), (Subscriptions, true), "{path}");
        }
        assert_eq!(
            needs(
                Method::POST,
                "/api/v2/subscriptions/sub_1/levels/pro/preview"
            ),
            (Subscriptions, false)
        );
    }

    #[test]
    fn reading_a_secret_needs_write_access() {
        assert_eq!(
            needs(Method::GET, "/api/v2/webhooks/endpoints/we_1/secret"),
            (Webhooks, true)
        );
        assert_eq!(
            needs(Method::GET, "/api/v1/licenses/lic_1/activation-code"),
            (Entitlements, true)
        );
        assert_eq!(
            needs(Method::GET, "/api/v2/webhooks/endpoints/we_1"),
            (Webhooks, false)
        );
    }

    #[test]
    fn a_portal_token_needs_what_the_portal_can_do() {
        for path in [
            "/api/v1/customers/cus_1/portal-token",
            "/api/v2/customers/ext:acme/portal-token",
        ] {
            assert_eq!(
                requirement(&Method::POST, path),
                Requirement::AllWrite(PORTAL_TOKEN)
            );
        }
    }

    #[test]
    fn customer_and_subscription_subresources_have_their_own_resource() {
        assert_eq!(
            needs(Method::GET, "/api/v2/customers/ext:acme/usage/costs"),
            (Usage, false)
        );
        assert_eq!(
            needs(Method::GET, "/api/v2/subscriptions/sub_1/entitlements"),
            (Entitlements, false)
        );
        assert_eq!(
            needs(Method::GET, "/api/v2/customers/cus_1/wallets"),
            (Payments, false)
        );
        assert_eq!(
            needs(Method::GET, "/api/v1/customers/cus_1/credit-ledger"),
            (Payments, false)
        );
        assert_eq!(
            needs(Method::POST, "/api/v2/customers/cus_1/top-ups"),
            (Payments, true)
        );
    }

    #[test]
    fn money_writes_need_payments() {
        for path in [
            "/api/v2/invoices/inv_1/mark-paid",
            "/api/v1/invoices/inv_1/payments",
            "/api/v2/invoices/inv_1/payments/pay_1/reverse",
            "/api/v2/credit-notes",
        ] {
            assert_eq!(needs(Method::POST, path), (Payments, true), "{path}");
        }
        assert_eq!(
            needs(Method::GET, "/api/v2/credit-notes/cn_1"),
            (Invoices, false)
        );
        assert_eq!(
            needs(Method::POST, "/api/v2/invoices/inv_1/finalize"),
            (Invoices, true)
        );
    }

    #[test]
    fn event_ingestion_is_a_usage_write() {
        assert_eq!(needs(Method::POST, "/api/v2/events/ingest"), (Usage, true));
    }

    #[test]
    fn me_takes_any_key_and_unmapped_paths_take_an_unrestricted_one() {
        assert_eq!(requirement(&Method::GET, "/api/v2/me"), Requirement::AnyKey);
        for path in [
            "/api/v2/connected-accounts",
            "/api/v2/oauth-apps",
            "/api/v1/customersx",
            "/api/v3/customers",
        ] {
            assert_eq!(
                requirement(&Method::GET, path),
                Requirement::Unrestricted,
                "{path}"
            );
        }
    }

    /// A route added later must be mapped on purpose: restricted keys cannot reach it otherwise.
    #[test]
    fn every_core_route_needs_a_resource() {
        let (_, spec) =
            crate::openapi::edition_spec(&meteroid_rest_common::spec::EditionSpec::core());
        let mut unmapped = vec![];
        for (path, item) in &spec.paths.paths {
            let methods = [
                (Method::GET, item.get.is_some()),
                (Method::POST, item.post.is_some()),
                (Method::PUT, item.put.is_some()),
                (Method::PATCH, item.patch.is_some()),
                (Method::DELETE, item.delete.is_some()),
            ];
            for (method, present) in methods {
                if present && requirement(&method, path) == Requirement::Unrestricted {
                    unmapped.push(format!("{method} {path}"));
                }
            }
        }
        let expected_unrestricted = |route: &String| route.contains("/oauth");
        unmapped.retain(|route| !expected_unrestricted(route));
        assert!(unmapped.is_empty(), "unmapped routes: {unmapped:#?}");
    }
}

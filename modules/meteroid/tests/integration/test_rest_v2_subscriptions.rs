//! API v2 subscriptions (ADR-0013 §10): created by codes, read by external id, listed by cursor,
//! and changed — a seat delta replayed by its Idempotency-Key, a level, a plan change previewed
//! then applied (an immediate upgrade, a downgrade at the period's end), a cancellation revoked
//! then made — with the problems a refused request answers.
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::test_rest_v2::{Answer, Fixture, fixture, moment_from, second_tenant_key, seed_catalog};
use meteroid_grpc::meteroid::api;
use reqwest::StatusCode;
use serde_json::{Value, json};

/// A plan of a platform fee per option, seats of `users` in every option and API usage sold in
/// two levels, offering the `support` add-on.
fn version(platform_monthly: &str, seat: &str) -> Value {
    json!({
        "currency": "EUR",
        "options": [
            {"alias": "monthly", "name": "Monthly"},
            {"alias": "annual", "name": "Annual"}
        ],
        "charges": [
            {
                "product": "platform",
                "preset": "flat_fee",
                "prices": [
                    {"option": "monthly", "currency": "EUR", "cadence": "monthly",
                     "model": {"type": "flat", "amount": platform_monthly}},
                    {"option": "annual", "currency": "EUR", "cadence": "annual",
                     "model": {"type": "flat", "amount": "1990"}}
                ]
            },
            {
                "product": "seat",
                "preset": "seats",
                "quantity": {"type": "managed", "quantity": "users", "min": 1},
                "prices": [
                    {"currency": "EUR", "model": {"type": "per_unit", "unit_price": seat}}
                ]
            },
            {
                "product": "api",
                "preset": "usage_levels",
                "quantity": {"type": "metered", "metric": "bandwidth"},
                "prices": [
                    {"currency": "EUR", "model": {"type": "levels", "levels": [
                        {"included": 1000, "price": "19", "overage_unit_price": "0.02"},
                        {"included": 10000, "price": "99", "overage_unit_price": "0.01"}
                    ]}}
                ]
            }
        ],
        "add_ons": [{"add_on": "support"}]
    })
}

async fn publish(f: &Fixture, code: &str, name: &str, draft: Value) {
    let created = f
        .post(
            "/api/v2/plans",
            json!({"code": code, "name": name, "draft": draft}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let published = f
        .post(&format!("/api/v2/plans/{code}/draft/publish"), json!({}))
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:#}", published.body);
}

/// The catalog of the v2 tests, an add-on, a coupon, and the plans `studio` and `scale` (the
/// dearer one).
pub(crate) async fn seed(f: &Fixture) {
    seed_catalog(f).await;
    let product = f
        .post(
            "/api/v2/products",
            json!({"code": "support", "name": "Support"}),
        )
        .await;
    assert_eq!(product.status, StatusCode::CREATED, "{:#}", product.body);
    let add_on = f
        .post(
            "/api/v2/add-ons",
            json!({
                "code": "support",
                "name": "Support",
                "charge": {
                    "product": "support",
                    "preset": "flat_fee",
                    "prices": [{"currency": "EUR", "model": {"type": "flat", "amount": "29"}}]
                }
            }),
        )
        .await;
    assert_eq!(add_on.status, StatusCode::CREATED, "{:#}", add_on.body);
    let licences = f
        .post(
            "/api/v2/managed-quantities",
            json!({"code": "licences", "name": "Licences"}),
        )
        .await;
    assert_eq!(licences.status, StatusCode::CREATED, "{:#}", licences.body);

    publish(f, "studio", "Studio", version("49", "10")).await;
    publish(f, "scale", "Scale", version("199", "12")).await;

    let auth = meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    clients
        .coupons
        .clone()
        .create_coupon(api::coupons::v1::CreateCouponRequest {
            code: "LAUNCH10".into(),
            description: "Launch".into(),
            discount: Some(api::coupons::v1::CouponDiscount {
                discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Percentage(
                    api::coupons::v1::coupon_discount::PercentageDiscount {
                        percentage: "10".into(),
                    },
                )),
            }),
            reusable: true,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn charge<'a>(subscription: &'a Value, product: &str) -> &'a Value {
    subscription["charges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|charge| charge["product"] == product)
        .unwrap_or_else(|| panic!("no charge of {product}: {subscription:#}"))
}

/// Rules: ADR-0013 §10
#[tokio::test]
async fn test_rest_v2_subscription_created_read_and_listed() {
    let f = fixture().await;
    seed(&f).await;
    // The add-on grants a feature: what a subscription holding it is entitled to.
    let sso = f
        .post(
            "/api/v2/features",
            json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}}),
        )
        .await;
    assert_eq!(sso.status, StatusCode::CREATED, "{:#}", sso.body);
    let granted = f
        .patch(
            "/api/v2/add-ons/support",
            json!({"entitlements": [{"feature": "sso", "value": {"type": "on"}}]}),
        )
        .await;
    assert_eq!(granted.status, StatusCode::OK, "{:#}", granted.body);

    // Created by codes: the customer by its alias, the plan, its option, seats, a level, a price
    // override, an add-on and a coupon.
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-spotify",
            json!({
                "customer": "ext:spotify",
                "plan": "studio",
                "option": "monthly",
                "currency": "EUR",
                "external_id": "crm-spotify-1",
                "quantities": {"users": "5"},
                "levels": {"api": "10000"},
                "charges": {
                    "override": [{"charge": "platform", "price": {"type": "flat", "amount": "39"}}]
                },
                "add_ons": [{"add_on": "support"}],
                "coupons": ["launch10"],
                "invoicing": {"charge_automatically": false, "net_terms": 14, "purchase_order": "PO-7"}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let sub = &created.body;
    let id = sub["id"].as_str().unwrap().to_owned();
    assert_eq!(sub["external_id"], "crm-spotify-1");
    assert_eq!(sub["customer"]["alias"], "spotify");
    assert_eq!(
        plan_named(&sub["plan"]),
        json!({"code": "studio", "version": 1, "option": "monthly"})
    );
    assert!(sub["plan"]["id"].as_str().unwrap().starts_with("plan_"));
    assert!(sub["customer"]["id"].as_str().unwrap().starts_with("cus_"));
    assert_eq!(sub["currency"], "EUR");
    assert_eq!(sub["period"], "monthly");
    assert_eq!(sub["quantities"]["users"]["held"], "5");
    assert_eq!(sub["invoicing"]["net_terms"], 14);
    assert_eq!(sub["invoicing"]["purchase_order"], "PO-7");
    assert_eq!(sub["invoicing"]["charge_automatically"], false);
    assert_eq!(sub["mrr"]["currency"], "EUR");

    let platform = charge(sub, "platform");
    assert_eq!(platform["origin"], "plan");
    assert_eq!(platform["prices"][0]["option"], "*");
    assert_eq!(
        platform["prices"][0]["model"],
        json!({"type": "flat", "amount": "39"}),
        "the price overridden"
    );
    let seats = charge(sub, "seat");
    assert_eq!(
        seats["quantity"],
        json!({"type": "managed", "quantity": "users", "min": 1})
    );
    let api = charge(sub, "api");
    assert_eq!(api["preset"], "usage_levels");
    assert_eq!(api["prices"][0]["model"]["levels"][0]["included"], 10000);
    let support = charge(sub, "support");
    assert_eq!(support["origin"], "add_on");
    assert_eq!(support["add_on"]["code"], "support");
    assert_eq!(sub["add_ons"][0]["add_on"], support["add_on"]);
    assert!(
        sub["add_ons"][0]["add_on"]["id"]
            .as_str()
            .unwrap()
            .starts_with("add_")
    );
    assert_eq!(sub["add_ons"][0]["quantity"], 1);
    assert_eq!(sub["coupons"][0]["coupon"]["code"], "LAUNCH10");
    assert!(
        sub["coupons"][0]["coupon"]["id"]
            .as_str()
            .unwrap()
            .starts_with("cou_")
    );

    // Read by its external id as by its id.
    let by_external = f.get("/api/v2/subscriptions/crm-spotify-1").await;
    assert_eq!(by_external.status, StatusCode::OK, "{:#}", by_external.body);
    assert_eq!(by_external.body["id"], id.as_str());
    let by_id = f.get(&format!("/api/v2/subscriptions/{id}")).await;
    assert_eq!(by_id.body["external_id"], "crm-spotify-1");
    assert_eq!(by_id.body["tax_inclusive"], false);
    assert!(by_id.body.get("resource").is_none(), "{:#}", by_id.body);
    assert!(by_id.body.get("warnings").is_none(), "{:#}", by_id.body);
    assert_eq!(by_id.body["entitlement_overrides"], json!([]));

    // Its summary reads as a list item: its own fields, nothing read beside.
    let summary = f.get("/api/v2/subscriptions/crm-spotify-1/summary").await;
    assert_eq!(summary.status, StatusCode::OK, "{:#}", summary.body);
    assert_eq!(summary.body["id"], id.as_str());
    assert_eq!(summary.body["status"], by_id.body["status"]);
    assert_eq!(summary.body["current_period"], by_id.body["current_period"]);
    assert_eq!(summary.body["mrr"], by_id.body["mrr"]);
    assert_eq!(summary.body["tax_inclusive"], false);
    for detail in ["charges", "quantities", "add_ons", "coupons", "schedule"] {
        assert!(
            summary.body.get(detail).is_none(),
            "{detail}: {:#}",
            summary.body
        );
    }
    assert_eq!(
        f.get("/api/v2/subscriptions/no-such-subscription/summary")
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // What it grants, not paged: the add-on's row, at the customer level.
    let entitlements = f
        .get("/api/v2/subscriptions/crm-spotify-1/entitlements")
        .await;
    assert_eq!(
        entitlements.status,
        StatusCode::OK,
        "{:#}",
        entitlements.body
    );
    assert!(
        entitlements.body.get("has_more").is_none(),
        "{:#}",
        entitlements.body
    );
    let sso_grant = entitlements.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entitlement| entitlement["feature"]["code"] == "sso")
        .unwrap_or_else(|| panic!("sso is granted: {:#}", entitlements.body));
    assert!(
        sso_grant["feature"]["id"]
            .as_str()
            .unwrap()
            .starts_with("feat_")
    );
    assert_eq!(sso_grant["type"], "boolean");
    assert_eq!(sso_grant["level"], "customer");
    assert!(sso_grant.get("resource").is_none(), "{sso_grant:#}");
    assert_eq!(sso_grant["value"], json!({"type": "on"}));
    assert_eq!(sso_grant["deciding_subscription"]["id"], id.as_str());
    assert_eq!(sso_grant["origin"]["owner"]["type"], "add_on");
    assert!(sso_grant.get("policy").is_none(), "{sso_grant:#}");
    assert!(sso_grant.get("own_grant").is_none(), "{sso_grant:#}");
    // One feature, as the list reads it (an entry per level); a feature it does not grant is not
    // found, and another tenant reads none of the subscription's.
    let one = f
        .get("/api/v2/subscriptions/crm-spotify-1/entitlements/sso")
        .await;
    assert_eq!(one.status, StatusCode::OK, "{:#}", one.body);
    assert_eq!(one.body["data"], json!([sso_grant]));
    let by_feature_id = f
        .get(&format!(
            "/api/v2/subscriptions/{id}/entitlements/{}",
            sso_grant["feature"]["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(by_feature_id.body, one.body);
    let ungranted = f
        .post(
            "/api/v2/features",
            json!({"code": "audit-log", "name": "Audit log", "type": {"type": "boolean"}}),
        )
        .await;
    assert_eq!(
        ungranted.status,
        StatusCode::CREATED,
        "{:#}",
        ungranted.body
    );
    for missing in ["audit-log", "no-such-feature"] {
        let answer = f
            .get(&format!(
                "/api/v2/subscriptions/crm-spotify-1/entitlements/{missing}"
            ))
            .await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{missing}: {:#}",
            answer.body
        );
        assert_eq!(answer.body["code"], "not_found");
    }
    let other_key = second_tenant_key(&f).await;
    let theirs = f
        .send(
            f.client
                .get(f.url(&format!("/api/v2/subscriptions/{id}/entitlements/sso")))
                .bearer_auth(&other_key),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{:#}", theirs.body);
    let by_id_entitlements = f
        .get(&format!("/api/v2/subscriptions/{id}/entitlements"))
        .await;
    assert_eq!(by_id_entitlements.body, entitlements.body);

    // The external id is the subscription's own.
    let again = f
        .post_once(
            "/api/v2/subscriptions",
            "create-1",
            json!({
                "customer": "ext:uber",
                "plan": "studio",
                "external_id": "crm-spotify-1",
                "quantities": {"users": "1"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{:#}", again.body);
    assert_eq!(again.body["code"], "code_taken");
    assert_eq!(again.body["errors"][0]["path"], "/external_id");

    // Never created without an Idempotency-Key: a retry must not create a second one.
    let unkeyed = f
        .post(
            "/api/v2/subscriptions",
            json!({"customer": "ext:uber", "plan": "studio", "quantities": {"users": "1"}}),
        )
        .await;
    assert_eq!(unkeyed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed.body["errors"][0]["path"], "Idempotency-Key");

    // Refused at their field: an option the plan has not, a quantity no seat charge counts.
    let unknown_option = f
        .post_once(
            "/api/v2/subscriptions",
            "create-2",
            json!({
                "customer": "ext:uber",
                "plan": "studio",
                "option": "weekly",
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(unknown_option.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown_option.body["code"], "validation_failed");
    assert_eq!(unknown_option.body["errors"][0]["path"], "/option");
    let not_counted = f
        .post_once(
            "/api/v2/subscriptions",
            "create-3",
            json!({
                "customer": "ext:uber",
                "plan": "studio",
                "quantities": {"licences": "3"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(not_counted.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        not_counted.body["errors"][0]["path"],
        "/quantities/licences"
    );
    let not_a_level = f
        .post_once(
            "/api/v2/subscriptions",
            "create-4",
            json!({
                "customer": "ext:uber",
                "plan": "studio",
                "quantities": {"users": "1"},
                "levels": {"api": "5000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(not_a_level.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A second subscription, its level the lowest when none is asked.
    let uber = f
        .post_once(
            "/api/v2/subscriptions",
            "create-5",
            json!({
                "customer": "ext:uber",
                "plan": "studio",
                "quantities": {"users": "1"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(uber.status, StatusCode::CREATED, "{:#}", uber.body);
    assert_eq!(
        charge(&uber.body, "api")["prices"][0]["model"]["levels"][0]["included"],
        1000
    );
    let uber_id = uber.body["id"].as_str().unwrap().to_owned();
    let without_add_on = f
        .get(&format!("/api/v2/subscriptions/{uber_id}/entitlements"))
        .await;
    assert_eq!(
        without_add_on.status,
        StatusCode::OK,
        "{:#}",
        without_add_on.body
    );
    assert!(
        without_add_on.body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entitlement| entitlement["feature"]["code"] != "sso"),
        "the add-on's grant is the subscription holding it's: {:#}",
        without_add_on.body
    );

    // Listed page by page, each once, without what a single one reads.
    let mut seen: Vec<String> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(cursor) => format!("/api/v2/subscriptions?limit=1&after={cursor}"),
            None => "/api/v2/subscriptions?limit=1".to_owned(),
        };
        let page = f.get(&path).await;
        assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
        let data = page.body["data"].as_array().unwrap();
        assert!(data.len() <= 1);
        for item in data {
            assert!(item.get("charges").is_none(), "a list item is the summary");
            assert!(item["tax_inclusive"].is_boolean());
            assert!(item["plan"]["code"].is_string());
            seen.push(item["id"].as_str().unwrap().to_owned());
        }
        if page.body["has_more"] == false {
            assert!(page.body["next_cursor"].is_null());
            break;
        }
        after = Some(page.body["next_cursor"].as_str().unwrap().to_owned());
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no subscription twice: {seen:?}");
    assert!(seen.contains(&id) && seen.contains(&uber_id), "{seen:?}");

    let spotify_only = f.get("/api/v2/subscriptions?customer=ext:spotify").await;
    assert_eq!(
        spotify_only.status,
        StatusCode::OK,
        "{:#}",
        spotify_only.body
    );
    let ids: Vec<&str> = spotify_only.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![id.as_str()]);
    let on_scale = f.get("/api/v2/subscriptions?plan=scale").await;
    assert_eq!(on_scale.body["data"].as_array().unwrap().len(), 0);
    let changed_later = f
        .get("/api/v2/subscriptions?updated_after=2999-01-01T00:00:00Z")
        .await;
    assert_eq!(changed_later.status, StatusCode::OK);
    assert_eq!(changed_later.body["data"].as_array().unwrap().len(), 0);

    // Several statuses, the parameter repeated; an unknown one refuses the list at `status`.
    let listed_ids = |answer: &Answer| -> Vec<String> {
        answer.body["data"]
            .as_array()
            .unwrap_or_else(|| panic!("not a page: {:#}", answer.body))
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let spotify_status = spotify_only.body["data"][0]["status"]
        .as_str()
        .unwrap()
        .to_owned();
    let others: Vec<&str> = ["active", "trial_active", "pending_activation", "completed"]
        .into_iter()
        .filter(|status| *status != spotify_status)
        .collect();
    let either = f
        .get(&format!(
            "/api/v2/subscriptions?customer=ext:spotify&status={}&status={spotify_status}",
            others[0]
        ))
        .await;
    assert_eq!(either.status, StatusCode::OK, "{:#}", either.body);
    assert_eq!(listed_ids(&either), vec![id.clone()]);
    let neither = f
        .get(&format!(
            "/api/v2/subscriptions?customer=ext:spotify&status={}&status={}",
            others[0], others[1]
        ))
        .await;
    assert_eq!(neither.status, StatusCode::OK, "{:#}", neither.body);
    assert!(listed_ids(&neither).is_empty());
    let unknown = f
        .get("/api/v2/subscriptions?status=active&status=paused")
        .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.body["errors"][0]["path"], "status");

    // Created after or before a moment, exclusive.
    let created_at = &spotify_only.body["data"][0]["created_at"];
    let (since, until) = (moment_from(created_at, -1), moment_from(created_at, 1));
    let created_since = f
        .get(&format!(
            "/api/v2/subscriptions?created_after={since}&limit=100"
        ))
        .await;
    assert_eq!(
        created_since.status,
        StatusCode::OK,
        "{:#}",
        created_since.body
    );
    assert!(listed_ids(&created_since).contains(&id));
    let created_earlier = f
        .get(&format!(
            "/api/v2/subscriptions?created_before={since}&limit=100"
        ))
        .await;
    assert_eq!(
        created_earlier.status,
        StatusCode::OK,
        "{:#}",
        created_earlier.body
    );
    assert!(!listed_ids(&created_earlier).contains(&id));
    let within = f
        .get(&format!(
            "/api/v2/subscriptions?customer=ext:spotify&created_after={since}&created_before={until}"
        ))
        .await;
    assert_eq!(listed_ids(&within), vec![id.clone()]);
    let not_a_moment = f.get("/api/v2/subscriptions?created_after=today").await;
    assert_eq!(not_a_moment.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(not_a_moment.body["errors"][0]["path"], "created_after");
}

/// Rules: ADR-0013 §10
#[tokio::test]
async fn test_rest_v2_subscription_changes() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-6",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "option": "monthly",
                "external_id": "comodo-main",
                "quantities": {"users": "5"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let base = "/api/v2/subscriptions/comodo-main";

    // A seat added once, whatever the retries; never without a key.
    let unkeyed = f
        .post(&format!("{base}/quantities/users"), json!({"delta": "+1"}))
        .await;
    assert_eq!(unkeyed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed.body["errors"][0]["path"], "Idempotency-Key");
    let preview = f
        .post(
            &format!("{base}/quantities/users/preview"),
            json!({"delta": "+1"}),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{:#}", preview.body);
    assert_eq!(preview.body["direction"], "upgrade");
    assert_eq!(
        preview.body["subscription_after"]["quantities"]["users"]["held"],
        "6"
    );
    let added = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-plus-one",
            json!({"delta": "+1"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{:#}", added.body);
    assert_eq!(added.body["status"], "applied");
    assert_eq!(
        added.body["subscription"]["quantities"]["users"]["held"],
        "6"
    );
    let replayed = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-plus-one",
            json!({"delta": "+1"}),
        )
        .await;
    assert_eq!(replayed.status, StatusCode::OK);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replayed.body, added.body);
    let read = f.get(base).await;
    assert_eq!(
        read.body["quantities"]["users"]["held"], "6",
        "the replay added nothing"
    );

    // A higher level applies at once; it is never held to the period's end.
    let held_back = f
        .post(
            &format!("{base}/levels/api/preview"),
            json!({"included": "10000", "when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(held_back.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(held_back.body["code"], "when_refused");
    let level_preview = f
        .post(
            &format!("{base}/levels/api/preview"),
            json!({"included": "10000"}),
        )
        .await;
    assert_eq!(
        level_preview.status,
        StatusCode::OK,
        "{:#}",
        level_preview.body
    );
    assert_eq!(level_preview.body["direction"], "upgrade");
    assert_eq!(level_preview.body["when"], "immediate");
    assert_eq!(
        level_preview.body["subscription_after"]["levels"]["api"],
        "10000"
    );
    let leveled = f
        .post_once(
            &format!("{base}/levels/api"),
            "level-up",
            json!({"included": "10000"}),
        )
        .await;
    assert_eq!(leveled.status, StatusCode::OK, "{:#}", leveled.body);
    assert_eq!(leveled.body["status"], "applied");
    assert_eq!(
        charge(&leveled.body["subscription"], "api")["prices"][0]["model"]["levels"][0]["included"],
        10000
    );

    // An upgrade, previewed then applied at once.
    let upgrade = json!({"plan": "scale", "when": {"type": "immediate"}});
    let previewed = f
        .post(&format!("{base}/plan-change/preview"), upgrade.clone())
        .await;
    assert_eq!(previewed.status, StatusCode::OK, "{:#}", previewed.body);
    assert_eq!(previewed.body["direction"], "upgrade");
    assert_eq!(previewed.body["when"], "immediate");
    assert_eq!(
        previewed.body["subscription_after"]["plan"]["code"],
        "scale"
    );
    assert_eq!(
        previewed.body["subscription_after"]["plan"]["option"],
        "monthly"
    );
    let platform_after = charge(&previewed.body["subscription_after"], "platform");
    assert_eq!(platform_after["prices"][0]["model"]["amount"], "199");
    let still = f.get(base).await;
    assert_eq!(
        still.body["plan"]["code"], "studio",
        "a preview writes nothing"
    );
    let unkeyed_upgrade = f
        .post(&format!("{base}/plan-change"), upgrade.clone())
        .await;
    assert_eq!(unkeyed_upgrade.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed_upgrade.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed_upgrade.body["errors"][0]["path"], "Idempotency-Key");
    let upgraded = f
        .post_once(&format!("{base}/plan-change"), "upgrade", upgrade)
        .await;
    assert_eq!(upgraded.status, StatusCode::OK, "{:#}", upgraded.body);
    assert_eq!(upgraded.body["status"], "applied");
    assert_eq!(upgraded.body["subscription"]["plan"]["code"], "scale");
    assert_eq!(
        upgraded.body["subscription"]["quantities"]["users"]["held"], "6",
        "the seats held move with it"
    );

    // A downgrade is refused at once, and scheduled for the period's end.
    let at_once = f
        .post(
            &format!("{base}/plan-change/preview"),
            json!({"plan": "studio", "when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(at_once.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(at_once.body["code"], "when_refused");
    assert_eq!(at_once.body["errors"][0]["path"], "/when/type");
    let on_a_date = f
        .post_once(
            &format!("{base}/plan-change"),
            "downgrade-on-a-date",
            json!({"plan": "studio", "when": {"type": "date", "date": "2030-01-01"}}),
        )
        .await;
    assert_eq!(on_a_date.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(on_a_date.body["code"], "when_refused");
    let downgrade = json!({"plan": "studio", "when": {"type": "period_end"}});
    let downgrade_preview = f
        .post(&format!("{base}/plan-change/preview"), downgrade.clone())
        .await;
    assert_eq!(
        downgrade_preview.status,
        StatusCode::OK,
        "{:#}",
        downgrade_preview.body
    );
    assert_eq!(downgrade_preview.body["direction"], "downgrade");
    assert_eq!(downgrade_preview.body["when"], "period_end");
    let scheduled = f
        .post_once(&format!("{base}/plan-change"), "downgrade", downgrade)
        .await;
    assert_eq!(scheduled.status, StatusCode::OK, "{:#}", scheduled.body);
    assert_eq!(scheduled.body["status"], "scheduled");
    assert!(scheduled.body["scheduled_change"].is_string());
    assert_eq!(scheduled.body["subscription"]["plan"]["code"], "scale");
    let schedule = f.get(&format!("{base}/schedule")).await;
    assert_eq!(schedule.status, StatusCode::OK, "{:#}", schedule.body);
    let change = &schedule.body["changes"][0];
    assert_eq!(change["type"], "plan_change");
    assert_eq!(change["plan"]["code"], "studio");
    assert_eq!(change["effective_date"], scheduled.body["effective_date"]);

    // A cancellation at the period's end, revoked, then made at once.
    let cancel_preview = f
        .post(
            &format!("{base}/cancel/preview"),
            json!({"when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(
        cancel_preview.status,
        StatusCode::OK,
        "{:#}",
        cancel_preview.body
    );
    assert_eq!(
        cancel_preview.body["subscription_after"]["status"],
        "cancelled"
    );
    let unsaid = f
        .post_once(&format!("{base}/cancel"), "cancel-unsaid", json!({}))
        .await;
    assert_eq!(unsaid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unsaid.body["errors"][0]["path"], "/when");
    let cancelled = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-at-period-end",
            json!({"when": {"type": "period_end"}, "reason": "budget"}),
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    assert_eq!(cancelled.body["status"], "scheduled");
    assert_eq!(
        cancelled.body["subscription"]["cancellation"]["reason"],
        "budget"
    );
    let revoked = f.post(&format!("{base}/cancel/revoke"), json!({})).await;
    assert_eq!(revoked.status, StatusCode::OK, "{:#}", revoked.body);
    assert!(revoked.body.get("cancellation").is_none());
    let nothing_to_revoke = f.post(&format!("{base}/cancel/revoke"), json!({})).await;
    assert_eq!(nothing_to_revoke.status, StatusCode::CONFLICT);
    let ended = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-now",
            json!({"when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(ended.status, StatusCode::OK, "{:#}", ended.body);
    assert_eq!(ended.body["status"], "applied");
    assert_eq!(ended.body["subscription"]["status"], "cancelled");
}

fn amount(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("not an amount: {value:#}"))
}

/// The documents a preview lists, each with its lines and its total in minor units; an invoice
/// bills a line at least.
fn documents_with_lines(preview: &Value) -> &Vec<Value> {
    let documents = preview["documents"].as_array().unwrap();
    for document in documents {
        let lines = document["lines"].as_array().unwrap();
        if document["type"] != "credit_note" {
            assert!(
                !lines.is_empty(),
                "an invoice without its lines: {document:#}"
            );
        }
        assert!(document["total"].is_i64(), "{document:#}");
    }
    documents
}

/// Rules: ADR-0013 §10 (previews read whole: documents with lines, the subscription after the
/// change; seats on payment; a cancellation's date)
#[tokio::test]
async fn test_rest_v2_subscription_previews_in_full() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-full",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "option": "monthly",
                "external_id": "comodo-full",
                "quantities": {"users": "5"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    // The external id is written with the subscription, in its create.
    assert_eq!(created.body["external_id"], "comodo-full");
    assert_eq!(created.body["levels"]["api"], "1000");
    let mrr = amount(&created.body["mrr"]["amount"]);
    let charges = created.body["charges"].as_array().unwrap().len();
    let base = "/api/v2/subscriptions/comodo-full";

    // Two seats more: the subscription after holds them and bills them in its MRR.
    let seats = f
        .post(
            &format!("{base}/quantities/users/preview"),
            json!({"delta": "+2"}),
        )
        .await;
    assert_eq!(seats.status, StatusCode::OK, "{:#}", seats.body);
    let after = &seats.body["subscription_after"];
    assert_eq!(after["id"], created.body["id"]);
    assert_eq!(after["quantities"]["users"]["held"], "7");
    assert_eq!(after["plan"]["code"], "studio");
    assert_eq!(after["charges"].as_array().unwrap().len(), charges);
    assert_eq!(
        amount(&after["mrr"]["amount"]),
        mrr + 2 * 1000,
        "two seats at 10 a month"
    );
    let seat_documents = documents_with_lines(&seats.body);
    assert_eq!(seat_documents[0]["type"], "invoice");

    // A level up: the level and its price read in the subscription after.
    let level = f
        .post(
            &format!("{base}/levels/api/preview"),
            json!({"included": "10000"}),
        )
        .await;
    assert_eq!(level.status, StatusCode::OK, "{:#}", level.body);
    let after = &level.body["subscription_after"];
    assert_eq!(after["levels"]["api"], "10000");
    assert_eq!(
        charge(after, "api")["prices"][0]["model"]["levels"][0]["included"],
        10000
    );
    assert!(amount(&after["mrr"]["amount"]) > mrr, "{after:#}");
    documents_with_lines(&level.body);

    // An add-on amended in: its charge and the add-on read in the subscription after.
    let amended = f
        .post(
            &format!("{base}/amendments/preview"),
            json!({"add_ons": {"add": [{"add_on": "support"}]}, "when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(amended.status, StatusCode::OK, "{:#}", amended.body);
    let after = &amended.body["subscription_after"];
    assert_eq!(after["add_ons"][0]["add_on"]["code"], "support");
    assert_eq!(charge(after, "support")["origin"], "add_on");
    assert_eq!(after["charges"].as_array().unwrap().len(), charges + 1);
    documents_with_lines(&amended.body);

    // An upgrade with more seats: what it invoices, line by line, and the subscription on the
    // new plan, its seats, its level kept and its MRR.
    let upgrade = f
        .post(
            &format!("{base}/plan-change/preview"),
            json!({"plan": "scale", "quantities": {"users": "8"}, "when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(upgrade.status, StatusCode::OK, "{:#}", upgrade.body);
    let documents = documents_with_lines(&upgrade.body);
    let invoice = documents
        .iter()
        .find(|document| document["type"] == "invoice")
        .unwrap_or_else(|| panic!("the upgrade invoices: {:#}", upgrade.body));
    assert!(amount(&invoice["total"]) > 0, "{invoice:#}");
    for note in documents.iter().filter(|d| d["type"] == "credit_note") {
        assert!(amount(&note["total"]) <= 0, "{note:#}");
    }
    let after = &upgrade.body["subscription_after"];
    assert_eq!(
        plan_named(&after["plan"]),
        json!({"code": "scale", "version": 1, "option": "monthly"})
    );
    assert_eq!(after["quantities"]["users"]["held"], "8");
    assert_eq!(after["levels"]["api"], "1000");
    assert_eq!(
        charge(after, "platform")["prices"][0]["model"]["amount"],
        "199"
    );
    assert_eq!(
        charge(after, "seat")["prices"][0]["model"]["unit_price"],
        "12"
    );
    assert!(amount(&after["mrr"]["amount"]) > mrr, "{after:#}");
    // At the period's end, the renewal under the new plan.
    let upgrade_later = f
        .post(
            &format!("{base}/plan-change/preview"),
            json!({"plan": "scale", "when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(
        upgrade_later.status,
        StatusCode::OK,
        "{:#}",
        upgrade_later.body
    );
    assert_eq!(
        upgrade_later.body["subscription_after"]["plan"]["code"],
        "scale"
    );
    for document in documents_with_lines(&upgrade_later.body) {
        assert_eq!(document["type"], "next_invoice", "{document:#}");
    }

    // A cancellation now: the documents it issues, and the subscription cancelled.
    let cancel_now = f
        .post(
            &format!("{base}/cancel/preview"),
            json!({"when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(cancel_now.status, StatusCode::OK, "{:#}", cancel_now.body);
    let documents = documents_with_lines(&cancel_now.body);
    assert!(
        !documents.is_empty(),
        "an immediate cancellation settles its period: {:#}",
        cancel_now.body
    );
    for note in documents.iter().filter(|d| d["type"] == "credit_note") {
        assert!(amount(&note["total"]) < 0, "{note:#}");
    }
    assert_eq!(cancel_now.body["subscription_after"]["status"], "cancelled");
    assert_eq!(cancel_now.body["subscription_after"]["mrr"]["amount"], 0);
    let at_period_end = f
        .post(
            &format!("{base}/cancel/preview"),
            json!({"when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(
        at_period_end.status,
        StatusCode::OK,
        "{:#}",
        at_period_end.body
    );
    assert_eq!(
        at_period_end.body["subscription_after"]["cancellation"]["effective_date"],
        at_period_end.body["effective_date"]
    );
    documents_with_lines(&at_period_end.body);

    // A cancellation is never dated in the past.
    let past = json!({"when": {"type": "date", "date": "2020-01-01"}});
    let past_preview = f
        .post(&format!("{base}/cancel/preview"), past.clone())
        .await;
    assert_eq!(past_preview.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(past_preview.body["code"], "when_refused");
    assert_eq!(past_preview.body["errors"][0]["path"], "/when/date");
    let past_cancel = f
        .post_once(&format!("{base}/cancel"), "cancel-in-the-past", past)
        .await;
    assert_eq!(past_cancel.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(past_cancel.body["errors"][0]["path"], "/when/date");
    let read = f.get(base).await;
    assert!(read.body.get("cancellation").is_none(), "{:#}", read.body);

    // A seat bought on payment: pending until its invoice is paid.
    let on_payment = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-on-payment",
            json!({"delta": "+1", "activation": "on_payment"}),
        )
        .await;
    assert_eq!(on_payment.status, StatusCode::OK, "{:#}", on_payment.body);
    assert_eq!(on_payment.body["status"], "pending_payment");
    let awaiting = &on_payment.body["awaiting_payment"];
    assert!(
        awaiting["id"].as_str().unwrap().starts_with("inv_"),
        "{awaiting}"
    );
    assert!(awaiting["number"].is_string(), "{awaiting}");
    assert_eq!(&on_payment.body["invoices"][0], awaiting);
    let users = &on_payment.body["subscription"]["quantities"]["users"];
    assert_eq!(users["held"], "5", "not held before its payment");
    assert_eq!(users["pending"][0]["value"], "6");
}

/// Rules: ADR-0013 §3 (a write committed keeps its answer for its Idempotency-Key)
#[tokio::test]
async fn test_rest_v2_subscription_write_kept_when_not_read_back() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-legacy",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "external_id": "comodo-legacy",
                "quantities": {"users": "2"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    // A charge without its product, as data older than products reads: v2 cannot read it back.
    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(
            "UPDATE subscription_component SET product_id = NULL \
             WHERE subscription_id = (SELECT id FROM subscription WHERE external_id = $1) \
             AND product_id IN (SELECT id FROM product WHERE code = 'platform')",
        )
        .bind::<diesel::sql_types::Text, _>("comodo-legacy")
        .execute(&mut f.setup.store.get_conn().await.unwrap())
        .await
        .unwrap();
    }
    let base = "/api/v2/subscriptions/comodo-legacy";
    let unreadable = f.get(base).await;
    assert_eq!(unreadable.status, StatusCode::UNPROCESSABLE_ENTITY);

    // The cancellation is made: it answers what it did, without the subscription, and its
    // retry replays that answer rather than cancelling again.
    let body = json!({"when": {"type": "period_end"}});
    let cancelled = f
        .post_once(&format!("{base}/cancel"), "cancel-legacy", body.clone())
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    assert_eq!(cancelled.body["status"], "scheduled");
    assert!(cancelled.body.get("subscription").is_none());
    assert!(cancelled.body["scheduled_change"].is_string());
    let replayed = f
        .post_once(&format!("{base}/cancel"), "cancel-legacy", body)
        .await;
    assert_eq!(replayed.status, StatusCode::OK);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replayed.body, cancelled.body);
    let schedule = f.get(&format!("{base}/schedule")).await;
    assert_eq!(schedule.status, StatusCode::OK, "{:#}", schedule.body);
    assert_eq!(schedule.body["changes"].as_array().unwrap().len(), 1);
}

/// A millisecond after `at`: times are read at the millisecond, stored at the microsecond.
fn just_after(at: &str) -> String {
    (chrono::DateTime::parse_from_rfc3339(at).unwrap() + chrono::Duration::milliseconds(1))
        .to_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Rules: ADR-0013 §10 (`updated_after`: what invoicing writes for itself beside a subscription
/// moves nothing)
#[tokio::test]
async fn test_rest_v2_subscription_updated_at_skips_bookkeeping() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-quiet",
            json!({
                "customer": "ext:spotify",
                "plan": "studio",
                "option": "monthly",
                "external_id": "quiet",
                "quantities": {"users": "2"},
                "coupons": ["LAUNCH10"],
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let id = created.body["id"].as_str().unwrap().to_owned();
    let fixture = &f;
    let read = || async move {
        let subscription = fixture.get("/api/v2/subscriptions/quiet").await;
        assert_eq!(
            subscription.status,
            StatusCode::OK,
            "{:#}",
            subscription.body
        );
        subscription.body["updated_at"].as_str().unwrap().to_owned()
    };
    let changed_after = |at: String| async move {
        let page = fixture
            .get(&format!("/api/v2/subscriptions?updated_after={at}"))
            .await;
        assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
        page.body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // Each committed on its own, its triggers' writes with it.
    let write = |sql: &'static str| async move {
        use diesel_async::RunQueryDsl;
        diesel::sql_query(sql)
            .bind::<diesel::sql_types::Text, _>("quiet")
            .execute(&mut fixture.setup.store.get_conn().await.unwrap())
            .await
            .unwrap()
    };
    let before = read().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // What an invoice run writes beside it for itself: the coupon's use counted, the seats'
    // transactions pointed at another invoice; and a charge written back as it was.
    let counted = write(
        "UPDATE applied_coupon SET applied_count = COALESCE(applied_count, 0) + 1, \
         applied_amount = COALESCE(applied_amount, 0) + 1, last_applied_at = now() \
         WHERE subscription_id = (SELECT id FROM subscription WHERE external_id = $1)",
    )
    .await;
    assert_eq!(counted, 1);
    let repointed = write(
        "UPDATE slot_transaction t SET invoice_id = CASE WHEN t.invoice_id IS NULL THEN \
         (SELECT i.id FROM invoice i WHERE i.subscription_id = t.subscription_id \
          ORDER BY i.id LIMIT 1) END \
         WHERE t.subscription_id = (SELECT id FROM subscription WHERE external_id = $1) \
         AND EXISTS (SELECT 1 FROM invoice i WHERE i.subscription_id = t.subscription_id)",
    )
    .await;
    assert!(
        repointed > 0,
        "the seats' transactions of an invoiced subscription"
    );
    write(
        "UPDATE subscription_component SET name = name \
         WHERE subscription_id = (SELECT id FROM subscription WHERE external_id = $1)",
    )
    .await;
    assert_eq!(read().await, before);
    assert!(!changed_after(just_after(&before)).await.contains(&id));

    // The coupon running out is a change of what a reader sees.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    write(
        "UPDATE applied_coupon SET is_active = false \
         WHERE subscription_id = (SELECT id FROM subscription WHERE external_id = $1)",
    )
    .await;
    let changed = read().await;
    assert!(changed > before, "{changed} after {before}");
    assert!(changed_after(just_after(&before)).await.contains(&id));
}

async fn patch(f: &Fixture, path: &str, body: Value) -> crate::test_rest_v2::Answer {
    f.send(
        f.client
            .patch(f.url(path))
            .bearer_auth(&f.api_key)
            .header("Content-Type", "application/merge-patch+json")
            .body(body.to_string()),
    )
    .await
}

/// Rules: ADR-0013 §10 (PATCH)
#[tokio::test]
async fn test_rest_v2_subscription_patched() {
    let f = fixture().await;
    seed(&f).await;
    let mut ids = Vec::new();
    for (customer, key) in [
        ("ext:spotify", "patch-create-1"),
        ("ext:uber", "patch-create-2"),
    ] {
        let created = f
            .post_once(
                "/api/v2/subscriptions",
                key,
                json!({
                    "customer": customer,
                    "plan": "studio",
                    "quantities": {"users": "1"},
                    "invoicing": {"charge_automatically": false, "purchase_order": "PO-1"}
                }),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
        ids.push(created.body["id"].as_str().unwrap().to_owned());
    }
    let (id, other) = (&ids[0], &ids[1]);

    // Set: an external id, a memo, net terms; what is left out stays (the purchase order).
    let patched = patch(
        &f,
        &format!("/api/v2/subscriptions/{id}"),
        json!({"external_id": "crm-patch-1", "invoicing": {"memo": "Thanks", "net_terms": 30}}),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["external_id"], "crm-patch-1");
    assert_eq!(patched.body["invoicing"]["memo"], "Thanks");
    assert_eq!(patched.body["invoicing"]["net_terms"], 30);
    assert_eq!(patched.body["invoicing"]["purchase_order"], "PO-1");
    let read = f.get("/api/v2/subscriptions/crm-patch-1").await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["id"], id.as_str());

    // The same patch again: the id is its own.
    let again = patch(
        &f,
        &format!("/api/v2/subscriptions/{id}"),
        json!({"external_id": "crm-patch-1"}),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "{:#}", again.body);

    // Another subscription's external id is refused at its field; one reading as an id too.
    let taken = patch(
        &f,
        &format!("/api/v2/subscriptions/{other}"),
        json!({"external_id": "crm-patch-1"}),
    )
    .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{:#}", taken.body);
    assert_eq!(taken.body["code"], "code_taken");
    assert_eq!(taken.body["errors"][0]["path"], "/external_id");
    let like_an_id = patch(
        &f,
        &format!("/api/v2/subscriptions/{other}"),
        json!({"external_id": other}),
    )
    .await;
    assert_eq!(like_an_id.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(like_an_id.body["errors"][0]["path"], "/external_id");

    // `null` clears the external id, the memo and the purchase order; net terms stay.
    let cleared = patch(
        &f,
        "/api/v2/subscriptions/crm-patch-1",
        json!({"external_id": null, "invoicing": {"memo": null, "purchase_order": null}}),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{:#}", cleared.body);
    assert!(
        cleared.body.get("external_id").is_none(),
        "{:#}",
        cleared.body
    );
    assert!(cleared.body["invoicing"].get("memo").is_none());
    assert!(cleared.body["invoicing"].get("purchase_order").is_none());
    assert_eq!(cleared.body["invoicing"]["net_terms"], 30);
    let gone = f.get("/api/v2/subscriptions/crm-patch-1").await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    // Freed, the other subscription takes it.
    let moved = patch(
        &f,
        &format!("/api/v2/subscriptions/{other}"),
        json!({"external_id": "crm-patch-1"}),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{:#}", moved.body);
    assert_eq!(moved.body["external_id"], "crm-patch-1");
}

fn date_of(value: &Value) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(
        value
            .as_str()
            .unwrap_or_else(|| panic!("not a date: {value:#}")),
        "%Y-%m-%d",
    )
    .unwrap()
}

/// Rules: R-140, ADR-0013 §10 (PATCH `end_date`: an end date moves over what is not billed yet)
#[tokio::test]
async fn test_rest_v2_subscription_end_date_moved() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-end-date",
            json!({
                "customer": "ext:comodo",
                "plan": "scale",
                "option": "monthly",
                "external_id": "comodo-term",
                "quantities": {"users": "2"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    assert!(
        created.body["dates"].get("end").is_none(),
        "{:#}",
        created.body
    );
    let base = "/api/v2/subscriptions/comodo-term";
    let today = chrono::Utc::now().date_naive();
    let period_end = date_of(&created.body["current_period"]["end"]);
    assert!(period_end > today.succ_opt().unwrap(), "{period_end}");

    // Today, or a day inside the period billed in advance (19900 + 2 × 1200 for the month): both
    // refused at the field, nothing written. A day inside it is a cancellation on that date.
    for refused_on in [today, period_end.pred_opt().unwrap()] {
        let refused = patch(&f, base, json!({"end_date": refused_on.to_string()})).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{refused_on}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], "/end_date");
    }
    let unchanged = f.get(base).await;
    assert!(unchanged.body["dates"].get("end").is_none());

    // The period's end, a renewal: it stops renewing there.
    let at_renewal = patch(&f, base, json!({"end_date": period_end.to_string()})).await;
    assert_eq!(at_renewal.status, StatusCode::OK, "{:#}", at_renewal.body);
    assert_eq!(date_of(&at_renewal.body["dates"]["end"]), period_end);

    // Further on, inside a period not billed yet: taken (that period will bill up to it).
    let later = period_end + chrono::Duration::days(45);
    let moved = patch(&f, base, json!({"end_date": later.to_string()})).await;
    assert_eq!(moved.status, StatusCode::OK, "{:#}", moved.body);
    assert_eq!(date_of(&moved.body["dates"]["end"]), later);
    // The same again changes nothing.
    let again = patch(&f, base, json!({"end_date": later.to_string()})).await;
    assert_eq!(again.status, StatusCode::OK, "{:#}", again.body);
    assert_eq!(again.body["dates"], moved.body["dates"]);

    // Cleared: it renews until cancelled.
    let cleared = patch(&f, base, json!({"end_date": null})).await;
    assert_eq!(cleared.status, StatusCode::OK, "{:#}", cleared.body);
    assert!(
        cleared.body["dates"].get("end").is_none(),
        "{:#}",
        cleared.body
    );

    // A downgrade scheduled for the period's end: an end date on it would leave the change no
    // day to apply to (R-092), refused; one after it is taken.
    let downgrade = f
        .post_once(
            &format!("{base}/plan-change"),
            "end-date-downgrade",
            json!({"plan": "studio", "when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(downgrade.status, StatusCode::OK, "{:#}", downgrade.body);
    assert_eq!(downgrade.body["status"], "scheduled");
    let on_the_change = patch(&f, base, json!({"end_date": period_end.to_string()})).await;
    assert_eq!(
        on_the_change.status,
        StatusCode::CONFLICT,
        "{:#}",
        on_the_change.body
    );
    assert_eq!(on_the_change.body["code"], "conflict");
    let after_the_change = patch(&f, base, json!({"end_date": later.to_string()})).await;
    assert_eq!(
        after_the_change.status,
        StatusCode::OK,
        "{:#}",
        after_the_change.body
    );
    assert_eq!(date_of(&after_the_change.body["dates"]["end"]), later);

    // A cancellation scheduled ends it first: the end date waits for its revocation.
    let cancelled = f
        .post_once(
            &format!("{base}/cancel"),
            "end-date-cancel",
            json!({"when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    assert_eq!(cancelled.body["status"], "scheduled");
    let while_cancelled = patch(
        &f,
        base,
        json!({"end_date": (later + chrono::Duration::days(30)).to_string()}),
    )
    .await;
    assert_eq!(
        while_cancelled.status,
        StatusCode::CONFLICT,
        "{:#}",
        while_cancelled.body
    );
    let revoked = f.post(&format!("{base}/cancel/revoke"), json!({})).await;
    assert_eq!(revoked.status, StatusCode::OK, "{:#}", revoked.body);
    let after_revoke = patch(&f, base, json!({"end_date": null})).await;
    assert_eq!(
        after_revoke.status,
        StatusCode::OK,
        "{:#}",
        after_revoke.body
    );
    assert!(after_revoke.body["dates"].get("end").is_none());
}

/// Rules: R-137, R-138 (a seat add-on bills the count its quantity holds), ADR-0013 §10 (the
/// subscription after a seat change, its MRR)
#[tokio::test]
async fn test_rest_v2_seat_preview_mrr_counts_seat_add_ons() {
    let f = fixture().await;
    seed(&f).await;
    let product = f
        .post(
            "/api/v2/products",
            json!({"code": "seat-support", "name": "Seat support"}),
        )
        .await;
    assert_eq!(product.status, StatusCode::CREATED, "{:#}", product.body);
    let add_on = f
        .post(
            "/api/v2/add-ons",
            json!({
                "code": "seat-support",
                "name": "Seat support",
                "charge": {
                    "product": "seat-support",
                    "preset": "seats",
                    "quantity": {"type": "managed", "quantity": "users"},
                    "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "3"}}]
                }
            }),
        )
        .await;
    assert_eq!(add_on.status, StatusCode::CREATED, "{:#}", add_on.body);
    let mut team = version("49", "10");
    team["add_ons"] = json!([{"add_on": "seat-support"}]);
    publish(&f, "team", "Team", team).await;

    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-seat-add-on",
            json!({
                "customer": "ext:comodo",
                "plan": "team",
                "option": "monthly",
                "external_id": "comodo-seat-add-on",
                "quantities": {"users": "5"},
                "levels": {"api": "1000"},
                "add_ons": [{"add_on": "seat-support"}],
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    // 4900 + 5 × 1000 seats + 1900 for the level + 5 × 300 of the add-on = 13300.
    let mrr = amount(&created.body["mrr"]["amount"]);
    assert_eq!(mrr, 13_300, "{:#}", created.body);
    let base = "/api/v2/subscriptions/comodo-seat-add-on";

    // Two seats more: the plan's seats and the add-on's both count 7, 2 × (1000 + 300) = 2600
    // more a month, where the add-on read at its own 5 added 2000.
    let preview = f
        .post(
            &format!("{base}/quantities/users/preview"),
            json!({"delta": "+2"}),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{:#}", preview.body);
    let after = &preview.body["subscription_after"];
    assert_eq!(after["quantities"]["users"]["held"], "7");
    let previewed = amount(&after["mrr"]["amount"]);
    assert_eq!(previewed, mrr + 2_600, "{after:#}");

    // Applied, the subscription states the MRR its preview did.
    let added = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-add-on-plus-two",
            json!({"delta": "+2"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{:#}", added.body);
    assert_eq!(added.body["status"], "applied");
    assert_eq!(
        amount(&added.body["subscription"]["mrr"]["amount"]),
        previewed,
        "{:#}",
        added.body
    );
}

/// The ids of a list's items.
async fn listed_ids(f: &Fixture, path: &str) -> std::collections::BTreeSet<String> {
    let page = f.get(path).await;
    assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
    page.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

fn ids_of(value: &Value) -> std::collections::BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|document| document["id"].as_str().unwrap().to_owned())
        .collect()
}

/// A plan reference without its id: what a test names it by.
fn plan_named(plan: &Value) -> Value {
    let mut plan = plan.clone();
    plan.as_object_mut().map(|plan| plan.remove("id"));
    plan
}

/// Rules: R-141, ADR-0013 §10 (a cancellation replaces the one still to come; applied now, it
/// answers the documents it issued; an ended subscription is not cancelled again)
#[tokio::test]
async fn test_rest_v2_cancellation_replaced_then_refused() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-cancelled",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "option": "monthly",
                "external_id": "comodo-cancelled",
                "quantities": {"users": "2"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let base = "/api/v2/subscriptions/comodo-cancelled";
    let tomorrow = chrono::Utc::now().date_naive().succ_opt().unwrap();

    // At the period's end, then moved to tomorrow: one cancellation to come, at its new date.
    let later = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-later",
            json!({"when": {"type": "period_end"}}),
        )
        .await;
    assert_eq!(later.status, StatusCode::OK, "{:#}", later.body);
    assert_eq!(later.body["status"], "scheduled");
    let sooner = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-sooner",
            json!({"when": {"type": "date", "date": tomorrow.to_string()}}),
        )
        .await;
    assert_eq!(sooner.status, StatusCode::OK, "{:#}", sooner.body);
    assert_eq!(sooner.body["status"], "scheduled");
    assert_eq!(date_of(&sooner.body["effective_date"]), tomorrow);
    let schedule = f.get(&format!("{base}/schedule")).await;
    let changes = schedule.body["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{:#}", schedule.body);
    assert_eq!(date_of(&changes[0]["effective_date"]), tomorrow);

    // Now: it answers what its ending issued, the documents the subscription gained.
    let invoices = "/api/v2/invoices?subscription=comodo-cancelled";
    let notes = "/api/v2/credit-notes?subscription=comodo-cancelled";
    let (invoices_before, notes_before) =
        (listed_ids(&f, invoices).await, listed_ids(&f, notes).await);
    let now = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-now",
            json!({"when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(now.status, StatusCode::OK, "{:#}", now.body);
    assert_eq!(now.body["status"], "applied");
    assert_eq!(now.body["subscription"]["status"], "cancelled");
    let issued: std::collections::BTreeSet<String> = listed_ids(&f, invoices)
        .await
        .difference(&invoices_before)
        .cloned()
        .collect();
    let credited: std::collections::BTreeSet<String> = listed_ids(&f, notes)
        .await
        .difference(&notes_before)
        .cloned()
        .collect();
    assert_eq!(ids_of(&now.body["invoices"]), issued, "{:#}", now.body);
    assert_eq!(
        ids_of(&now.body["credit_notes"]),
        credited,
        "{:#}",
        now.body
    );

    // Ended: neither cancelled again nor previewed, nothing written.
    let again = f
        .post_once(
            &format!("{base}/cancel"),
            "cancel-again",
            json!({"when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{:#}", again.body);
    assert_eq!(again.body["code"], "conflict");
    let previewed = f
        .post(
            &format!("{base}/cancel/preview"),
            json!({"when": {"type": "immediate"}}),
        )
        .await;
    assert_eq!(
        previewed.status,
        StatusCode::CONFLICT,
        "{:#}",
        previewed.body
    );
    assert_eq!(
        listed_ids(&f, invoices).await.len(),
        invoices_before.len() + issued.len()
    );
}

/// 10 seats held and 5 bought on payment, still paying: `set: 20` buys 5, in its preview as in
/// its apply. The subscription started today, so the 5 bill the whole month: 5 × 10.00 = 5000.
/// Rules: ADR-0013 §10 (a count counted as the apply counts it)
#[tokio::test]
async fn test_rest_v2_seat_count_counts_the_seats_still_paying() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-seat-count",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "option": "monthly",
                "external_id": "comodo-seat-count",
                "quantities": {"users": "10"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let base = "/api/v2/subscriptions/comodo-seat-count";
    let paying = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-count-on-payment",
            json!({"delta": "+5", "activation": "on_payment"}),
        )
        .await;
    assert_eq!(paying.status, StatusCode::OK, "{:#}", paying.body);
    assert_eq!(paying.body["status"], "pending_payment");
    assert_eq!(
        paying.body["subscription"]["quantities"]["users"]["held"],
        "10"
    );

    let preview = f
        .post(
            &format!("{base}/quantities/users/preview"),
            json!({"set": "20"}),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{:#}", preview.body);
    assert_eq!(preview.body["direction"], "upgrade");
    let documents = documents_with_lines(&preview.body);
    let invoice = documents
        .iter()
        .find(|document| document["type"] == "invoice")
        .unwrap_or_else(|| panic!("the seats bought invoice: {:#}", preview.body));
    assert_eq!(amount(&invoice["subtotal"]), 5_000, "{invoice:#}");
    assert_eq!(
        preview.body["subscription_after"]["quantities"]["users"]["held"],
        "15"
    );

    let set = f
        .post_once(
            &format!("{base}/quantities/users"),
            "seat-count-set-20",
            json!({"set": "20"}),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{:#}", set.body);
    assert_eq!(set.body["status"], "applied");
    assert_eq!(
        set.body["subscription"]["quantities"]["users"]["held"],
        "15"
    );
    let invoice_id = set.body["invoices"][0]["id"].as_str().unwrap();
    let billed = f.get(&format!("/api/v2/invoices/{invoice_id}")).await;
    assert_eq!(billed.status, StatusCode::OK, "{:#}", billed.body);
    assert_eq!(
        amount(&billed.body["subtotal"]),
        5_000,
        "what the preview said: {:#}",
        billed.body
    );
}

/// A plan change naming no option moves to the option the store picks, its preview and its apply
/// alike: on "studio" annual, "pro-cadence" has no `annual` alias, so the option billing the annual
/// cadence (`yearly`), not the first sold (`monthly`).
/// Rules: ADR-0013 §10 (a plan change without `option`)
#[tokio::test]
async fn test_rest_v2_plan_change_keeps_the_cadence_of_its_option() {
    let f = fixture().await;
    seed(&f).await;
    let mut pro = version("99", "12");
    pro["options"] = json!([
        {"alias": "monthly", "name": "Monthly"},
        {"alias": "yearly", "name": "Yearly"}
    ]);
    pro["charges"][0]["prices"][1]["option"] = json!("yearly");
    pro["charges"][0]["prices"][1]["model"]["amount"] = json!("2990");
    publish(&f, "pro-cadence", "Pro cadence", pro).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-annual",
            json!({
                "customer": "ext:comodo",
                "plan": "studio",
                "option": "annual",
                "external_id": "comodo-annual",
                "quantities": {"users": "2"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let base = "/api/v2/subscriptions/comodo-annual";

    let change = json!({"plan": "pro-cadence", "when": {"type": "immediate"}});
    let preview = f
        .post(&format!("{base}/plan-change/preview"), change.clone())
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{:#}", preview.body);
    assert_eq!(
        preview.body["subscription_after"]["plan"]["option"],
        "yearly"
    );
    assert_eq!(
        charge(&preview.body["subscription_after"], "platform")["prices"][0]["model"]["amount"],
        "2990"
    );
    let changed = f
        .post_once(&format!("{base}/plan-change"), "change-annual", change)
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{:#}", changed.body);
    assert_eq!(changed.body["subscription"]["plan"]["code"], "pro-cadence");
    assert_eq!(changed.body["subscription"]["plan"]["option"], "yearly");
}

/// Rules: ADR-0013 §10 (PATCH is one write: a refused field writes nothing)
#[tokio::test]
async fn test_rest_v2_subscription_patch_is_one_write() {
    let f = fixture().await;
    seed(&f).await;
    let mut ids = Vec::new();
    for (external_id, key) in [
        ("one-write-1", "one-write-1"),
        ("one-write-2", "one-write-2"),
    ] {
        let created = f
            .post_once(
                "/api/v2/subscriptions",
                key,
                json!({
                    "customer": "ext:comodo",
                    "plan": "scale",
                    "option": "monthly",
                    "external_id": external_id,
                    "quantities": {"users": "2"},
                    "invoicing": {"charge_automatically": false}
                }),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
        ids.push(created.body["id"].as_str().unwrap().to_owned());
    }
    let base = "/api/v2/subscriptions/one-write-1";
    let read = f.get(base).await;
    let period_end = date_of(&read.body["current_period"]["end"]);

    // An end date taken beside a refused invoicing change: neither is written.
    let refused = patch(
        &f,
        base,
        json!({
            "end_date": period_end.to_string(),
            "invoicing": {"memo": "Not kept", "charge_automatically": true}
        }),
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    let unchanged = f.get(base).await;
    assert!(
        unchanged.body["dates"].get("end").is_none(),
        "{:#}",
        unchanged.body
    );
    assert!(unchanged.body["invoicing"].get("memo").is_none());

    // Net terms past their bound are refused at their field, as on create.
    let refused = patch(&f, base, json!({"invoicing": {"net_terms": 366}})).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(refused.body["errors"][0]["path"], "/invoicing/net_terms");

    // An external id taken by another subscription after v2 checked it (a write racing this
    // one): the store refuses it under its transaction, the memo beside it is not written.
    use meteroid_store::repositories::CatalogCodesInterface;
    let id = f
        .setup
        .store
        .find_subscription_id_by_external_id(crate::data::ids::TENANT_ID, "one-write-1")
        .await
        .expect("the subscription");
    let memo = meteroid_store::domain::SubscriptionPatch {
        id,
        charge_automatically: None,
        auto_advance_invoices: None,
        net_terms: None,
        invoice_memo: Some(Some("Not kept either".to_owned())),
        purchase_order: None,
        payment_methods_config: None,
        custom_properties: None,
        resource_id: None,
    };
    let raced = f
        .setup
        .services
        .update_subscription(
            common_domain::actor::Actor::System,
            crate::data::ids::TENANT_ID,
            id,
            meteroid_store::domain::SubscriptionUpdate {
                end_date: None,
                patch: Some(memo),
                external_id: Some(Some("one-write-2".to_owned())),
            },
        )
        .await;
    let Err(error) = raced else {
        panic!("an external id another subscription holds is refused");
    };
    assert!(
        matches!(
            error.current_context(),
            meteroid_store::errors::StoreError::DuplicateValue { .. }
        ),
        "{error:?}"
    );
    let unchanged = f.get(base).await;
    assert_eq!(unchanged.body["external_id"], "one-write-1");
    assert!(unchanged.body["invoicing"].get("memo").is_none());
    let other = f.get(&format!("/api/v2/subscriptions/{}", ids[1])).await;
    assert_eq!(other.body["external_id"], "one-write-2");

    // Both together, accepted: written together.
    let taken = patch(
        &f,
        base,
        json!({"end_date": period_end.to_string(), "invoicing": {"memo": "Kept"}}),
    )
    .await;
    assert_eq!(taken.status, StatusCode::OK, "{:#}", taken.body);
    assert_eq!(date_of(&taken.body["dates"]["end"]), period_end);
    assert_eq!(taken.body["invoicing"]["memo"], "Kept");
}

/// A draft offering add-ons is discarded with its offers; a published version's stay.
/// Rules: ADR-0013 §5 (a draft discarded)
#[tokio::test]
async fn test_rest_v2_a_draft_offering_add_ons_is_discarded() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post(
            "/api/v2/plans",
            json!({"code": "offering", "name": "Offering", "draft": version("49", "12")}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let draft = f.get("/api/v2/plans/offering?version=draft").await;
    assert_eq!(
        draft.body["version"]["add_ons"][0]["add_on"], "support",
        "{:#}",
        draft.body
    );
    let discarded = f.delete("/api/v2/plans/offering/draft").await;
    assert_eq!(
        discarded.status,
        StatusCode::NO_CONTENT,
        "{:#}",
        discarded.body
    );
    assert_eq!(
        f.get("/api/v2/plans/offering").await.status,
        StatusCode::NOT_FOUND
    );

    // `studio` is published with the add-on: a new draft discarded leaves its offer.
    let drafted = f
        .put("/api/v2/plans/studio/draft", version("59", "12"))
        .await;
    assert_eq!(drafted.status, StatusCode::OK, "{:#}", drafted.body);
    let discarded = f.delete("/api/v2/plans/studio/draft").await;
    assert_eq!(
        discarded.status,
        StatusCode::NO_CONTENT,
        "{:#}",
        discarded.body
    );
    let studio = f.get("/api/v2/plans/studio?version=active").await;
    assert_eq!(studio.status, StatusCode::OK, "{:#}", studio.body);
    assert_eq!(
        studio.body["version"]["add_ons"][0]["add_on"], "support",
        "{:#}",
        studio.body
    );
}

/// The `site` resource type, keyed by `site_id`.
pub(crate) async fn seed_sites(f: &Fixture) {
    let created = f
        .post(
            "/api/v2/resource-types",
            json!({"code": "site", "name": "Site", "plural_name": "Sites", "keys": ["site_id"]}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
}

/// A stored site of `customer` (`ext:<alias>`), by its id.
pub(crate) async fn site(f: &Fixture, customer: &str, key: &str) -> String {
    let stored = f
        .put(
            &format!("/api/v2/customers/{customer}/resources/site/{key}"),
            json!({}),
        )
        .await;
    assert_eq!(stored.status, StatusCode::CREATED, "{:#}", stored.body);
    let id = stored.body["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("res_"), "{id}");
    id
}

fn on_studio(extra: Value) -> Value {
    let mut body = json!({
        "customer": "ext:spotify",
        "plan": "studio",
        "quantities": {"users": "1"},
        "invoicing": {"charge_automatically": false}
    });
    for (field, value) in extra.as_object().unwrap() {
        body[field] = value.clone();
    }
    body
}

fn refused_at(answer: &crate::test_rest_v2::Answer, path: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{path}: {:#}",
        answer.body
    );
    assert_eq!(answer.body["errors"][0]["path"], path, "{:#}", answer.body);
}

/// Rules: ADR-0013 §25 (a subscription for a resource, named by id or by type and key)
#[tokio::test]
async fn test_rest_v2_subscription_for_a_resource() {
    let f = fixture().await;
    seed(&f).await;
    seed_sites(&f).await;
    let s1 = site(&f, "ext:spotify", "s1").await;
    let s2 = site(&f, "ext:spotify", "s2").await;
    site(&f, "ext:spotify", "s3").await;
    site(&f, "ext:spotify", "s4").await;
    site(&f, "ext:spotify", "s5").await;
    let theirs = site(&f, "ext:uber", "u1").await;

    // By id and by type and key, read back as `{id, type, key}`; studio is sold for the customer,
    // and prices `bandwidth`, attributed to the customer only: a subscription for a site is
    // accepted with both warnings, and bills none of `bandwidth`.
    let by_id = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s1",
            on_studio(json!({"resource": {"id": s1}})),
        )
        .await;
    assert_eq!(by_id.status, StatusCode::CREATED, "{:#}", by_id.body);
    assert_eq!(
        by_id.body["resource"],
        json!({"id": s1, "type": "site", "key": "s1"})
    );
    assert_eq!(
        by_id.body["warnings"],
        json!(["plan_sold_for_mismatch", "metric_not_attributed"])
    );
    assert_eq!(by_id.body["metrics_not_attributed"], json!(["bandwidth"]));
    let on_s1 = by_id.body["id"].as_str().unwrap().to_owned();
    let read = f.get(&format!("/api/v2/subscriptions/{on_s1}")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["resource"], by_id.body["resource"]);
    assert_eq!(
        read.body["warnings"], by_id.body["warnings"],
        "a read tells what the creation told"
    );
    assert_eq!(
        read.body["metrics_not_attributed"],
        by_id.body["metrics_not_attributed"]
    );
    let summary = f
        .get(&format!("/api/v2/subscriptions/{on_s1}/summary"))
        .await;
    assert_eq!(summary.body["resource"], by_id.body["resource"]);

    let by_key = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s2",
            on_studio(json!({"resource": {"type": "site", "key": "s2"}})),
        )
        .await;
    assert_eq!(by_key.status, StatusCode::CREATED, "{:#}", by_key.body);
    assert_eq!(
        by_key.body["resource"],
        json!({"id": s2, "type": "site", "key": "s2"})
    );
    assert_eq!(
        by_key.body["warnings"],
        json!(["plan_sold_for_mismatch", "metric_not_attributed"])
    );

    let customer_wide = f
        .post_once("/api/v2/subscriptions", "for-spotify", on_studio(json!({})))
        .await;
    assert_eq!(
        customer_wide.status,
        StatusCode::CREATED,
        "{:#}",
        customer_wide.body
    );
    assert!(customer_wide.body.get("resource").is_none());
    assert!(
        customer_wide.body.get("warnings").is_none(),
        "{:#}",
        customer_wide.body
    );

    // A plan sold for sites pricing no metric, subscribed for one: nothing to warn of.
    let mut draft = version("29", "5");
    draft["charges"]
        .as_array_mut()
        .unwrap()
        .retain(|charge| charge["product"] != "api");
    let plan = f
        .post(
            "/api/v2/plans",
            json!({"code": "per-site", "name": "Per site", "sold_for": "site", "draft": draft}),
        )
        .await;
    assert_eq!(plan.status, StatusCode::CREATED, "{:#}", plan.body);
    assert_eq!(plan.body["sold_for"]["code"], "site");
    let published = f
        .post("/api/v2/plans/per-site/draft/publish", json!({}))
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:#}", published.body);
    let sold_for_sites = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s5",
            on_studio(json!({"plan": "per-site", "resource": {"type": "site", "key": "s5"}})),
        )
        .await;
    assert_eq!(
        sold_for_sites.status,
        StatusCode::CREATED,
        "{:#}",
        sold_for_sites.body
    );
    assert!(
        sold_for_sites.body.get("warnings").is_none(),
        "{:#}",
        sold_for_sites.body
    );

    // Refused at `/resource`: sold twice over one period, archived, another customer's.
    let twice = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s1-again",
            on_studio(json!({"resource": {"id": s1}})),
        )
        .await;
    refused_at(&twice, "/resource");
    let archived = f
        .post(
            "/api/v2/customers/ext:spotify/resources/site/s3/archive",
            json!({}),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    let on_archived = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s3",
            on_studio(json!({"resource": {"type": "site", "key": "s3"}})),
        )
        .await;
    refused_at(&on_archived, "/resource");
    let not_theirs = f
        .post_once(
            "/api/v2/subscriptions",
            "for-u1",
            on_studio(json!({"resource": {"id": theirs}})),
        )
        .await;
    refused_at(&not_theirs, "/resource");
    assert_eq!(not_theirs.body["errors"][0]["code"], "not_found");
    let unknown = f
        .post_once(
            "/api/v2/subscriptions",
            "for-s9",
            on_studio(json!({"resource": {"type": "site", "key": "s9"}})),
        )
        .await;
    refused_at(&unknown, "/resource");

    // Fixed once active: another resource is refused at `/resource`, the first one kept.
    let moved = f
        .patch(
            &format!("/api/v2/subscriptions/{on_s1}"),
            json!({"resource": {"type": "site", "key": "s4"}}),
        )
        .await;
    refused_at(&moved, "/resource");
    let kept = f.get(&format!("/api/v2/subscriptions/{on_s1}")).await;
    assert_eq!(kept.body["resource"]["key"], "s1");

    // A resource a subscription names is not deleted.
    let deleted = f
        .delete("/api/v2/customers/ext:spotify/resources/site/s1")
        .await;
    assert_eq!(deleted.status, StatusCode::CONFLICT, "{:#}", deleted.body);
}

fn history_len(answer: &crate::test_rest_v2::Answer) -> usize {
    assert_eq!(answer.status, StatusCode::OK, "{:#}", answer.body);
    answer.body["data"].as_array().unwrap().len()
}

fn projects_of(readout: &crate::test_rest_v2::Answer) -> Value {
    assert_eq!(readout.status, StatusCode::OK, "{:#}", readout.body);
    readout.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entitlement| entitlement["feature"]["code"] == "projects")
        .cloned()
        .unwrap_or_else(|| panic!("projects is granted: {:#}", readout.body))
}

/// Rules: ADR-0013 §25 (a subscription's own rows, written whole; its readout)
#[tokio::test]
async fn test_rest_v2_subscription_entitlement_overrides() {
    let f = fixture().await;
    seed(&f).await;
    let feature = f
        .post(
            "/api/v2/features",
            json!({"code": "projects", "name": "Projects", "unit": "projects",
                   "type": {"type": "limit", "counted_by": "app"}}),
        )
        .await;
    assert_eq!(feature.status, StatusCode::CREATED, "{:#}", feature.body);
    let five = json!([{"feature": "projects", "value": {"type": "limit", "limit": "5"}}]);

    // Refused at their row: an effect the subscription tier does not take, a row twice.
    let added = f
        .post_once(
            "/api/v2/subscriptions",
            "overrides-add",
            on_studio(json!({"entitlement_overrides": [
                {"feature": "projects", "value": {"type": "limit", "limit": "5"}, "effect": "add"}
            ]})),
        )
        .await;
    refused_at(&added, "/entitlement_overrides/0/effect");
    let twice = f
        .post_once(
            "/api/v2/subscriptions",
            "overrides-twice",
            on_studio(json!({"entitlement_overrides": [
                {"feature": "projects", "value": {"type": "limit", "limit": "5"}},
                {"feature": "projects", "value": {"type": "limit", "limit": "6"}}
            ]})),
        )
        .await;
    refused_at(&twice, "/entitlement_overrides/1/feature");
    assert_eq!(twice.body["errors"][0]["code"], "duplicate");
    let unknown = f
        .post_once(
            "/api/v2/subscriptions",
            "overrides-unknown",
            on_studio(json!({"entitlement_overrides": [
                {"feature": "no-such-feature", "value": {"type": "on"}}
            ]})),
        )
        .await;
    refused_at(&unknown, "/entitlement_overrides/0/feature");

    // Written with the subscription, read back on a single one.
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "overrides-1",
            on_studio(json!({"entitlement_overrides": five.clone()})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let first = created.body["id"].as_str().unwrap().to_owned();
    let base = format!("/api/v2/subscriptions/{first}");
    let rows = created.body["entitlement_overrides"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{:#}", created.body);
    let row = &rows[0];
    let row_id = row["id"].as_str().unwrap().to_owned();
    assert!(row_id.starts_with("ent_"), "{row_id}");
    assert_eq!(row["feature"], "projects");
    assert_eq!(row["effect"], "set");
    assert_eq!(row["level"], "customer");
    assert_eq!(row["value"], json!({"type": "limit", "limit": "5"}));
    assert_eq!(row["owner"]["type"], "subscription");
    assert_eq!(row["owner"]["id"], first.as_str());
    assert_eq!(
        f.get(&base).await.body["entitlement_overrides"],
        created.body["entitlement_overrides"]
    );
    assert!(
        f.get(&format!("{base}/summary"))
            .await
            .body
            .get("entitlement_overrides")
            .is_none(),
        "a summary leaves them out"
    );
    let history = format!("/api/v2/entitlements/{row_id}/history");

    // The same list again changes nothing; a changed row is succeeded under its id.
    let same = f
        .patch(&base, json!({"entitlement_overrides": five.clone()}))
        .await;
    assert_eq!(same.status, StatusCode::OK, "{:#}", same.body);
    assert_eq!(same.body["entitlement_overrides"][0]["id"], row_id.as_str());
    assert_eq!(history_len(&f.get(&history).await), 1);
    let eight = json!([{"feature": "projects", "value": {"type": "limit", "limit": "8"}}]);
    let changed = f
        .patch(&base, json!({"entitlement_overrides": eight.clone()}))
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{:#}", changed.body);
    assert_eq!(
        changed.body["entitlement_overrides"][0]["id"],
        row_id.as_str()
    );
    assert_eq!(
        changed.body["entitlement_overrides"][0]["value"]["limit"],
        "8"
    );
    let versions = f.get(&history).await;
    assert_eq!(history_len(&versions), 2);
    assert_eq!(
        versions.body["data"][0]["value"]["limit"], "8",
        "newest first"
    );
    assert!(versions.body["data"][1]["revoked_at"].is_string());
    let refused = f
        .patch(
            &base,
            json!({"entitlement_overrides": [
                {"feature": "projects", "value": {"type": "limit", "limit": "9"}, "effect": "cap_at"}
            ]}),
        )
        .await;
    refused_at(&refused, "/entitlement_overrides/0/effect");
    assert_eq!(history_len(&f.get(&history).await), 2, "nothing written");

    // The readout: the subscription's level with the count the app reported.
    let reported = f
        .post_once(
            "/api/v2/customers/ext:spotify/entitlements/projects/usage",
            "projects-set-3",
            json!({"set": "3"}),
        )
        .await;
    assert_eq!(reported.status, StatusCode::OK, "{:#}", reported.body);
    assert_eq!(reported.body["count"], "3");
    let readout = f.get(&format!("{base}/entitlements")).await;
    let projects = projects_of(&readout);
    assert_eq!(projects["level"], "customer");
    assert!(projects.get("resource").is_none(), "{projects:#}");
    assert_eq!(projects["type"], "limit");
    assert_eq!(projects["unit"], "projects");
    assert_eq!(projects["value"], json!({"type": "limit", "limit": "8"}));
    assert_eq!(projects["used"], "3");
    assert_eq!(projects["policy"]["at_limit"], "deny_new");
    assert_eq!(projects["policy"]["resets"]["type"], "never");
    assert!(projects.get("window").is_none(), "{projects:#}");
    assert_eq!(projects["deciding_subscription"]["id"], first.as_str());
    assert_eq!(projects["origin"]["entitlement"], row_id.as_str());
    assert_eq!(projects["origin"]["owner"]["type"], "subscription");
    assert!(projects.get("own_grant").is_none(), "{projects:#}");

    // A second subscription granting more decides (the best value): the first's grant is its own.
    let second = f
        .post_once(
            "/api/v2/subscriptions",
            "overrides-2",
            on_studio(json!({"entitlement_overrides": [
                {"feature": "projects", "value": {"type": "limit", "limit": "100"}}
            ]})),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{:#}", second.body);
    let second_id = second.body["id"].as_str().unwrap().to_owned();
    let decided = projects_of(&f.get(&format!("{base}/entitlements")).await);
    assert_eq!(decided["deciding_subscription"]["id"], second_id.as_str());
    assert_eq!(decided["value"], json!({"type": "limit", "limit": "100"}));
    assert_eq!(decided["used"], "3");
    let own_grant = &decided["own_grant"];
    assert_eq!(own_grant["value"], json!({"type": "limit", "limit": "8"}));
    assert_eq!(own_grant["deciding_subscription"]["id"], second_id.as_str());
    assert_eq!(own_grant["deciding_plan_name"], "Studio");
    assert!(
        own_grant.get("deciding_resource").is_none(),
        "{own_grant:#}"
    );
    let deciding = projects_of(
        &f.get(&format!("/api/v2/subscriptions/{second_id}/entitlements"))
            .await,
    );
    assert_eq!(deciding["deciding_subscription"]["id"], second_id.as_str());
    assert!(deciding.get("own_grant").is_none(), "{deciding:#}");

    // `[]` revokes them: the row is gone, its history kept.
    let emptied = f.patch(&base, json!({"entitlement_overrides": []})).await;
    assert_eq!(emptied.status, StatusCode::OK, "{:#}", emptied.body);
    assert_eq!(emptied.body["entitlement_overrides"], json!([]));
    assert_eq!(f.get(&base).await.body["entitlement_overrides"], json!([]));
    let gone = f.get(&format!("/api/v2/entitlements/{row_id}")).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{:#}", gone.body);
    let kept = f.get(&history).await;
    assert_eq!(history_len(&kept), 2);
    assert!(
        kept.body["data"][0]["revoked_at"].is_string(),
        "{:#}",
        kept.body
    );
    let after = f.get(&format!("{base}/entitlements")).await;
    assert_eq!(after.status, StatusCode::OK, "{:#}", after.body);
    assert!(
        after.body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entitlement| entitlement["feature"]["code"] != "projects"),
        "no row of its own, it no longer includes projects: {:#}",
        after.body
    );
}

//! Customers through the public REST API v2 (ADR-0013 §11): created and replayed by their
//! Idempotency-Key, read by alias, listed by cursor, patched (RFC 7396), archived and restored,
//! their portal token, entitlements and credit.
use crate::data::ids::TENANT_ID;
use crate::data::payment::run_mock_payment_provider_seed;
use crate::test_rest_v2::{Answer, Fixture, fixture};
use common_domain::ids::CustomerId;
use meteroid_store::domain::credit::GrantClassification;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::str::FromStr;

async fn patch(f: &Fixture, path: &str, body: Value) -> Answer {
    f.send(
        f.client
            .patch(f.url(path))
            .bearer_auth(&f.api_key)
            .header("content-type", "application/merge-patch+json")
            .body(body.to_string()),
    )
    .await
}

fn ids(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|customer| customer["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn test_rest_v2_customers() {
    let f = fixture().await;

    // Created as written: addresses, tax and invoicing nested, snake_case, ids prefixed.
    let acme = json!({
        "alias": "acme-v2",
        "name": "Acme",
        "email": "billing@acme.test",
        "phone": "+33 1 23 45 67 89",
        "currency": "EUR",
        "billing_address": {"line1": "1 rue de la Paix", "city": "Paris", "postal_code": "75002", "country": "FR"},
        "shipping_address": {"same_as_billing": true},
        "tax": {"vat_number": "FR40303265045"},
        "invoicing": {"emails": ["ap@acme.test"], "preferred_locales": ["fr-FR"]}
    });
    let created = f
        .post_once("/api/v2/customers", "create-acme", acme.clone())
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let customer = &created.body;
    let id = customer["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("cus_"), "{id}");
    assert_eq!(customer["alias"], "acme-v2");
    assert_eq!(customer["type"], "company");
    assert_eq!(customer["name"], "Acme");
    assert_eq!(customer["currency"], "EUR");
    assert_eq!(customer["billing_address"]["postal_code"], "75002");
    assert_eq!(customer["billing_address"]["country"], "FR");
    assert_eq!(customer["shipping_address"]["same_as_billing"], true);
    assert_eq!(customer["tax"]["status"], "taxable");
    assert_eq!(customer["tax"]["vat_number"], "FR40303265045");
    assert_eq!(customer["invoicing"]["emails"], json!(["ap@acme.test"]));
    assert_eq!(customer["invoicing"]["preferred_locales"], json!(["fr-FR"]));
    assert!(
        customer["invoicing"]["entity"]
            .as_str()
            .unwrap()
            .starts_with("ive_")
    );
    assert_eq!(customer["archived"], false);
    assert!(customer.get("zip_code").is_none() && customer.get("billing_email").is_none());

    // The same key and body replays the first answer; another body under it is refused.
    let replay = f.post_once("/api/v2/customers", "create-acme", acme).await;
    assert_eq!(replay.status, StatusCode::CREATED);
    assert_eq!(replay.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replay.body["id"], id.as_str());
    let reused = f
        .post_once(
            "/api/v2/customers",
            "create-acme",
            json!({"name": "Other", "currency": "EUR"}),
        )
        .await;
    assert_eq!(reused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reused.body["code"], "idempotency_key_reused");

    // Refused at their JSON pointer.
    let taken = f
        .post(
            "/api/v2/customers",
            json!({"alias": "acme-v2", "name": "Acme again", "currency": "EUR"}),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{:#}", taken.body);
    assert_eq!(taken.body["errors"][0]["path"], "/alias");
    let nameless = f
        .post("/api/v2/customers", json!({"currency": "EUR"}))
        .await;
    assert_eq!(nameless.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(nameless.body["errors"][0]["path"], "/name");
    let unrendered = f
        .post(
            "/api/v2/customers",
            json!({"name": "Babel", "currency": "EUR", "invoicing": {"preferred_locales": ["xx-XX"]}}),
        )
        .await;
    assert_eq!(unrendered.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unrendered.body["errors"][0]["path"],
        "/invoicing/preferred_locales"
    );

    // Read by id and by alias; a bare alias is not an id.
    let by_alias = f.get("/api/v2/customers/ext:acme-v2").await;
    assert_eq!(by_alias.status, StatusCode::OK, "{:#}", by_alias.body);
    assert_eq!(by_alias.body["id"], id.as_str());
    let by_id = f.get(&format!("/api/v2/customers/{id}")).await;
    assert_eq!(by_id.body["alias"], "acme-v2");
    let bare = f.get("/api/v2/customers/acme-v2").await;
    assert_eq!(bare.status, StatusCode::NOT_FOUND);
    assert_eq!(bare.body["code"], "not_found");

    // Listed by cursor, in creation order, narrowed by a search.
    for (alias, name) in [("beta-v2", "Beta"), ("gamma-v2", "Gamma")] {
        let created = f
            .post(
                "/api/v2/customers",
                json!({"alias": alias, "name": name, "currency": "EUR"}),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    }
    let first = f.get("/api/v2/customers?search=-v2&limit=2").await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(first.body["data"].as_array().unwrap().len(), 2);
    assert_eq!(first.body["has_more"], true);
    assert_eq!(first.body["data"][0]["alias"], "acme-v2");
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "/api/v2/customers?search=-v2&limit=2&after={cursor}"
        ))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(second.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(second.body["data"][0]["alias"], "gamma-v2");
    assert_eq!(second.body["has_more"], false);
    assert!(second.body["next_cursor"].is_null());
    let mut all = ids(&first.body);
    all.extend(ids(&second.body));
    assert_eq!(all.len(), 3);
    assert!(all.contains(&id));
    let whole = f.get("/api/v2/customers?limit=100").await;
    assert!(
        ids(&whole.body).len() > 3,
        "the seeded customers are listed too"
    );
    let since_ever = f
        .get("/api/v2/customers?search=-v2&updated_after=2000-01-01T00:00:00Z")
        .await;
    assert_eq!(since_ever.body["data"].as_array().unwrap().len(), 3);
    let from_the_future = f
        .get("/api/v2/customers?search=-v2&updated_after=2999-01-01T00:00:00Z")
        .await;
    assert_eq!(from_the_future.body["data"].as_array().unwrap().len(), 0);
    let bad_limit = f.get("/api/v2/customers?limit=0").await;
    assert_eq!(bad_limit.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A merge patch: null clears, an object merges, a list replaces, what is left out stays.
    let patched = patch(
        &f,
        "/api/v2/customers/ext:acme-v2",
        json!({
            "email": null,
            "phone": "+33 9 99 99 99 99",
            "billing_address": {"city": "Lyon", "postal_code": "69001"},
            "tax": {"legal_number": "303265045"},
            "invoicing": {"emails": ["finance@acme.test", "ap@acme.test"]}
        }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    let after = &patched.body;
    assert!(after.get("email").is_none(), "{after:#}");
    assert_eq!(after["phone"], "+33 9 99 99 99 99");
    assert_eq!(after["billing_address"]["city"], "Lyon");
    assert_eq!(after["billing_address"]["line1"], "1 rue de la Paix");
    assert_eq!(after["tax"]["vat_number"], "FR40303265045");
    assert_eq!(after["tax"]["legal_number"], "303265045");
    assert_eq!(
        after["invoicing"]["emails"],
        json!(["finance@acme.test", "ap@acme.test"])
    );
    assert_eq!(after["invoicing"]["preferred_locales"], json!(["fr-FR"]));
    assert_eq!(after["name"], "Acme");
    assert_eq!(after["alias"], "acme-v2");
    let unreadable = patch(&f, "/api/v2/customers/ext:acme-v2", json!({"phone": 42})).await;
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    let currencyless = patch(
        &f,
        "/api/v2/customers/ext:acme-v2",
        json!({"currency": null}),
    )
    .await;
    assert_eq!(currencyless.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Archived and restored, each answering the customer; the live list leaves it out.
    let archived = f
        .post("/api/v2/customers/ext:gamma-v2/archive", json!({}))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    assert!(archived.body["archived_at"].is_string());
    let live = f.get("/api/v2/customers?search=-v2").await;
    assert_eq!(live.body["data"].as_array().unwrap().len(), 2);
    let gone = f.get("/api/v2/customers?search=-v2&archived=true").await;
    assert_eq!(gone.body["data"][0]["alias"], "gamma-v2");
    let read_archived = f.get("/api/v2/customers/ext:gamma-v2").await;
    assert_eq!(read_archived.status, StatusCode::OK);
    assert_eq!(read_archived.body["archived"], true);
    let restored = f
        .post("/api/v2/customers/ext:gamma-v2/unarchive", json!({}))
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{:#}", restored.body);
    assert_eq!(restored.body["archived"], false);
    assert!(restored.body.get("archived_at").is_none());

    // v2 has no DELETE of a customer: archiving is `POST …/archive`, which refuses a customer
    // with a live subscription.
    let deleted = f.delete("/api/v2/customers/ext:gamma-v2").await;
    assert_eq!(
        deleted.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{:#}",
        deleted.body
    );
    assert_eq!(
        f.get("/api/v2/customers/ext:gamma-v2").await.body["archived"],
        false
    );
    crate::test_rest_v2_subscriptions::seed(&f).await;
    let subscription = f
        .post_once(
            "/api/v2/subscriptions",
            "subscribe-beta",
            json!({
                "customer": "ext:beta-v2",
                "plan": "studio",
                "quantities": {"users": "1"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(
        subscription.status,
        StatusCode::CREATED,
        "{:#}",
        subscription.body
    );
    let subscribed = f
        .post("/api/v2/customers/ext:beta-v2/archive", json!({}))
        .await;
    assert_eq!(
        subscribed.status,
        StatusCode::CONFLICT,
        "{:#}",
        subscribed.body
    );
    assert_eq!(subscribed.body["code"], "conflict");
    assert_eq!(
        f.get("/api/v2/customers/ext:beta-v2").await.body["archived"],
        false
    );

    // The portal token: a body may be left out; an empty scope list is refused.
    let token = f
        .post(
            "/api/v2/customers/ext:acme-v2/portal-token",
            json!({"scopes": ["read"], "expires_in": 3600}),
        )
        .await;
    assert_eq!(token.status, StatusCode::CREATED, "{:#}", token.body);
    let value = token.body["token"].as_str().unwrap();
    assert!(!value.is_empty());
    assert!(
        token.body["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/portal/customer?token={value}"))
    );
    assert!(token.body["expires_at"].is_string());
    let bodiless = f
        .send(
            f.client
                .post(f.url("/api/v2/customers/ext:acme-v2/portal-token"))
                .bearer_auth(&f.api_key),
        )
        .await;
    assert_eq!(bodiless.status, StatusCode::CREATED, "{:#}", bodiless.body);
    let scopeless = f
        .post(
            "/api/v2/customers/ext:acme-v2/portal-token",
            json!({"scopes": []}),
        )
        .await;
    assert_eq!(scopeless.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(scopeless.body["errors"][0]["path"], "/scopes");

    // Entitlements: a page of them, and none to a feature that does not exist.
    let entitlements = f.get("/api/v2/customers/ext:acme-v2/entitlements").await;
    assert_eq!(
        entitlements.status,
        StatusCode::OK,
        "{:#}",
        entitlements.body
    );
    assert!(entitlements.body["data"].is_array());
    assert_eq!(entitlements.body["has_more"], false);
    let unknown = f
        .get("/api/v2/customers/ext:acme-v2/entitlements/no-such-feature")
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    // The v2 document carries them.
    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    assert!(spec.body["paths"]["/api/v2/customers/{id_or_alias}"]["patch"].is_object());
    assert!(
        spec.body["paths"]["/api/v2/customers/{id_or_alias}"]
            .get("delete")
            .is_none()
    );
    assert!(spec.body["paths"]["/api/v2/customers/{id_or_alias}/credit-ledger"]["get"].is_object());
}

#[tokio::test]
async fn test_rest_v2_customer_credit() {
    let f = fixture().await;
    let created = f
        .post(
            "/api/v2/customers",
            json!({"alias": "credit-v2", "name": "Credit Co", "currency": "EUR"}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let grants = "/api/v2/customers/ext:credit-v2/credit-grants";

    // Granting credit takes an Idempotency-Key, and replays by it.
    let grant = json!({"amount": 5_000, "type": "promotional", "reason": "Welcome"});
    let keyless = f.post(grants, grant.clone()).await;
    assert_eq!(keyless.status, StatusCode::BAD_REQUEST);
    assert_eq!(keyless.body["code"], "idempotency_key_required");
    assert_eq!(keyless.body["errors"][0]["path"], "Idempotency-Key");
    let granted = f.post_once(grants, "grant-1", grant.clone()).await;
    assert_eq!(granted.status, StatusCode::CREATED, "{:#}", granted.body);
    let body = &granted.body;
    assert!(body["id"].as_str().unwrap().starts_with("cgr_"));
    assert_eq!(body["type"], "promotional");
    assert_eq!(body["status"], "active");
    assert_eq!(body["source"], "manual");
    assert_eq!(body["currency"], "EUR");
    assert_eq!(body["applies_to"], "all");
    assert_eq!(body["amount"], 5_000);
    assert_eq!(body["remaining"], 5_000);
    assert_eq!(body["reason"], "Welcome");
    let replay = f.post_once(grants, "grant-1", grant).await;
    assert_eq!(replay.status, StatusCode::CREATED);
    assert_eq!(replay.body["id"], body["id"]);
    let refused = f
        .post_once(
            grants,
            "grant-2",
            json!({"amount": 0, "type": "promotional"}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["errors"][0]["path"], "/amount");

    // Read back: the wallet, the grants, the ledger.
    let wallets = f.get("/api/v2/customers/ext:credit-v2/wallets").await;
    assert_eq!(wallets.status, StatusCode::OK, "{:#}", wallets.body);
    let eur = &wallets.body["data"][0];
    assert_eq!(eur["currency"], "EUR");
    assert_eq!(eur["credit_on_account"], 5_000);
    assert_eq!(eur["usage_at_zero"], "follow_plan");
    let listed = f.get(grants).await;
    assert_eq!(listed.status, StatusCode::OK, "{:#}", listed.body);
    assert_eq!(listed.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed.body["data"][0]["id"], body["id"]);
    let second = f
        .post_once(
            grants,
            "grant-3",
            json!({"amount": 1_000, "type": "promotional", "reason": "Goodwill"}),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{:#}", second.body);
    let ledger = f
        .get("/api/v2/customers/ext:credit-v2/credit-ledger?limit=1")
        .await;
    assert_eq!(ledger.status, StatusCode::OK, "{:#}", ledger.body);
    assert_eq!(ledger.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(ledger.body["data"][0]["type"], "credit");
    assert_eq!(ledger.body["has_more"], true);
    let cursor = ledger.body["next_cursor"].as_str().unwrap().to_owned();
    let rest = f
        .get(&format!(
            "/api/v2/customers/ext:credit-v2/credit-ledger?limit=1&after={cursor}"
        ))
        .await;
    assert_eq!(rest.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(rest.body["has_more"], false);
    assert_ne!(rest.body["data"][0]["id"], ledger.body["data"][0]["id"]);
    assert_eq!(rest.body["data"][0]["balance_after"], 5_000);
    assert_eq!(ledger.body["data"][0]["balance_after"], 6_000);

    // Wallet settings: the usage controls are not in this edition; a patch of nothing opens it.
    let controls = patch(
        &f,
        "/api/v2/customers/ext:credit-v2/wallets/EUR",
        json!({"low_balance_thresholds": [1_000]}),
    )
    .await;
    assert_eq!(
        controls.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        controls.body
    );
    let opened = patch(&f, "/api/v2/customers/ext:credit-v2/wallets/EUR", json!({})).await;
    assert_eq!(opened.status, StatusCode::OK, "{:#}", opened.body);
    assert_eq!(opened.body["currency"], "EUR");
    assert_eq!(opened.body["credit_on_account"], 6_000);

    // Top-ups: invoiced, and replayed by their key; charged without a payment method (Comodo has
    // none), the customer pays at a checkout.
    run_mock_payment_provider_seed(&f.setup.store.pool, false).await;
    let top_ups = "/api/v2/customers/ext:uber/top-ups";
    let invoiced = json!({"amount": 100_000, "collection": "send_invoice"});
    let issued = f.post_once(top_ups, "top-up-1", invoiced.clone()).await;
    assert_eq!(issued.status, StatusCode::CREATED, "{:#}", issued.body);
    assert_eq!(issued.body["status"], "awaiting_payment");
    assert_eq!(issued.body["amount"], 100_000);
    assert!(
        issued.body["invoice"]["id"]
            .as_str()
            .unwrap()
            .starts_with("inv_")
    );
    assert!(issued.body["invoice"]["number"].is_string());
    assert!(
        issued.body["checkout_url"]
            .as_str()
            .is_some_and(|url| url.contains("/portal/invoice-payment?token="))
    );
    let replayed = f.post_once(top_ups, "top-up-1", invoiced).await;
    assert_eq!(replayed.body["invoice"], issued.body["invoice"]);
    let checkout = f
        .post_once(
            "/api/v2/customers/ext:comodo/top-ups",
            "top-up-2",
            json!({"amount": 2_000}),
        )
        .await;
    assert_eq!(checkout.status, StatusCode::CREATED, "{:#}", checkout.body);
    assert_eq!(checkout.body["status"], "requires_customer_action");
    assert!(checkout.body["checkout_url"].is_string());

    // Grants page by cursor, newest first; a cursor of another list, or none at all, is refused.
    let newest = f.get(&format!("{grants}?limit=1")).await;
    assert_eq!(newest.status, StatusCode::OK, "{:#}", newest.body);
    assert_eq!(newest.body["data"][0]["reason"], "Goodwill");
    assert_eq!(newest.body["has_more"], true);
    let grant_cursor = newest.body["next_cursor"].as_str().unwrap().to_owned();
    let oldest = f
        .get(&format!("{grants}?limit=1&after={grant_cursor}"))
        .await;
    assert_eq!(oldest.status, StatusCode::OK, "{:#}", oldest.body);
    assert_eq!(oldest.body["data"][0]["id"], body["id"]);
    assert_eq!(oldest.body["has_more"], false);
    assert!(oldest.body["next_cursor"].is_null());
    let ledger_path = "/api/v2/customers/ext:credit-v2/credit-ledger";
    let first = f.get(&format!("{ledger_path}?limit=1")).await;
    let ledger_cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    for after in [ledger_cursor.as_str(), "not-a-cursor"] {
        let refused = f.get(&format!("{grants}?after={after}")).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], "after");
    }
    let refused = f.get(&format!("{ledger_path}?after={grant_cursor}")).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);

    // The ledger pages by its order (when a movement took effect, then when it was recorded):
    // credit recorded between two pages with an earlier date shows on a later page, and no page
    // skips or repeats a movement. Balances run over the whole statement.
    assert_eq!(first.body["data"][0]["amount"], 1_000);
    let customer_id = CustomerId::from_str(created.body["id"].as_str().unwrap()).unwrap();
    let wallet = f
        .setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer_id, "EUR")
        .await
        .unwrap();
    f.setup
        .store
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id,
                wallet_id: wallet.id,
                amount_minor: 500,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Backdated".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            chrono::Utc::now() - chrono::Duration::days(1),
        )
        .await
        .unwrap();
    let second = f
        .get(&format!("{ledger_path}?limit=1&after={ledger_cursor}"))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(second.body["data"][0]["amount"], 5_000);
    assert_eq!(second.body["data"][0]["balance_after"], 5_500);
    assert_eq!(second.body["has_more"], true);
    let cursor = second.body["next_cursor"].as_str().unwrap().to_owned();
    let third = f
        .get(&format!("{ledger_path}?limit=1&after={cursor}"))
        .await;
    assert_eq!(third.status, StatusCode::OK, "{:#}", third.body);
    assert_eq!(third.body["data"][0]["amount"], 500);
    assert_eq!(third.body["data"][0]["balance_after"], 500);
    assert_eq!(third.body["has_more"], false);
    let mut seen: Vec<String> = [&first, &second, &third]
        .iter()
        .map(|page| page.body["data"][0]["id"].as_str().unwrap().to_owned())
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3, "{seen:?}");
    let whole = f.get(&format!("{ledger_path}?limit=100")).await;
    assert_eq!(whole.body["data"].as_array().unwrap().len(), 3);
    assert_eq!(whole.body["data"][0]["balance_after"], 6_500);
}

/// Rules: ADR-0013 §18 (core reads custom property definitions; writing them is a module's)
#[tokio::test]
async fn test_rest_v2_custom_property_definitions_read_in_core() {
    use meteroid_store::domain::custom_property_definitions::{
        CustomPropertyDefinitionNew, PropertyConfig,
    };
    use meteroid_store::domain::enums::{CustomPropertyEntityType, CustomPropertyType};
    use meteroid_testkit::custom_properties::{
        archive_custom_property_definition, create_custom_property_definition,
    };

    let f = fixture().await;
    let define = |entity_type, key: &str| CustomPropertyDefinitionNew {
        tenant_id: crate::data::ids::TENANT_ID,
        entity_type,
        key: key.to_owned(),
        name: key.to_owned(),
        description: None,
        property_type: CustomPropertyType::Number,
        config: PropertyConfig {
            min: Some(0.0),
            ..PropertyConfig::default()
        },
        required: false,
        default_value: None,
        display_order: 0,
    };
    let seats = create_custom_property_definition(
        &f.setup.store,
        define(CustomPropertyEntityType::Customer, "seats_estimate"),
    )
    .await;
    let po = create_custom_property_definition(
        &f.setup.store,
        define(CustomPropertyEntityType::Invoice, "po_number"),
    )
    .await;
    archive_custom_property_definition(&f.setup.store, crate::data::ids::TENANT_ID, po.id).await;

    let path = "/api/v2/custom-property-definitions";
    let live = f.get(path).await;
    assert_eq!(live.status, StatusCode::OK, "{:#}", live.body);
    assert_eq!(
        live.body["data"].as_array().unwrap().len(),
        1,
        "{:#}",
        live.body
    );
    let read = &live.body["data"][0];
    assert!(read["id"].as_str().unwrap().starts_with("cpd_"));
    assert_eq!(read["key"], "seats_estimate");
    assert_eq!(read["entity_type"], "customer");
    assert_eq!(read["type"], "number");
    assert_eq!(read["config"]["min"], 0.0);
    assert_eq!(read["archived"], false);
    let all = f.get(&format!("{path}?include_archived=true")).await;
    assert_eq!(
        all.body["data"].as_array().unwrap().len(),
        2,
        "{:#}",
        all.body
    );
    let invoices = f
        .get(&format!("{path}?entity_type=invoice&include_archived=true"))
        .await;
    assert_eq!(invoices.body["data"][0]["key"], "po_number");
    assert_eq!(invoices.body["data"][0]["archived"], true);
    let one = f
        .get(&format!("{path}/{}", read["id"].as_str().unwrap()))
        .await;
    assert_eq!(one.status, StatusCode::OK, "{:#}", one.body);
    assert_eq!(&one.body, read);
    assert_eq!(seats.key, "seats_estimate");
    let bad_type = f.get(&format!("{path}?entity_type=planet")).await;
    assert_eq!(
        bad_type.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        bad_type.body
    );
    assert_eq!(bad_type.body["errors"][0]["path"], "entity_type");

    // Writing them is the Enterprise module's: core serves no such route.
    let create = f
        .post(
            path,
            json!({"entity_type": "customer", "key": "tier", "name": "Tier", "type": "text"}),
        )
        .await;
    assert!(
        matches!(
            create.status,
            StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND
        ),
        "{}: {:#}",
        create.status,
        create.body
    );

    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    let item = &spec.body["paths"][path];
    assert_eq!(item["get"]["x-meteroid-edition"], "oss", "{item:#}");
    assert!(item.get("post").is_none(), "{item:#}");
}

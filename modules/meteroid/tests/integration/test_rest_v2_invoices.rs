//! API v2 invoices and credit notes (ADR-0013 §11): a subscription created through v2 issues its
//! invoice, read with its lines and listed by filter and cursor; it is issued, paid by hand once
//! whatever the retries, its files asked for; the invoices a change issues are readable here; a
//! credit note against it is read, listed and linked back.
use crate::data::ids::TENANT_ID;
use crate::test_rest_v2::{Answer, Fixture, fixture, moment_from};
use crate::test_rest_v2_subscriptions::seed;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, InvoiceId};
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::str::FromStr;

async fn patch(f: &Fixture, path: &str, body: Value) -> Answer {
    f.send(
        f.client
            .patch(f.url(path))
            .bearer_auth(&f.api_key)
            .header("Content-Type", "application/merge-patch+json")
            .body(body.to_string()),
    )
    .await
}

/// A monthly subscription to `studio` of `customer`, invoiced without a payment provider.
async fn subscribe(f: &Fixture, customer: &str, external_id: &str) -> String {
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            &format!("create-{external_id}"),
            json!({
                "customer": customer,
                "plan": "studio",
                "option": "monthly",
                "external_id": external_id,
                "quantities": {"users": "3"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    created.body["id"].as_str().unwrap().to_owned()
}

fn ids_of(page: &Answer) -> Vec<String> {
    page.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("not a page: {:#}", page.body))
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The subscription's one invoice, issued.
async fn issued_invoice(f: &Fixture, subscription: &str) -> Value {
    let page = f
        .get(&format!("/api/v2/invoices?subscription={subscription}"))
        .await;
    assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
    let ids = ids_of(&page);
    assert!(!ids.is_empty(), "the subscription issued no invoice");
    let id = &ids[0];
    let invoice = f.get(&format!("/api/v2/invoices/{id}")).await;
    assert_eq!(invoice.status, StatusCode::OK, "{:#}", invoice.body);
    if invoice.body["status"] == "draft" {
        let finalized = f
            .post(&format!("/api/v2/invoices/{id}/finalize"), json!({}))
            .await;
        assert_eq!(finalized.status, StatusCode::OK, "{:#}", finalized.body);
        assert_eq!(finalized.body["status"], "finalized");
        return finalized.body;
    }
    invoice.body
}

/// A file a document has (`files.<kind>`) is served; one it has not is a `404` problem.
async fn file(f: &Fixture, path: &str, document: &Value, kind: &str, content_type: &str) {
    let answer = f.get(&format!("{path}/{kind}")).await;
    if document["files"][kind] == true {
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.header("content-type"), Some(content_type));
    } else {
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{:#}", answer.body);
        assert_eq!(answer.body["code"], "not_found");
    }
}

/// Rules: ADR-0013 §11
#[tokio::test]
async fn test_rest_v2_invoice_read_listed_and_paid() {
    let f = fixture().await;
    seed(&f).await;
    let subscription = subscribe(&f, "ext:spotify", "inv-spotify").await;
    let other = subscribe(&f, "ext:uber", "inv-uber").await;

    // The subscription's invoice, read whole.
    let invoice = issued_invoice(&f, "inv-spotify").await;
    let id = invoice["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("inv_"), "{id}");
    // What it names reads as references; the customer as printed beside.
    assert_eq!(invoice["subscription"]["id"], subscription.as_str());
    assert_eq!(invoice["customer"]["alias"], "spotify");
    assert!(
        invoice["customer"]["id"]
            .as_str()
            .unwrap()
            .starts_with("cus_")
    );
    assert!(invoice["customer"]["name"].is_string());
    assert_eq!(invoice["customer_details"]["alias"], "spotify");
    assert!(invoice["customer"].get("billing_address").is_none());
    assert_eq!(invoice["currency"], "EUR");
    assert!(invoice["number"].is_string());
    let lines = invoice["lines"].as_array().unwrap();
    assert!(!lines.is_empty(), "{invoice:#}");
    let products: Vec<&str> = lines
        .iter()
        .filter_map(|line| line["product"]["code"].as_str())
        .collect();
    assert!(
        products.contains(&"platform") && products.contains(&"seat"),
        "lines name their product by code: {products:?}"
    );
    let subtotal: i64 = lines
        .iter()
        .map(|line| line["amount"].as_i64().unwrap())
        .sum();
    assert_eq!(invoice["subtotal"], subtotal);
    let seat = lines
        .iter()
        .find(|line| line["product"]["code"] == "seat")
        .unwrap();
    assert!(seat["product"]["id"].as_str().unwrap().starts_with("prd_"));
    assert!(
        seat["quantity"].is_string(),
        "a quantity is a decimal string"
    );
    assert_eq!(
        invoice["amount_due"].as_i64().unwrap()
            + invoice["amount_paid"].as_i64().unwrap()
            + invoice["applied_credits"].as_i64().unwrap(),
        invoice["total"].as_i64().unwrap() + invoice["carried_balance"].as_i64().unwrap()
    );
    assert_eq!(invoice["payment_status"], "unpaid");
    assert_eq!(invoice["payments"], json!([]));
    assert_eq!(invoice["credit_notes"], json!([]));

    // By its UUID as by its id; an id of nothing is not found.
    let uuid = InvoiceId::from_str(&id).unwrap().as_uuid().to_string();
    let by_uuid = f.get(&format!("/api/v2/invoices/{uuid}")).await;
    assert_eq!(by_uuid.status, StatusCode::OK, "{:#}", by_uuid.body);
    assert_eq!(by_uuid.body["id"], id.as_str());
    let missing = f.get("/api/v2/invoices/not-an-invoice").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body["code"], "not_found");

    // Listed by filter, without what a single one reads.
    let spotify = f.get("/api/v2/invoices?customer=ext:spotify").await;
    assert_eq!(spotify.status, StatusCode::OK, "{:#}", spotify.body);
    assert!(ids_of(&spotify).contains(&id));
    assert!(spotify.body["data"][0].get("lines").is_none());
    let uber = f.get("/api/v2/invoices?customer=ext:uber").await;
    assert!(!ids_of(&uber).contains(&id));
    let by_subscription = f
        .get(&format!("/api/v2/invoices?subscription={subscription}"))
        .await;
    assert!(ids_of(&by_subscription).contains(&id));
    let others = f.get("/api/v2/invoices?subscription=inv-uber").await;
    assert!(!ids_of(&others).contains(&id));
    let issued = f
        .get("/api/v2/invoices?status=finalized&currency=eur&issued_after=2000-01-01T00:00:00Z")
        .await;
    assert_eq!(issued.status, StatusCode::OK, "{:#}", issued.body);
    assert!(ids_of(&issued).contains(&id));
    for path in [
        "/api/v2/invoices?status=void",
        "/api/v2/invoices?currency=USD",
        "/api/v2/invoices?payment_status=paid",
        "/api/v2/invoices?updated_after=2999-01-01T00:00:00Z",
    ] {
        let none = f.get(path).await;
        assert_eq!(none.status, StatusCode::OK, "{path}: {:#}", none.body);
        assert!(!ids_of(&none).contains(&id), "{path}");
    }
    let unknown = f.get("/api/v2/invoices?customer=ext:nobody").await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.body["errors"][0]["path"], "customer");
    let not_a_moment = f.get("/api/v2/invoices?updated_after=yesterday").await;
    assert_eq!(not_a_moment.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(not_a_moment.body["errors"][0]["path"], "updated_after");

    // Several statuses, the parameter repeated; created before or after a moment, exclusive.
    let either = f
        .get("/api/v2/invoices?customer=ext:spotify&status=void&status=finalized&limit=100")
        .await;
    assert_eq!(either.status, StatusCode::OK, "{:#}", either.body);
    assert!(ids_of(&either).contains(&id));
    let neither = f
        .get("/api/v2/invoices?customer=ext:spotify&status=void&status=draft&limit=100")
        .await;
    assert!(!ids_of(&neither).contains(&id));
    let unpaid_or_paid = f
        .get("/api/v2/invoices?customer=ext:spotify&payment_status=paid&payment_status=unpaid&limit=100")
        .await;
    assert_eq!(
        unpaid_or_paid.status,
        StatusCode::OK,
        "{:#}",
        unpaid_or_paid.body
    );
    assert!(ids_of(&unpaid_or_paid).contains(&id));
    let created_at = &invoice["created_at"];
    let since = moment_from(created_at, -1);
    let until = moment_from(created_at, 1);
    let created_after = f
        .get(&format!(
            "/api/v2/invoices?customer=ext:spotify&created_after={since}&limit=100"
        ))
        .await;
    assert_eq!(
        created_after.status,
        StatusCode::OK,
        "{:#}",
        created_after.body
    );
    assert!(ids_of(&created_after).contains(&id));
    let created_before = f
        .get(&format!(
            "/api/v2/invoices?customer=ext:spotify&created_before={since}&limit=100"
        ))
        .await;
    assert!(!ids_of(&created_before).contains(&id));
    let within = f
        .get(&format!(
            "/api/v2/invoices?customer=ext:spotify&created_after={since}&created_before={until}&limit=100"
        ))
        .await;
    assert!(ids_of(&within).contains(&id));
    let after_it = f
        .get(&format!(
            "/api/v2/invoices?customer=ext:spotify&created_after={until}&limit=100"
        ))
        .await;
    assert!(!ids_of(&after_it).contains(&id));
    for refused in [
        "/api/v2/invoices?status=finalized&status=paid",
        "/api/v2/invoices?created_after=yesterday",
        "/api/v2/invoices?created_before=2026-13-01T00:00:00Z",
    ] {
        let answer = f.get(refused).await;
        assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        let parameter = refused.split(['?', '=']).nth(1).unwrap();
        assert_eq!(answer.body["errors"][0]["path"], parameter, "{refused}");
    }

    // Page by page, each once.
    let mut seen: Vec<String> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(cursor) => format!("/api/v2/invoices?limit=1&after={cursor}"),
            None => "/api/v2/invoices?limit=1".to_owned(),
        };
        let page = f.get(&path).await;
        assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
        let ids = ids_of(&page);
        assert!(ids.len() <= 1);
        seen.extend(ids);
        if page.body["has_more"] == false {
            assert!(page.body["next_cursor"].is_null());
            break;
        }
        after = Some(page.body["next_cursor"].as_str().unwrap().to_owned());
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no invoice twice: {seen:?}");
    assert!(seen.contains(&id), "{seen:?}");
    let uber_invoice = ids_of(&others);
    assert!(
        uber_invoice.iter().all(|other| seen.contains(other)),
        "{seen:?} {uber_invoice:?} of {other}"
    );

    // Its files: served once generated.
    let base = format!("/api/v2/invoices/{id}");
    file(&f, &base, &invoice, "pdf", "application/pdf").await;
    file(&f, &base, &invoice, "xml", "application/xml").await;

    // Custom properties, merged.
    let patched = patch(&f, &base, json!({"custom_properties": {}})).await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["id"], id.as_str());

    // A payment is recorded with a key only; marked paid once whatever the retries.
    let unkeyed = f
        .post(&format!("{base}/payments"), json!({"amount": 100}))
        .await;
    assert_eq!(unkeyed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed.body["errors"][0]["path"], "Idempotency-Key");
    let paid = f
        .post_once(
            &format!("{base}/mark-paid"),
            "wire-spotify",
            json!({"reference": "wire-1"}),
        )
        .await;
    assert_eq!(paid.status, StatusCode::OK, "{:#}", paid.body);
    assert_eq!(paid.body["payment_status"], "paid");
    assert_eq!(paid.body["amount_due"], 0);
    assert_eq!(paid.body["amount_paid"], invoice["amount_due"]);
    assert!(paid.body["paid_at"].is_string());
    let payments = paid.body["payments"].as_array().unwrap();
    assert_eq!(payments.len(), 1);
    assert!(payments[0]["id"].as_str().unwrap().starts_with("pay_"));
    assert_eq!(payments[0]["type"], "payment");
    assert_eq!(payments[0]["status"], "settled");
    assert_eq!(payments[0]["reference"], "wire-1");
    let replayed = f
        .post_once(
            &format!("{base}/mark-paid"),
            "wire-spotify",
            json!({"reference": "wire-1"}),
        )
        .await;
    assert_eq!(replayed.status, StatusCode::OK);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    let read = f.get(&base).await;
    assert_eq!(read.body["payments"].as_array().unwrap().len(), 1);
    let listed_paid = f.get("/api/v2/invoices?payment_status=paid").await;
    assert!(ids_of(&listed_paid).contains(&id));

    // Reversed: due again.
    let reversed = f
        .post_once(
            &format!(
                "{base}/payments/{}/reverse",
                payments[0]["id"].as_str().unwrap()
            ),
            "wire-spotify-returned",
            json!({"kind": "returned", "reason": "bank return"}),
        )
        .await;
    assert_eq!(reversed.status, StatusCode::OK, "{:#}", reversed.body);
    assert_eq!(reversed.body["amount_due"], invoice["amount_due"]);
    let returned = reversed.body["payments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|payment| payment["id"] == payments[0]["id"])
        .unwrap();
    assert_eq!(returned["reversal_kind"], "returned");

    // Written off.
    let written_off = f
        .post(&format!("{base}/mark-uncollectible"), json!({}))
        .await;
    assert_eq!(written_off.status, StatusCode::OK, "{:#}", written_off.body);
    assert_eq!(written_off.body["status"], "uncollectible");
    assert!(written_off.body["marked_uncollectible_at"].is_string());
}

/// Rules: ADR-0013 §10, §11
#[tokio::test]
async fn test_rest_v2_change_documents_and_credit_notes() {
    let f = fixture().await;
    seed(&f).await;
    let subscription = subscribe(&f, "ext:comodo", "notes-comodo").await;
    let invoice = issued_invoice(&f, "notes-comodo").await;
    let id = invoice["id"].as_str().unwrap().to_owned();

    // What a change issues is read here.
    let added = f
        .post_once(
            "/api/v2/subscriptions/notes-comodo/quantities/users",
            "notes-seat",
            json!({"delta": "+1"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{:#}", added.body);
    for issued in added.body["invoices"].as_array().unwrap() {
        let number = &issued["number"];
        let issued = issued["id"].as_str().unwrap();
        let read = f.get(&format!("/api/v2/invoices/{issued}")).await;
        assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
        assert_eq!(read.body["subscription"]["id"], subscription.as_str());
        assert_eq!(&read.body["number"], number);
    }
    for note in added.body["credit_notes"].as_array().unwrap() {
        let note = note["id"].as_str().unwrap();
        let read = f.get(&format!("/api/v2/credit-notes/{note}")).await;
        assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    }

    // A credit note against the first invoice, issued by an operator.
    let paid = f
        .post(&format!("/api/v2/invoices/{id}/mark-paid"), json!({}))
        .await;
    assert_eq!(paid.status, StatusCode::OK, "{:#}", paid.body);
    let stored = f
        .setup
        .store
        .get_invoice_by_id(TENANT_ID, InvoiceId::from_str(&id).unwrap())
        .await
        .unwrap();
    let credited = stored
        .line_items
        .iter()
        .find(|line| line.amount_subtotal > 0)
        .unwrap();
    let note = f
        .setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: stored.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: credited.local_id.clone(),
                    amount_cents: credited.amount_subtotal,
                }],
                reason: Some("goodwill".to_owned()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .unwrap();
    let note_id = note.id.to_string();

    let read = f.get(&format!("/api/v2/credit-notes/{note_id}")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["id"], note_id.as_str());
    assert_eq!(read.body["invoice"]["id"], id.as_str());
    assert!(read.body["invoice"]["number"].is_string());
    assert_eq!(read.body["status"], "finalized");
    assert_eq!(read.body["type"], "credit_to_balance");
    assert_eq!(read.body["origin"], "manual");
    assert_eq!(read.body["reason"], "goodwill");
    assert_eq!(read.body["customer"]["alias"], "comodo");
    assert!(read.body["total"].as_i64().unwrap() < 0, "{:#}", read.body);
    let lines = read.body["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0]["amount"].as_i64().unwrap() < 0, "{:#}", lines[0]);

    // Linked from its invoice, listed by it.
    let invoice_after = f.get(&format!("/api/v2/invoices/{id}")).await;
    assert_eq!(
        invoice_after.body["credit_notes"][0]["id"],
        note_id.as_str()
    );
    assert_eq!(
        invoice_after.body["credit_notes"][0]["number"],
        read.body["number"]
    );
    let of_invoice = f.get(&format!("/api/v2/credit-notes?invoice={id}")).await;
    assert_eq!(of_invoice.status, StatusCode::OK, "{:#}", of_invoice.body);
    assert_eq!(ids_of(&of_invoice), vec![note_id.clone()]);
    assert!(of_invoice.body["data"][0].get("lines").is_none());
    let of_customer = f.get("/api/v2/credit-notes?customer=ext:comodo").await;
    assert!(ids_of(&of_customer).contains(&note_id));
    let of_subscription = f
        .get("/api/v2/credit-notes?subscription=notes-comodo&status=finalized")
        .await;
    assert!(ids_of(&of_subscription).contains(&note_id));
    let drafts = f.get("/api/v2/credit-notes?status=draft").await;
    assert!(!ids_of(&drafts).contains(&note_id));
    let drafts_or_issued = f
        .get("/api/v2/credit-notes?status=draft&status=finalized&limit=100")
        .await;
    assert_eq!(
        drafts_or_issued.status,
        StatusCode::OK,
        "{:#}",
        drafts_or_issued.body
    );
    assert!(ids_of(&drafts_or_issued).contains(&note_id));
    let since = moment_from(&read.body["created_at"], -1);
    let created_after = f
        .get(&format!(
            "/api/v2/credit-notes?created_after={since}&limit=100"
        ))
        .await;
    assert_eq!(
        created_after.status,
        StatusCode::OK,
        "{:#}",
        created_after.body
    );
    assert!(ids_of(&created_after).contains(&note_id));
    let created_before = f
        .get(&format!(
            "/api/v2/credit-notes?created_before={since}&limit=100"
        ))
        .await;
    assert!(!ids_of(&created_before).contains(&note_id));
    let elsewhere = f.get("/api/v2/credit-notes?customer=ext:spotify").await;
    assert!(!ids_of(&elsewhere).contains(&note_id));
    let first = f.get("/api/v2/credit-notes?limit=1").await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert!(ids_of(&first).len() <= 1);

    // Its files and custom properties.
    let base = format!("/api/v2/credit-notes/{note_id}");
    file(&f, &base, &read.body, "pdf", "application/pdf").await;
    file(&f, &base, &read.body, "xml", "application/xml").await;
    let patched = patch(&f, &base, json!({"custom_properties": {}})).await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["id"], note_id.as_str());
    let missing = f.get("/api/v2/credit-notes/crn_nothing").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    // A note of the stored list reads as the store wrote it.
    let stored_notes = f
        .setup
        .store
        .list_credit_notes_by_invoice_id(TENANT_ID, stored.id)
        .await
        .unwrap();
    assert_eq!(stored_notes.len(), 1);
}

/// Rules: ADR-0013 §12 (`updated_after`)
#[tokio::test]
async fn test_rest_v2_invoice_updated_at_moves_on_every_edit() {
    let f = fixture().await;
    seed(&f).await;
    let created = f
        .post_once(
            "/api/v2/subscriptions",
            "create-edited",
            json!({
                "customer": "ext:spotify",
                "plan": "studio",
                "option": "monthly",
                "external_id": "inv-edited",
                "quantities": {"users": "3"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false, "auto_finalize": false}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let page = f.get("/api/v2/invoices?subscription=inv-edited").await;
    let id = ids_of(&page)
        .first()
        .cloned()
        .expect("the subscription issued no invoice");
    let fixture = &f;
    let read = || {
        let path = format!("/api/v2/invoices/{id}");
        async move {
            let invoice = fixture.get(&path).await;
            assert_eq!(invoice.status, StatusCode::OK, "{:#}", invoice.body);
            invoice.body["updated_at"].as_str().unwrap().to_owned()
        }
    };
    let uuid = InvoiceId::from_str(&id).unwrap().as_uuid().to_string();
    let write = |sql: &'static str| {
        let uuid = uuid.clone();
        async move {
            use diesel_async::RunQueryDsl;
            diesel::sql_query(sql)
                .bind::<diesel::sql_types::Text, _>(uuid)
                .execute(&mut fixture.setup.store.get_conn().await.unwrap())
                .await
                .unwrap();
        }
    };
    let before = read().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // An edit no store path stamps (a memo written by hand, as an annex or the customer's details
    // of a draft would be): it counts.
    write("UPDATE invoice SET memo = 'edited by hand' WHERE id = $1::uuid").await;
    let edited = read().await;
    assert!(edited > before, "{edited} after {before}");
    let since = f
        .get(&format!(
            "/api/v2/invoices?subscription=inv-edited&updated_after={before}"
        ))
        .await;
    assert_eq!(since.status, StatusCode::OK, "{:#}", since.body);
    assert!(ids_of(&since).contains(&id), "{:#}", since.body);

    // The engine's bookkeeping does not, nor a refresh of the lines that moved none.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    write("UPDATE invoice SET next_payment_attempt_at = now() WHERE id = $1::uuid").await;
    assert_eq!(read().await, edited);
    write(
        "UPDATE invoice SET data_updated_at = clock_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = $1::uuid",
    )
    .await;
    assert_eq!(read().await, edited);
    // A millisecond on: times read at the millisecond, stored at the microsecond.
    let after_edit = (chrono::DateTime::parse_from_rfc3339(&edited).unwrap()
        + chrono::Duration::milliseconds(1))
    .to_utc()
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let since_edit = f
        .get(&format!(
            "/api/v2/invoices?subscription=inv-edited&updated_after={after_edit}"
        ))
        .await;
    assert_eq!(since_edit.status, StatusCode::OK, "{:#}", since_edit.body);
    assert!(!ids_of(&since_edit).contains(&id), "{:#}", since_edit.body);
}

/// Two lines written by hand: 2 × 150.00 taxed at 20% and 1 × 99.99 untaxed.
fn one_off(status: &str) -> Value {
    json!({
        "customer": "ext:spotify",
        "currency": "EUR",
        "memo": "Onboarding",
        "status": status,
        "lines": [
            {"name": "Onboarding", "quantity": "2", "unit_price": "150", "tax_rate": "0.2"},
            {"name": "Training", "quantity": "1", "unit_price": "99.99"}
        ]
    })
}

fn amount(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("not an amount: {value:#}"))
}

/// Rules: ADR-0013 §12 (a one-off invoice written by hand; a draft's lines edited, only a draft's)
#[tokio::test]
async fn test_rest_v2_one_off_invoice_created_and_edited() {
    let f = fixture().await;
    seed(&f).await;

    // Never without a key, refused at its pointer otherwise.
    let unkeyed = f.post("/api/v2/invoices", one_off("draft")).await;
    assert_eq!(unkeyed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed.body["errors"][0]["path"], "Idempotency-Key");
    for (body, path) in [
        (
            json!({"customer": "ext:nobody", "lines": [{"name": "A", "quantity": "1", "unit_price": "1"}]}),
            "/customer",
        ),
        (
            json!({"customer": "ext:spotify", "lines": [{"name": "A", "quantity": "1"}]}),
            "/lines/0/unit_price",
        ),
        (
            json!({"customer": "ext:spotify", "lines": [{"id": "x", "name": "A", "quantity": "1", "unit_price": "1"}]}),
            "/lines/0/id",
        ),
        (
            json!({"customer": "ext:spotify", "lines": [{"name": "A", "quantity": "1", "unit_price": "1", "tax_rate": "20"}]}),
            "/lines/0/tax_rate",
        ),
        (json!({"customer": "ext:spotify", "lines": []}), "/lines"),
    ] {
        let refused = f.post_once("/api/v2/invoices", path, body).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], path);
    }

    // A draft: 2 × 15000 = 30000 and its 20%, 6000; 9999 untaxed. 39999 + 6000 = 45999.
    let created = f
        .post_once("/api/v2/invoices", "one-off-draft", one_off("draft"))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let id = created.body["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("inv_"), "{id}");
    assert_eq!(created.body["status"], "draft");
    assert_eq!(created.body["type"], "one_off");
    assert_eq!(created.body["currency"], "EUR");
    assert_eq!(created.body["memo"], "Onboarding");
    assert_eq!(created.body["customer"]["alias"], "spotify");
    assert!(created.body.get("subscription").is_none());
    let lines = created.body["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{:#}", created.body);
    assert_eq!(
        (amount(&lines[0]["amount"]), amount(&lines[0]["tax"])),
        (30_000, 6_000)
    );
    assert_eq!(lines[0]["unit_price"], "150");
    assert_eq!(lines[0]["tax_rate"], "0.2");
    assert_eq!(
        (amount(&lines[1]["amount"]), amount(&lines[1]["tax"])),
        (9_999, 0)
    );
    assert_eq!(amount(&created.body["subtotal"]), 39_999);
    assert_eq!(amount(&created.body["tax"]), 6_000);
    assert_eq!(amount(&created.body["total"]), 45_999);
    assert_eq!(amount(&created.body["amount_due"]), 45_999);
    // Created once, whatever the retries.
    let replayed = f
        .post_once("/api/v2/invoices", "one-off-draft", one_off("draft"))
        .await;
    assert_eq!(replayed.status, StatusCode::CREATED);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replayed.body["id"], id.as_str());
    let listed = f.get("/api/v2/invoices?customer=ext:spotify").await;
    assert_eq!(
        ids_of(&listed).iter().filter(|other| **other == id).count(),
        1
    );

    // Edited: the training line kept at 3 units (3 × 9999 = 29997), the onboarding line removed,
    // 1 × 50.00 at 20% added (5000, 1000 of tax): 34997 + 1000 = 35997. The memo cleared.
    let base = format!("/api/v2/invoices/{id}");
    let training = lines[1]["id"].as_str().unwrap().to_owned();
    let invoice_date = created.body["invoice_date"].as_str().unwrap().to_owned();
    let due_date = (chrono::NaiveDate::parse_from_str(&invoice_date, "%Y-%m-%d").unwrap()
        + chrono::Duration::days(30))
    .to_string();
    let edited = patch(
        &f,
        &base,
        json!({
            "lines": [
                {"id": training, "quantity": "3"},
                {"name": "Support", "quantity": "1", "unit_price": "50", "tax_rate": "0.2"}
            ],
            "memo": null,
            "due_date": due_date
        }),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK, "{:#}", edited.body);
    let lines = edited.body["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{:#}", edited.body);
    assert_eq!(
        lines[0]["id"],
        training.as_str(),
        "the line kept keeps its id"
    );
    assert_eq!(lines[0]["name"], "Training");
    assert_eq!(
        (amount(&lines[0]["amount"]), amount(&lines[0]["tax"])),
        (29_997, 0)
    );
    assert_eq!(lines[1]["name"], "Support");
    assert_eq!(
        (amount(&lines[1]["amount"]), amount(&lines[1]["tax"])),
        (5_000, 1_000)
    );
    assert_eq!(amount(&edited.body["subtotal"]), 34_997);
    assert_eq!(amount(&edited.body["total"]), 35_997);
    assert!(edited.body.get("memo").is_none(), "{:#}", edited.body);
    assert_eq!(edited.body["due_date"], due_date.as_str());

    // A line of nothing is refused at its pointer.
    let unknown = patch(&f, &base, json!({"lines": [{"id": "not-a-line"}]})).await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.body["errors"][0]["path"], "/lines/0/id");

    // Issued, its lines no longer change; its custom properties still do.
    let finalized = f.post(&format!("{base}/finalize"), json!({})).await;
    assert_eq!(finalized.status, StatusCode::OK, "{:#}", finalized.body);
    assert_eq!(amount(&finalized.body["total"]), 35_997);
    let too_late = patch(
        &f,
        &base,
        json!({"lines": [{"id": training, "quantity": "1"}]}),
    )
    .await;
    assert_eq!(too_late.status, StatusCode::CONFLICT, "{:#}", too_late.body);
    assert_eq!(too_late.body["code"], "conflict");
    let memo_too_late = patch(&f, &base, json!({"memo": "late"})).await;
    assert_eq!(memo_too_late.status, StatusCode::CONFLICT);
    let properties = patch(&f, &base, json!({"custom_properties": {}})).await;
    assert_eq!(properties.status, StatusCode::OK, "{:#}", properties.body);

    // Issued at once when asked.
    let issued = f
        .post_once("/api/v2/invoices", "one-off-issued", one_off("finalized"))
        .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{:#}", issued.body);
    assert_eq!(issued.body["status"], "finalized");
    assert!(issued.body["number"].is_string(), "{:#}", issued.body);
    assert_eq!(amount(&issued.body["total"]), 45_999);
}

/// Rules: ADR-0013 §12 (a credit note issued through v2: by quantity, by amount, refunded, left a
/// draft; never on a draft)
#[tokio::test]
async fn test_rest_v2_credit_note_issued() {
    let f = fixture().await;
    seed(&f).await;
    // 2 × 150.00 at 20% (30000 + 6000) and 99.99 (9999): 45999, issued and paid.
    let invoice = f
        .post_once("/api/v2/invoices", "note-invoice", one_off("finalized"))
        .await;
    assert_eq!(invoice.status, StatusCode::CREATED, "{:#}", invoice.body);
    let id = invoice.body["id"].as_str().unwrap().to_owned();
    let paid = f
        .post(&format!("/api/v2/invoices/{id}/mark-paid"), json!({}))
        .await;
    assert_eq!(paid.status, StatusCode::OK, "{:#}", paid.body);
    let onboarding = invoice.body["lines"][0]["id"].as_str().unwrap().to_owned();
    let training = invoice.body["lines"][1]["id"].as_str().unwrap().to_owned();

    // Never without a key; a line not on the invoice, an amount of nothing, two ways at once:
    // refused at their pointer.
    let one_unit = json!({
        "invoice": id,
        "lines": [{"line": onboarding, "quantity": "1"}],
        "reason": "One session cancelled"
    });
    let unkeyed = f.post("/api/v2/credit-notes", one_unit.clone()).await;
    assert_eq!(unkeyed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unkeyed.body["code"], "idempotency_key_required");
    assert_eq!(unkeyed.body["errors"][0]["path"], "Idempotency-Key");
    for (lines, path) in [
        (
            json!([{"line": "not-a-line", "quantity": "1"}]),
            "/lines/0/line",
        ),
        (
            json!([{"line": onboarding, "amount": 0}]),
            "/lines/0/amount",
        ),
        (
            json!([{"line": onboarding, "quantity": "1", "amount": 100}]),
            "/lines/0",
        ),
    ] {
        let refused = f
            .post_once(
                "/api/v2/credit-notes",
                &format!("note-refused-{path}"),
                json!({"invoice": id, "lines": lines}),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], path);
    }

    // One onboarding unit back to the balance: 15000, its tax reversed by the same share,
    // 6000 × 15000/30000 = 3000: 18000.
    let issued = f
        .post_once("/api/v2/credit-notes", "note-one-unit", one_unit.clone())
        .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{:#}", issued.body);
    let note = &issued.body;
    assert!(note["id"].as_str().unwrap().starts_with("crn_"), "{note:#}");
    assert_eq!(note["invoice"]["id"], id.as_str());
    assert_eq!(note["status"], "finalized");
    assert_eq!(note["type"], "credit_to_balance");
    assert_eq!(note["origin"], "manual");
    assert_eq!(note["reason"], "One session cancelled");
    assert_eq!(
        (
            amount(&note["subtotal"]),
            amount(&note["tax"]),
            amount(&note["total"])
        ),
        (-15_000, -3_000, -18_000)
    );
    assert_eq!(amount(&note["credited"]), 18_000);
    let lines = note["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "{note:#}");
    assert_eq!(amount(&lines[0]["amount"]), -15_000);
    let replayed = f
        .post_once("/api/v2/credit-notes", "note-one-unit", one_unit)
        .await;
    assert_eq!(replayed.status, StatusCode::CREATED);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replayed.body, issued.body);
    let of_invoice = f.get(&format!("/api/v2/credit-notes?invoice={id}")).await;
    assert_eq!(ids_of(&of_invoice).len(), 1, "credited once");

    // 50.00 of the training refunded as a correction: 5000, untaxed.
    let refund = f
        .post_once(
            "/api/v2/credit-notes",
            "note-refund",
            json!({
                "invoice": id,
                "lines": [{"line": training, "amount": 5000}],
                "type": "refund",
                "correction": true
            }),
        )
        .await;
    assert_eq!(refund.status, StatusCode::CREATED, "{:#}", refund.body);
    assert_eq!(refund.body["type"], "refund");
    assert_eq!(refund.body["origin"], "correction");
    assert_eq!(amount(&refund.body["total"]), -5_000);

    // Left a draft when asked: the onboarding unit left, 15000.
    let draft = f
        .post_once(
            "/api/v2/credit-notes",
            "note-draft",
            json!({"invoice": id, "lines": [{"line": onboarding, "quantity": "1"}], "status": "draft"}),
        )
        .await;
    assert_eq!(draft.status, StatusCode::CREATED, "{:#}", draft.body);
    assert_eq!(draft.body["status"], "draft");
    assert_eq!(amount(&draft.body["subtotal"]), -15_000);

    // A draft invoice is edited, never credited.
    let draft_invoice = f
        .post_once("/api/v2/invoices", "note-draft-invoice", one_off("draft"))
        .await;
    assert_eq!(draft_invoice.status, StatusCode::CREATED);
    let on_a_draft = f
        .post_once(
            "/api/v2/credit-notes",
            "note-on-a-draft",
            json!({"invoice": draft_invoice.body["id"]}),
        )
        .await;
    assert_eq!(
        on_a_draft.status,
        StatusCode::CONFLICT,
        "{:#}",
        on_a_draft.body
    );
    assert_eq!(on_a_draft.body["code"], "conflict");
}

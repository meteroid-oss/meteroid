//! A credit unit wallet through the public REST API: grants with their cost, balances, ledger
//! and grants all in the unit's minor units.
use crate::data::ids::TENANT_ID;
use crate::test_rest_wallets::{Fixture, customer_path, fixture};
use meteroid_store::domain::credit_units::CreditUnitNew;
use meteroid_store::repositories::CreditUnitsInterface;
use reqwest::StatusCode;
use serde_json::{Value, json};

const UNIT: &str = "CREDITS";

async fn with_unit(precision: u8) -> Fixture {
    let fixture = fixture().await;
    fixture
        .setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, UNIT, "Credit", "Credits", None, precision).unwrap(),
        )
        .await
        .expect("credit unit");
    fixture
}

fn unit_wallet(wallets: &Value) -> &Value {
    wallets["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|wallet| wallet["credit_unit"] == UNIT)
        .unwrap_or_else(|| panic!("a {UNIT} wallet: {wallets}"))
}

#[tokio::test]
async fn rest_unit_grants_state_their_cost_and_read_in_the_unit() {
    let fixture = with_unit(3).await;
    let grants = customer_path("uber", "credit-grants");

    let (status, body) = fixture
        .post(
            &grants,
            Some("paid-no-cost"),
            json!({
                "type": "PAID", "amount": 5_000_000, "credit_unit": UNIT,
                "reference": "INV-2026-0042",
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "paid credits need their cost: {body}"
    );

    let (status, paid) = fixture
        .post(
            &grants,
            Some("paid"),
            json!({
                "type": "PAID", "amount": 5_000_000, "credit_unit": "credits",
                "cost": {"amount": 4_000, "currency": "EUR"}, "reference": "INV-2026-0042",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{paid}");
    assert!(paid["currency"].is_null(), "{paid}");
    assert_eq!(paid["credit_unit"], UNIT);
    assert_eq!(paid["credit_unit_precision"], 3);
    assert_eq!(paid["amount"], 5_000_000);
    assert_eq!(paid["cost"], json!({"amount": 4_000, "currency": "EUR"}));
    assert_eq!(paid["cost_basis"], "0.008", "EUR 0.008 per credit");

    let (status, given) = fixture
        .post(
            &grants,
            Some("given"),
            json!({"type": "PROMOTIONAL", "amount": 1_000, "credit_unit": UNIT}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{given}");
    assert_eq!(
        given["cost"],
        json!({"amount": 0, "currency": "EUR"}),
        "given credits cost nothing, in the customer's currency"
    );

    for (key, body) in [
        (
            "both",
            json!({"type": "PROMOTIONAL", "amount": 1, "credit_unit": UNIT, "currency": "EUR"}),
        ),
        (
            "usage-only",
            json!({"type": "PROMOTIONAL", "amount": 1, "credit_unit": UNIT, "applies_to": "USAGE"}),
        ),
        (
            "unknown",
            json!({"type": "PROMOTIONAL", "amount": 1, "credit_unit": "TOKENS"}),
        ),
    ] {
        let (status, body) = fixture.post(&grants, Some(key), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {body}");
    }

    let (_, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(
        wallets["data"][0]["currency"], "EUR",
        "the customer's currency stays first"
    );
    let wallet = unit_wallet(&wallets);
    assert!(wallet["currency"].is_null());
    assert_eq!(wallet["credit_unit_precision"], 3);
    assert_eq!(wallet["credit_on_account"], 5_001_000);

    let (status, ledger) = fixture
        .get(&format!(
            "{}?credit_unit={UNIT}",
            customer_path("uber", "credit-ledger")
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{ledger}");
    let entries = ledger["data"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry["credit_unit"] == UNIT
        && entry["credit_unit_precision"] == 3
        && entry["currency"].is_null()));
    assert_eq!(entries[0]["balance_after"], 5_001_000);

    let (_, listed) = fixture.get(&format!("{grants}?credit_unit={UNIT}")).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 2, "{listed}");
    let (_, eur) = fixture.get(&format!("{grants}?currency=EUR")).await;
    assert_eq!(eur["data"], json!([]), "no money credit was granted");
}

//! The `/api/v1/rate-cards` REST surface: a money card per 1M tokens, its rates in two
//! currencies set in one dated batch, a key removed from a later day, read back by id or code.
use crate::data::ids::METRIC_BANDWIDTH;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_grpc::meteroid::api;
use reqwest::StatusCode;
use serde_json::{Value, json};

struct Fixture {
    setup: MeteroidSetup,
    client: reqwest::Client,
    api_key: String,
}

impl Fixture {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.setup.config.rest_api_external_url)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> (StatusCode, Value) {
        let response = request.bearer_auth(&self.api_key).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        (
            status,
            serde_json::from_str(&body).unwrap_or(Value::String(body)),
        )
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(self.client.get(self.url(path))).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(self.client.post(self.url(path)).json(&body))
            .await
    }

    async fn patch(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(self.client.patch(self.url(path)).json(&body))
            .await
    }
}

async fn fixture() -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "rate-cards".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;
    Fixture {
        setup,
        client: http_client_tracing::client(),
        api_key,
    }
}

fn rates_of(card: &Value) -> Vec<(String, String, String)> {
    card["rates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rate| {
            let key: Vec<&str> = rate["key"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap())
                .collect();
            (
                key.join(","),
                rate["currency"].as_str().unwrap().to_owned(),
                rate["rate"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Rules: R-131
#[tokio::test]
async fn test_rest_rate_cards_create_set_rates_and_read_back() {
    let f = fixture().await;

    let (status, card) = f
        .post(
            "/api/v1/rate-cards",
            json!({
                "code": "llm",
                "name": "LLM tokens",
                "metric_id": METRIC_BANDWIDTH.to_string(),
                "dimensions": ["model", "token_type"],
                "denomination": "MONEY",
                "unit_size": "1000000"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{card:?}");
    assert_eq!(card["unit_size"], "1000000");
    assert_eq!(card["denomination"], "MONEY");
    assert_eq!(card["on_unmatched"], "HOLD");
    let id = card["id"].as_str().unwrap().to_owned();

    let (status, _) = f
        .post(
            "/api/v1/rate-cards",
            json!({
                "code": "llm",
                "name": "Again",
                "metric_id": METRIC_BANDWIDTH.to_string(),
                "dimensions": ["model"],
                "denomination": "MONEY"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "a code is unique");
    let (status, _) = f
        .post(
            "/api/v1/rate-cards",
            json!({
                "code": "credits",
                "name": "Credits",
                "metric_id": METRIC_BANDWIDTH.to_string(),
                "dimensions": ["model"],
                "denomination": "CREDIT_UNIT"
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a credit card names its unit"
    );

    let batch = json!({
        "effective_from": "2026-01-01",
        "rates": [
            {"key": ["anthropic/*", "input"], "currency": "EUR", "rate": "3"},
            {"key": ["anthropic/*", "input"], "currency": "USD", "rate": "3.50"},
            {"key": ["*", "*"], "currency": "EUR", "rate": "10"}
        ]
    });
    let (status, body) = f.patch("/api/v1/rate-cards/llm/rates", batch.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        (body["inserted"].as_u64(), body["unchanged"].as_u64()),
        (Some(3), Some(0))
    );
    let (_, body) = f
        .patch(&format!("/api/v1/rate-cards/{id}/rates"), batch)
        .await;
    assert_eq!(body["unchanged"], 3, "a repeated batch changes nothing");

    let (status, _) = f
        .patch(
            "/api/v1/rate-cards/llm/rates",
            json!({"rates": [{"key": ["x", "input"], "rate": "1"}]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a money card's rate has a currency"
    );

    let (status, body) = f
        .patch(
            "/api/v1/rate-cards/llm/rates",
            json!({
                "effective_from": "2099-01-01",
                "remove": [{"key": ["*", "*"], "currency": "EUR"}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["removed"], 1);

    let (status, card) = f.get("/api/v1/rate-cards/llm?at=2026-06-01").await;
    assert_eq!(status, StatusCode::OK, "{card:?}");
    assert_eq!(card["id"], id);
    assert_eq!(
        rates_of(&card),
        vec![
            ("*,*".to_owned(), "EUR".to_owned(), "10".to_owned()),
            (
                "anthropic/*,input".to_owned(),
                "EUR".to_owned(),
                "3".to_owned()
            ),
            (
                "anthropic/*,input".to_owned(),
                "USD".to_owned(),
                "3.5".to_owned()
            ),
        ]
    );
    assert_eq!(card["rates"][0]["any_other"], true);
    let scheduled = card["scheduled"].as_array().unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0]["rate"], Value::Null, "a removal");

    let (_, card) = f
        .get(&format!("/api/v1/rate-cards/{id}?at=2099-01-02"))
        .await;
    assert_eq!(rates_of(&card).len(), 2, "`*, *` removed in EUR from 2099");

    let (status, list) = f.get("/api/v1/rate-cards").await;
    assert_eq!(status, StatusCode::OK, "{list:?}");
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id)
    );
    let (status, _) = f.get("/api/v1/rate-cards/unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

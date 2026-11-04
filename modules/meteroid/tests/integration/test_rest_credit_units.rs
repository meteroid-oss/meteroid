//! S3: the `/api/v1/credit-units` REST surface (list, get, create, patch); its publication is
//! checked by `api_rest::openapi` tests.
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

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap();
        read(response).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .unwrap();
        read(response).await
    }

    async fn patch(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .client
            .patch(self.url(path))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .unwrap();
        read(response).await
    }
}

async fn read(response: reqwest::Response) -> (StatusCode, Value) {
    let status = response.status();
    let body = response.text().await.unwrap();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

async fn fixture() -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
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
            name: "credit-units".to_owned(),
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

#[tokio::test]
async fn test_rest_credit_units_list_get_create_patch() {
    let f = fixture().await;

    let (status, body) = f
        .post(
            "/api/v1/credit-units",
            json!({
                "code": "credits",
                "name": "Credit",
                "plural_name": "Credits",
                "symbol": "cr",
                "precision": 2
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    assert_eq!(body["code"], "CREDITS");
    assert_eq!(body["archived"], false);
    let id = body["id"].as_str().unwrap().to_owned();

    let (status, body) = f.get("/api/v1/credit-units").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == id)
    );

    let (status, body) = f.get(&format!("/api/v1/credit-units/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["id"], id);

    // code and precision rejected by validation at creation when too short / out of range
    let (status, _) = f
        .post(
            "/api/v1/credit-units",
            json!({"code": "USD", "name": "X", "plural_name": "Xs", "precision": 2}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // duplicate code
    let (status, _) = f
        .post(
            "/api/v1/credit-units",
            json!({"code": "CREDITS", "name": "X", "plural_name": "Xs", "precision": 2}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // partial update: name/plural only, code/precision unaffected
    let (status, body) = f
        .patch(
            &format!("/api/v1/credit-units/{id}"),
            json!({"name": "Token", "plural_name": "Tokens"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "Token");
    assert_eq!(body["code"], "CREDITS");
    assert_eq!(body["precision"], 2);

    // archive: blocked from the default list, still visible with include_archived
    let (status, body) = f
        .patch(
            &format!("/api/v1/credit-units/{id}"),
            json!({"archived": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["archived"], true);

    let (_, body) = f.get("/api/v1/credit-units").await;
    assert!(
        !body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == id)
    );

    let (_, body) = f.get("/api/v1/credit-units?include_archived=true").await;
    assert!(
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == id)
    );
}

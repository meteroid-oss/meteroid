//! Customer credit through the public REST API (customer-scoped by id or alias), the admin gRPC API
//! and the customer portal: reads, credit grants and top-ups, each write replayed by its
//! Idempotency-Key and refused when the key names another request.
use crate::data::ids::{
    CUST_SPOTIFY_ID, CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, INVOICING_ENTITY_ID, TENANT_ID,
};
use crate::data::payment::{run_customer_payment_methods_seed, run_mock_payment_provider_seed};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, CustomerId, WalletId};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::wallets::v1 as wallets;
use meteroid_grpc::meteroid::portal::wallets::v1 as portal;
use meteroid_grpc::meteroid::portal::wallets::v1::portal_wallets_service_client::PortalWalletsServiceClient;
use meteroid_store::jwt_claims::{PortalScope, ResourceAccess, generate_portal_token};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct Fixture {
    pub(crate) setup: MeteroidSetup,
    clients: AllClients,
    client: reqwest::Client,
    api_key: String,
}

impl Fixture {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.setup.config.rest_api_external_url)
    }

    pub(crate) async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap();
        read(response).await
    }

    pub(crate) async fn post(
        &self,
        path: &str,
        key: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.api_key)
            .json(&body);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        read(request.send().await.unwrap()).await
    }

    pub(crate) async fn patch(
        &self,
        path: &str,
        key: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .patch(self.url(path))
            .bearer_auth(&self.api_key)
            .json(&body);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        read(request.send().await.unwrap()).await
    }

    fn portal(&self, customer: CustomerId, scopes: Vec<PortalScope>) -> PortalClient {
        let token = generate_portal_token(
            &self.setup.config.jwt_secret,
            TENANT_ID,
            ResourceAccess::Customer(customer),
            Some(scopes),
            None,
            false,
        )
        .unwrap();
        PortalClient {
            client: PortalWalletsServiceClient::new(self.setup.channel.clone()),
            token,
        }
    }

    async fn set_portal_switches(&self, top_up: bool, auto_top_up: bool) {
        self.clients
            .invoicing_entities
            .clone()
            .update_invoicing_entity(api::invoicingentities::v1::UpdateInvoicingEntityRequest {
                id: INVOICING_ENTITY_ID.as_proto(),
                data: Some(api::invoicingentities::v1::InvoicingEntityData {
                    portal_top_up_enabled: Some(top_up),
                    portal_auto_top_up_enabled: Some(auto_top_up),
                    ..Default::default()
                }),
            })
            .await
            .unwrap();
    }
}

struct PortalClient {
    client: PortalWalletsServiceClient<tonic::transport::Channel>,
    token: String,
}

impl PortalClient {
    fn request<T>(&self, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request
            .metadata_mut()
            .insert("x-portal-token", self.token.parse().unwrap());
        request
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

pub(crate) async fn fixture() -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await;
    run_mock_payment_provider_seed(&setup.store.pool, false).await;
    run_customer_payment_methods_seed(&setup.store.pool).await;
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
            name: "wallets".to_owned(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;
    Fixture {
        setup,
        clients,
        client: http_client_tracing::client(),
        api_key,
    }
}

async fn invoice_wallet(setup: &MeteroidSetup, customer: CustomerId) -> WalletId {
    let currency = setup
        .store
        .find_customer_by_id(customer, TENANT_ID)
        .await
        .unwrap()
        .currency;
    setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer, &currency)
        .await
        .unwrap()
        .id
}

async fn drain(setup: &MeteroidSetup) {
    for _ in 0..4 {
        run_once_invoice_orchestration(
            Arc::new(setup.store.clone()),
            Arc::new(setup.services.clone()),
        )
        .await;
    }
}

pub(crate) fn customer_path(customer: &str, resource: &str) -> String {
    format!("/api/v1/customers/{customer}/{resource}")
}

#[tokio::test]
async fn rest_paid_grant_below_face_returns_its_bonus_grant() {
    let fixture = fixture().await;
    let grants = customer_path("uber", "credit-grants");
    let request = json!({
        "type": "PAID", "amount": 55_000,
        "cost": {"amount": 50_000, "currency": "EUR"}, "reference": "RCPT-0042",
    });

    let (status, paid) = fixture
        .post(&grants, Some("paid-bonus"), request.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{paid}");
    assert_eq!(
        (paid["type"].clone(), paid["amount"].clone()),
        (json!("PAID"), json!(50_000))
    );
    let bonus_id = paid["bonus_grant_id"].clone();
    assert!(bonus_id.is_string(), "{paid}");

    let (_, replay) = fixture.post(&grants, Some("paid-bonus"), request).await;
    assert_eq!(replay["bonus_grant_id"], bonus_id, "{replay}");

    let (_, listed) = fixture.get(&grants).await;
    let bonus = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|grant| grant["id"] == bonus_id)
        .unwrap_or_else(|| panic!("the bonus grant is listed: {listed}"));
    assert_eq!(bonus["type"], "PROMOTIONAL");
    assert_eq!(bonus["amount"], 5_000);
    assert_eq!(bonus["reason"], "Bonus credit");
    assert!(bonus.get("bonus_grant_id").is_none(), "{bonus}");

    let (status, given) = fixture
        .post(
            &grants,
            Some("given-no-bonus"),
            json!({"type": "PROMOTIONAL", "amount": 1_000}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{given}");
    assert!(given.get("bonus_grant_id").is_none(), "{given}");
}

/// French entities keep a supporting document for every accounting entry: paid credit recorded
/// without a Meteroid invoice names the invoice or receipt of its payment.
#[tokio::test]
async fn rest_paid_grant_of_a_french_entity_needs_a_payment_reference() {
    let fixture = fixture().await;
    let grants = customer_path("uber", "credit-grants");
    let paid = json!({"type": "PAID", "amount": 10_000});

    let (status, refused) = fixture
        .post(&grants, Some("fr-no-reference"), paid.clone())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused.to_string().contains("invoice or receipt"),
        "{refused}"
    );

    let mut referenced = paid.clone();
    referenced["reference"] = json!("RCPT-2026-0117");
    let (status, granted) = fixture
        .post(&grants, Some("fr-reference"), referenced)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{granted}");
    assert_eq!(granted["reference"], "RCPT-2026-0117");

    let (status, given) = fixture
        .post(
            &grants,
            Some("fr-promotional"),
            json!({"type": "PROMOTIONAL", "amount": 1_000}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "nothing was paid: {given}");
    let (status, free) = fixture
        .post(
            &grants,
            Some("fr-no-cost"),
            json!({"type": "PAID", "amount": 1_000, "cost": {"amount": 0, "currency": "EUR"}}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "nothing was paid: {free}");

    let mut conn = fixture.setup.store.get_conn().await.unwrap();
    diesel_async::RunQueryDsl::execute(
        diesel::sql_query("UPDATE invoicing_entity SET country = 'US' WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(INVOICING_ENTITY_ID.as_uuid()),
        &mut conn,
    )
    .await
    .unwrap();
    let (status, elsewhere) = fixture.post(&grants, Some("us-no-reference"), paid).await;
    assert_eq!(status, StatusCode::CREATED, "{elsewhere}");
}

#[tokio::test]
async fn rest_credit_grants_by_id_or_alias_replay_by_idempotency_key() {
    let fixture = fixture().await;
    let by_alias = customer_path("uber", "credit-grants");
    let by_id = customer_path(&CUST_UBER_ID.as_base62(), "credit-grants");
    let promotion = json!({
        "type": "PROMOTIONAL", "amount": 2_500,
        "reason": "Launch promotion", "reference": "PROMO-7",
    });

    let (status, body) = fixture.post(&by_alias, None, promotion.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, first) = fixture
        .post(&by_alias, Some("grant-1"), promotion.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["type"], "PROMOTIONAL");
    assert_eq!(first["status"], "ACTIVE");
    assert_eq!(first["source"], "MANUAL");
    assert_eq!(first["currency"], "EUR");
    assert_eq!(first["applies_to"], "ALL");
    assert_eq!(first["amount"], 2_500);
    assert_eq!(first["remaining"], 2_500);
    assert_eq!(first["reference"], "PROMO-7");

    // The alias and the id name the same customer, so the retry replays the grant.
    let (status, replay) = fixture
        .post(&by_id, Some("grant-1"), promotion.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{replay}");
    assert_eq!(replay["id"], first["id"]);

    let mut changed = promotion.clone();
    changed["amount"] = json!(9_000);
    let (status, conflict) = fixture
        .post(&by_alias, Some("grant-1"), changed.clone())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    // A request refused before it granted anything leaves its key free for the corrected one.
    let mut past = promotion.clone();
    past["expires_at"] = json!("2001-01-01T00:00:00Z");
    let (status, refused) = fixture.post(&by_alias, Some("grant-2"), past).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, corrected) = fixture.post(&by_alias, Some("grant-2"), changed).await;
    assert_eq!(status, StatusCode::CREATED, "{corrected}");
    assert_eq!(corrected["amount"], 9_000);

    // PAID credit records a payment made elsewhere: nothing is invoiced.
    let (status, paid) = fixture
        .post(
            &by_alias,
            Some("grant-3"),
            json!({
                "type": "PAID", "amount": 1_000, "applies_to": "USAGE", "priority": 5,
                "reference": "INV-2026-0042",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{paid}");
    assert_eq!(paid["type"], "PAID");
    assert_eq!(paid["applies_to"], "USAGE");
    assert_eq!(paid["priority"], 5);
    assert!(paid["invoice_id"].is_null());

    let (status, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(status, StatusCode::OK, "{wallets}");
    assert_eq!(wallets["data"].as_array().unwrap().len(), 1);
    let eur = &wallets["data"][0];
    assert_eq!(eur["currency"], "EUR");
    assert_eq!(eur["credit_on_account"], 12_500);
    assert_eq!(eur["credit_on_account_usage_only"], 1_000);
    assert_eq!(eur["upcoming_credit"], 0);
    assert!(eur["auto_top_up"].is_null());
    assert_eq!(eur["low_balance_thresholds"], json!([]));
    assert_eq!(eur["usage_at_zero"], "FOLLOW_PLAN");
    assert!(eur.get("id").is_none());

    let (status, page) = fixture.get(&format!("{by_id}?per_page=2")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination_meta"]["total_items"], 3);
    assert_eq!(page["pagination_meta"]["total_pages"], 2);
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert_eq!(page["data"][0]["id"], paid["id"]);
    let (_, active) = fixture.get(&format!("{by_alias}?status=ACTIVE")).await;
    assert_eq!(active["pagination_meta"]["total_items"], 3, "{active}");
    let (_, expired) = fixture.get(&format!("{by_alias}?status=EXPIRED")).await;
    assert_eq!(expired["pagination_meta"]["total_items"], 0, "{expired}");
    let (_, dollars) = fixture.get(&format!("{by_alias}?currency=USD")).await;
    assert_eq!(dollars["pagination_meta"]["total_items"], 0, "{dollars}");

    let (status, ledger) = fixture
        .get(&format!(
            "{}?per_page=1",
            customer_path("uber", "credit-ledger")
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{ledger}");
    assert_eq!(ledger["pagination_meta"]["total_items"], 3);
    let latest = &ledger["data"][0];
    assert_eq!(latest["type"], "CREDIT");
    assert_eq!(latest["amount"], 1_000);
    assert_eq!(latest["applies_to"], "USAGE");
    assert_eq!(latest["currency"], "EUR");
    assert_eq!(latest["balance_after"], 12_500);
    assert_eq!(latest["sequence"], 3);
    assert_eq!(latest["credit_grant_id"], paid["id"]);
    let (_, empty) = fixture
        .get(&format!(
            "{}?currency=USD",
            customer_path("uber", "credit-ledger")
        ))
        .await;
    assert_eq!(empty["pagination_meta"]["total_items"], 0, "{empty}");

    for resource in ["wallets", "credit-grants", "credit-ledger"] {
        let (status, _) = fixture.get(&customer_path("nobody", resource)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{resource}");
        let (status, _) = fixture
            .get(&customer_path(&CustomerId::new().as_base62(), resource))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{resource}");
    }
    let (status, _) = fixture
        .post(
            &customer_path("nobody", "credit-grants"),
            Some("grant-4"),
            promotion,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_top_ups_charge_once_per_key() {
    let fixture = fixture().await;
    let by_id = customer_path(&CUST_UBER_ID.as_base62(), "top-ups");
    let by_alias = customer_path("uber", "top-ups");
    // No payment method given: the customer's default one is charged.
    let charge = json!({"amount": 5_000});

    let (status, missing_key) = fixture.post(&by_id, None, charge.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{missing_key}");

    let (status, first) = fixture.post(&by_id, Some("top-up-1"), charge.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["status"], "SUCCEEDED");
    assert_eq!(first["amount"], 5_000);
    assert_eq!(first["charged_amount"], 5_000);
    assert_eq!(first["currency"], "EUR");
    assert!(first["checkout_url"].is_null());
    drain(&fixture.setup).await;

    let (status, replay) = fixture
        .post(&by_alias, Some("top-up-1"), charge.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{replay}");
    assert_eq!(
        replay["payment_transaction_id"],
        first["payment_transaction_id"]
    );
    assert_eq!(replay["invoice_id"], first["invoice_id"]);

    let (status, conflict) = fixture
        .post(&by_alias, Some("top-up-1"), json!({"amount": 7_000}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    let (_, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(wallets["data"][0]["credit_on_account"], 5_000, "{wallets}");
    let (_, grants) = fixture.get(&customer_path("uber", "credit-grants")).await;
    assert_eq!(grants["data"][0]["type"], "PAID", "{grants}");
    assert_eq!(grants["data"][0]["source"], "PURCHASE");
    assert_eq!(grants["data"][0]["applies_to"], "ALL");
    assert_eq!(grants["data"][0]["invoice_id"], first["invoice_id"]);

    let (status, explicit) = fixture
        .post(
            &by_alias,
            Some("top-up-2"),
            json!({"amount": 2_000, "payment_method_id": CUST_UBER_PAYMENT_METHOD_ID.as_base62()}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{explicit}");
    assert_eq!(explicit["status"], "SUCCEEDED");

    let (status, refused) = fixture
        .post(
            &by_alias,
            Some("top-up-3"),
            json!({"amount": 2_000, "collection": "SEND_INVOICE",
                   "payment_method_id": CUST_UBER_PAYMENT_METHOD_ID.as_base62()}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // Comodo has no payment method to charge.
    let (status, no_method) = fixture
        .post(
            &customer_path("comodo", "top-ups"),
            Some("top-up-4"),
            charge,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{no_method}");
    assert_eq!(no_method["status"], "REQUIRES_CUSTOMER_ACTION");
    assert!(no_method["invoice_id"].is_null());
    assert!(
        no_method["checkout_url"]
            .as_str()
            .unwrap()
            .contains("/checkout?token=")
    );
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer::dsl as c;
        let mut conn = fixture.setup.store.get_conn().await.unwrap();
        diesel::update(c::customer.filter(c::id.eq(CUST_UBER_ID)))
            .set(
                c::current_payment_method_id
                    .eq(None::<common_domain::ids::CustomerPaymentMethodId>),
            )
            .execute(&mut conn)
            .await
            .unwrap();
    }
    let (status, replay_without_card) = fixture
        .post(&by_alias, Some("top-up-1"), json!({"amount": 5_000}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{replay_without_card}");
    assert_eq!(replay_without_card["status"], "SUCCEEDED");
    assert_eq!(
        replay_without_card["payment_transaction_id"],
        first["payment_transaction_id"]
    );
    assert_eq!(replay_without_card["invoice_id"], first["invoice_id"]);
}

/// Old code rejected `expires_at` for the CHARGE collection and the charge-now path never carried
/// an expiry to the grant it funded, so a top-up's purchased credit could never expire.
#[tokio::test]
async fn rest_top_up_charge_carries_its_expiry_to_the_grant() {
    let fixture = fixture().await;
    let by_alias = customer_path("uber", "top-ups");
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();

    let (status, past) = fixture
        .post(
            &by_alias,
            Some("charge-expiry-past"),
            json!({"amount": 2_000, "expires_at": "2001-01-01T00:00:00Z"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{past}");

    let (status, charged) = fixture
        .post(
            &by_alias,
            Some("charge-expiry-1"),
            json!({"amount": 2_000, "expires_at": expires_at}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{charged}");
    assert_eq!(charged["status"], "SUCCEEDED");
    drain(&fixture.setup).await;

    let (_, grants) = fixture.get(&customer_path("uber", "credit-grants")).await;
    assert_eq!(grants["data"][0]["amount"], 2_000, "{grants}");
    let actual =
        chrono::DateTime::parse_from_rfc3339(grants["data"][0]["expires_at"].as_str().unwrap())
            .unwrap();
    assert_eq!(
        actual.timestamp_millis(),
        chrono::DateTime::parse_from_rfc3339(&expires_at)
            .unwrap()
            .timestamp_millis()
    );
}

#[tokio::test]
async fn rest_invoiced_top_up_is_issued_once_per_key() {
    let fixture = fixture().await;
    let top_ups = customer_path("uber", "top-ups");
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(365)).to_rfc3339();
    let commit = json!({"amount": 100_000, "collection": "SEND_INVOICE", "expires_at": expires_at});

    let (status, issued) = fixture
        .post(&top_ups, Some("commit-1"), commit.clone())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    assert_eq!(issued["status"], "AWAITING_PAYMENT");
    assert_eq!(issued["amount"], 100_000);
    assert!(issued["charged_amount"].is_null());
    assert!(issued["payment_transaction_id"].is_null());
    assert!(
        issued["checkout_url"]
            .as_str()
            .is_some_and(|url| url.contains("/portal/invoice-payment?token="))
    );

    let (status, replay) = fixture.post(&top_ups, Some("commit-1"), commit).await;
    assert_eq!(status, StatusCode::CREATED, "{replay}");
    assert_eq!(replay["invoice_id"], issued["invoice_id"]);

    let (_, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(wallets["data"][0]["credit_on_account"], 0, "{wallets}");
    let (_, grants) = fixture.get(&customer_path("uber", "credit-grants")).await;
    assert_eq!(grants["pagination_meta"]["total_items"], 0, "{grants}");

    let invoice_id =
        common_domain::ids::InvoiceId::from_proto(issued["invoice_id"].as_str().unwrap()).unwrap();
    crate::test_customer_credit::pay_amount(&fixture.setup, invoice_id, 100_000)
        .await
        .unwrap();
    let (_, paid_grants) = fixture.get(&customer_path("uber", "credit-grants")).await;
    assert_eq!(paid_grants["data"][0]["amount"], 100_000);
    let actual = chrono::DateTime::parse_from_rfc3339(
        paid_grants["data"][0]["expires_at"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        actual.timestamp_millis(),
        chrono::DateTime::parse_from_rfc3339(&expires_at)
            .unwrap()
            .timestamp_millis()
    );

    // An invoice is issued in the customer's currency only.
    let (status, foreign) = fixture
        .post(
            &top_ups,
            Some("commit-2"),
            json!({"amount": 1_000, "currency": "USD", "collection": "SEND_INVOICE"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{foreign}");
}

/// Goodwill credit is a price reduction taken off before tax; credit paid outside Meteroid is a
/// means of payment, at face value after tax. The operator always says which.
#[tokio::test]
async fn admin_credit_nature_decides_before_or_after_tax() {
    let fixture = fixture().await;
    let mut client = fixture.clients.wallets.clone();
    let currency = fixture
        .setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .unwrap()
        .currency;
    let wallet = client
        .create_wallet(wallets::CreateWalletRequest {
            customer_id: CUST_UBER_ID.as_proto(),
            currency: currency.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .wallet
        .unwrap();
    let grant = |amount: i64, grant_type: wallets::CreditGrantType| wallets::GrantCreditRequest {
        customer_id: CUST_UBER_ID.as_proto(),
        wallet_id: wallet.id.clone(),
        amount,
        priority: None,
        expires_at: None,
        reason: "Settled by bank transfer".to_owned(),
        reference: Some("RCPT-42".to_owned()),
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        applicable_product_ids: Vec::new(),
        expires_in_months: None,
        subscription_id: None,
        applies_to: wallets::CreditAppliesTo::All.into(),
        grant_type: grant_type.into(),
    };

    let unspecified = client
        .grant_credit(grant(1_000, wallets::CreditGrantType::Unspecified))
        .await
        .expect_err("the nature is required");
    assert_eq!(unspecified.code(), tonic::Code::InvalidArgument);

    client
        .grant_credit(grant(3_000, wallets::CreditGrantType::Promotional))
        .await
        .unwrap();
    let paid = client
        .grant_credit(grant(5_000, wallets::CreditGrantType::Paid))
        .await
        .unwrap()
        .into_inner();
    let recorded = paid
        .wallet
        .unwrap()
        .grants
        .into_iter()
        .find(|grant| grant.id == paid.credit_grant_id)
        .unwrap();
    assert_eq!(recorded.r#type(), wallets::CreditGrantType::Paid);
    assert_eq!(
        recorded.cost,
        Some(5_000),
        "paid outside Meteroid at face value"
    );
    assert_eq!(recorded.cost_currency.as_deref(), Some(currency.as_str()));

    let draft = crate::test_credit_promotional::taxed_draft(
        &fixture.setup,
        vec![crate::test_credit_promotional::taxed("Seats", 10_000)],
        false,
    )
    .await
    .unwrap();
    let finalized = crate::test_customer_credit::finalize(&fixture.setup, draft.id)
        .await
        .unwrap()
        .invoice;
    assert_eq!(finalized.promotional_credits, 3_000);
    assert_eq!(
        finalized.tax_amount, 1_400,
        "20% of what the goodwill credit left"
    );
    assert_eq!(finalized.total, 8_400);
    assert_eq!(
        finalized.applied_credits, 5_000,
        "the payment covers the taxed total"
    );
    assert_eq!(finalized.amount_due, 3_400);
}

#[tokio::test]
async fn admin_wallet_rpcs_grant_remove_and_list() {
    let fixture = fixture().await;
    let mut client = fixture.clients.wallets.clone();
    let currency = fixture
        .setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .unwrap()
        .currency;
    let wallet = client
        .create_wallet(wallets::CreateWalletRequest {
            customer_id: CUST_UBER_ID.as_proto(),
            currency,
        })
        .await
        .unwrap()
        .into_inner()
        .wallet
        .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let grant = || wallets::GrantCreditRequest {
        customer_id: CUST_UBER_ID.as_proto(),
        wallet_id: wallet.id.clone(),
        amount: 4_000,
        priority: None,
        expires_at: None,
        reason: "Goodwill".to_owned(),
        reference: Some("TICKET-9".to_owned()),
        idempotency_key: key.clone(),
        applicable_product_ids: Vec::new(),
        expires_in_months: None,
        subscription_id: None,
        applies_to: wallets::CreditAppliesTo::All.into(),
        grant_type: wallets::CreditGrantType::Promotional.into(),
    };
    let granted = client.grant_credit(grant()).await.unwrap().into_inner();
    assert!(!granted.replayed);
    let replayed = client.grant_credit(grant()).await.unwrap().into_inner();
    assert!(replayed.replayed);
    assert_eq!(replayed.credit_grant_id, granted.credit_grant_id);
    let funded = replayed.wallet.unwrap();
    assert_eq!(funded.available, 4_000);
    assert!(funded.grants[0].removable);

    let removed = client
        .remove_credit(wallets::RemoveCreditRequest {
            customer_id: CUST_UBER_ID.as_proto(),
            credit_grant_id: granted.credit_grant_id.clone(),
            amount: Some(1_500),
            reason: "Over-granted".to_owned(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(removed.removed, 1_500);
    let reduced = removed.wallet.unwrap();
    assert_eq!(reduced.available, 2_500);
    assert_eq!(reduced.status(), wallets::WalletStatus::Healthy);

    let other_customer = client
        .grant_credit(wallets::GrantCreditRequest {
            customer_id: CUST_SPOTIFY_ID.as_proto(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            ..grant()
        })
        .await
        .expect_err("the wallet is not Spotify's");
    assert_eq!(other_customer.code(), tonic::Code::NotFound);

    let summaries = client
        .list_wallet_summaries(wallets::ListWalletSummariesRequest {
            pagination: None,
            search: Some("ube".to_owned()),
            status: None,
            auto_top_up_failing: false,
        })
        .await
        .unwrap()
        .into_inner();
    let summary = summaries
        .wallets
        .iter()
        .find(|summary| summary.id == wallet.id)
        .expect("the roster lists the wallet");
    assert_eq!(summary.available, 2_500);
    assert_eq!(summary.status(), wallets::WalletStatus::Healthy);

    let filtered = |status: wallets::WalletStatus, auto_top_up_failing: bool| {
        let mut client = client.clone();
        async move {
            client
                .list_wallet_summaries(wallets::ListWalletSummariesRequest {
                    pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                        per_page: Some(100),
                        page: 0,
                    }),
                    search: Some("ube".to_owned()),
                    status: (status != wallets::WalletStatus::Unspecified).then_some(status.into()),
                    auto_top_up_failing,
                })
                .await
                .unwrap()
                .into_inner()
        }
    };
    let healthy = filtered(wallets::WalletStatus::Healthy, false).await;
    assert!(
        healthy
            .wallets
            .iter()
            .any(|summary| summary.id == wallet.id)
    );
    assert_eq!(
        healthy.pagination_meta.map(|meta| meta.total_items),
        Some(u32::try_from(healthy.wallets.len()).unwrap())
    );
    let depleted = filtered(wallets::WalletStatus::Depleted, false).await;
    assert!(
        depleted
            .wallets
            .iter()
            .all(|summary| summary.id != wallet.id)
    );
    let failing = filtered(wallets::WalletStatus::Unspecified, true).await;
    assert!(
        failing
            .wallets
            .iter()
            .all(|summary| summary.id != wallet.id)
    );
    assert!(
        summaries
            .wallets
            .iter()
            .all(|summary| summary.customer_id == CUST_UBER_ID.as_proto())
    );

    let ledger = client
        .list_ledger_entries(wallets::ListLedgerEntriesRequest {
            wallet_id: wallet.id.clone(),
            pagination: None,
        })
        .await
        .unwrap()
        .into_inner();
    let kinds: Vec<_> = ledger.entries.iter().map(|entry| entry.r#type()).collect();
    assert_eq!(
        kinds,
        vec![
            wallets::LedgerEntryType::Removed,
            wallets::LedgerEntryType::Granted
        ]
    );
}

#[tokio::test]
async fn portal_wallets_are_the_token_customers_own() {
    let fixture = fixture().await;
    let uber_wallet = invoice_wallet(&fixture.setup, CUST_UBER_ID).await;
    let spotify_wallet = invoice_wallet(&fixture.setup, CUST_SPOTIFY_ID).await;
    let manage = fixture.portal(CUST_UBER_ID, PortalScope::ALL.to_vec());
    let mut client = manage.client.clone();

    let balances = client
        .get_balances(manage.request(portal::GetBalancesRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(!balances.top_up_enabled);
    assert!(
        balances
            .wallets
            .iter()
            .all(|wallet| wallet.id != spotify_wallet.as_proto())
    );
    assert!(balances.wallets.iter().all(|wallet| !wallet.can_top_up));

    let foreign = client
        .list_ledger(manage.request(portal::ListLedgerRequest {
            wallet_id: spotify_wallet.as_proto(),
            pagination: None,
        }))
        .await
        .expect_err("another customer's wallet");
    assert_eq!(foreign.code(), tonic::Code::NotFound);

    let disabled = client
        .quote_top_up(manage.request(portal::QuoteTopUpRequest {
            wallet_id: uber_wallet.as_proto(),
            amount: 5_000,
        }))
        .await
        .expect_err("portal top-ups are off");
    assert_eq!(disabled.code(), tonic::Code::FailedPrecondition);

    fixture.set_portal_switches(true, true).await;
    let quote = client
        .quote_top_up(manage.request(portal::QuoteTopUpRequest {
            wallet_id: uber_wallet.as_proto(),
            amount: 5_000,
        }))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    assert_eq!((quote.charge, quote.tax), (5_000, 0));

    let read_only = fixture.portal(CUST_UBER_ID, vec![PortalScope::Read]);
    let denied = read_only
        .client
        .clone()
        .top_up_wallet(read_only.request(portal::TopUpWalletRequest {
            wallet_id: uber_wallet.as_proto(),
            amount: 5_000,
            payment_method_id: CUST_UBER_PAYMENT_METHOD_ID.as_proto(),
            idempotency_key: "portal-top-up".to_owned(),
            expected_quote: Some(quote.clone()),
            return_url: None,
        }))
        .await
        .expect_err("a read token cannot charge");
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);

    let charged = client
        .top_up_wallet(manage.request(portal::TopUpWalletRequest {
            wallet_id: uber_wallet.as_proto(),
            amount: 5_000,
            payment_method_id: CUST_UBER_PAYMENT_METHOD_ID.as_proto(),
            idempotency_key: "portal-top-up".to_owned(),
            expected_quote: Some(quote),
            return_url: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(charged.status(), portal::TopUpStatus::Succeeded);
    drain(&fixture.setup).await;
    let mut states = Vec::new();
    for key in ["portal-top-up", "never-sent"] {
        let state = read_only
            .client
            .clone()
            .get_top_up(read_only.request(portal::GetTopUpRequest {
                wallet_id: uber_wallet.as_proto(),
                idempotency_key: key.to_owned(),
            }))
            .await
            .unwrap()
            .into_inner();
        states.push((state.status, state.failure_reason));
    }
    assert_eq!(
        states,
        [
            (Some(portal::TopUpStatus::Succeeded.into()), None),
            (None, None)
        ],
        "a read token sees what its key became; nothing was sent under an unknown key"
    );

    let statement = client
        .list_ledger(manage.request(portal::ListLedgerRequest {
            wallet_id: uber_wallet.as_proto(),
            pagination: None,
        }))
        .await
        .unwrap()
        .into_inner();
    let purchase = &statement.entries[0];
    let number = purchase
        .invoice_number
        .as_deref()
        .expect("the purchase names its invoice");
    assert_eq!(purchase.description, format!("Purchased credit · {number}"));
}

#[tokio::test]
async fn rest_wallet_threshold_is_the_wallet_of_its_currency() {
    let fixture = fixture().await;
    let wallet = fixture
        .setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .unwrap();
    let again = fixture
        .setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "eur")
        .await
        .unwrap();
    assert_eq!(wallet.id, again.id, "one wallet per currency");
    // Written as stored by the usage controls, which own the thresholds' write path.
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_credit_account::dsl as a;
        let mut conn = fixture.setup.store.get_conn().await.unwrap();
        diesel::update(a::customer_credit_account.filter(a::id.eq(wallet.id)))
            .set(a::low_balance_thresholds_minor.eq(vec![1_500_i64]))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    let (status, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wallets["data"][0]["low_balance_thresholds"], json!([1_500]));
}

#[tokio::test]
async fn rest_top_up_authentication_url_is_a_checkout_session() {
    let fixture = fixture().await;
    crate::data::payment::set_mock_charge_behavior(
        &fixture.setup.store.pool,
        meteroid_store::domain::connectors::MockChargeBehavior::RequiresAction,
    )
    .await;
    let (status, receipt) = fixture
        .post(
            &customer_path("uber", "top-ups"),
            Some("rest-3ds"),
            json!({"amount": 3_000}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{receipt}");
    assert_eq!(receipt["status"], "REQUIRES_CUSTOMER_ACTION");
    assert!(
        receipt["checkout_url"]
            .as_str()
            .unwrap()
            .contains("/checkout?token=")
    );
    use meteroid_store::repositories::InvoiceInterface;
    let invoice = fixture
        .setup
        .store
        .get_detailed_invoice_by_id(
            TENANT_ID,
            common_domain::ids::InvoiceId::from_proto(receipt["invoice_id"].as_str().unwrap())
                .unwrap(),
        )
        .await
        .unwrap()
        .invoice;
    assert_eq!(
        invoice.status,
        meteroid_store::domain::InvoiceStatusEnum::Draft
    );
    assert_eq!(invoice.invoice_number, "draft");
}

/// T12: usage-only and any-charge credit of one currency share the customer's one wallet, in
/// gRPC, REST and the portal; usage-only credit is the sum of its grants.
#[tokio::test]
async fn wallets_collapse() {
    use crate::helpers::credit::{Grant, fund_grant};
    use meteroid_store::domain::credit::CreditApplication;
    let fixture = fixture().await;
    let store = &fixture.setup.store;
    for (application, amount) in [
        (CreditApplication::Usage, 3_000),
        (CreditApplication::Usage, 500),
        (CreditApplication::Invoice, 2_000),
    ] {
        fund_grant(
            store,
            TENANT_ID,
            CUST_UBER_ID,
            Some("EUR"),
            Grant {
                application,
                ..Grant::invoice(amount)
            },
        )
        .await;
    }

    let wallets = store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .unwrap();
    assert_eq!(wallets.len(), 1, "{wallets:?}");
    assert_eq!(wallets[0].available_minor, 5_500);

    let listed = fixture
        .clients
        .wallets
        .clone()
        .list_customer_wallets(wallets::ListCustomerWalletsRequest {
            customer_id: CUST_UBER_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.wallets.len(), 1);
    assert_eq!(listed.wallets[0].grants.len(), 3);

    let customer_portal = fixture.portal(CUST_UBER_ID, vec![PortalScope::Read]);
    let balances = customer_portal
        .client
        .clone()
        .get_balances(customer_portal.request(portal::GetBalancesRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(balances.wallets.len(), 1);
    assert_eq!(balances.wallets[0].available, 5_500);

    let (status, rest) = fixture.get(&customer_path("uber", "wallets")).await;
    assert_eq!(status, StatusCode::OK, "{rest}");
    assert_eq!(rest["data"].as_array().unwrap().len(), 1);
    assert_eq!(rest["data"][0]["credit_on_account"], 5_500);
    assert_eq!(rest["data"][0]["credit_on_account_usage_only"], 3_500);
}

/// A wallet carries its spendable grants; the used-up ones are listed page by page, newest
/// first, each with how it was consumed.
#[tokio::test]
async fn used_up_grants_are_listed_page_by_page() {
    use crate::helpers::credit::{expire_grant, fund};
    use common_grpc::meteroid::common::v1::Pagination;
    let fixture = fixture().await;
    fixture
        .setup
        .skip_credit_invariants("moves grants' expiry into the past by raw SQL");
    let store = &fixture.setup.store;
    let live = fund(store, TENANT_ID, CUST_UBER_ID, Some("EUR"), 1_000).await;
    let older = fund(store, TENANT_ID, CUST_UBER_ID, Some("EUR"), 2_000).await;
    let newer = fund(store, TENANT_ID, CUST_UBER_ID, Some("EUR"), 3_000).await;
    expire_grant(store, older).await;
    expire_grant(store, newer).await;
    let mut client = fixture.clients.wallets.clone();

    let listed = client
        .list_customer_wallets(wallets::ListCustomerWalletsRequest {
            customer_id: CUST_UBER_ID.as_proto(),
        })
        .await
        .unwrap()
        .into_inner();
    let wallet = &listed.wallets[0];
    assert!(
        wallet
            .grants
            .iter()
            .any(|grant| grant.id == live.as_proto())
    );
    assert!(wallet.first_credit_at.is_some());

    let page = |page| wallets::ListWalletGrantsRequest {
        customer_id: CUST_UBER_ID.as_proto(),
        wallet_id: wallet.id.clone(),
        subscription_id: None,
        used_up: true,
        pagination: Some(Pagination {
            page,
            per_page: Some(1),
        }),
    };
    let first = client
        .list_wallet_grants(page(0))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.pagination_meta.unwrap().total_items, 2);
    assert_eq!(
        first
            .grants
            .iter()
            .map(|g| g.id.clone())
            .collect::<Vec<_>>(),
        vec![newer.as_proto()]
    );
    assert_eq!(first.activity[0].credit_grant_id, newer.as_proto());
    let second = client
        .list_wallet_grants(page(1))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        second
            .grants
            .iter()
            .map(|g| g.id.clone())
            .collect::<Vec<_>>(),
        vec![older.as_proto()]
    );

    let refused = client
        .list_wallet_grants(wallets::ListWalletGrantsRequest {
            customer_id: CUST_SPOTIFY_ID.as_proto(),
            ..page(0)
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::NotFound);
}

/// The statement downloads as CSV through a signed link, for its customer only.
#[tokio::test]
async fn the_ledger_downloads_as_csv_through_a_signed_link() {
    use crate::helpers::credit::fund;
    let fixture = fixture().await;
    let store = &fixture.setup.store;
    fund(store, TENANT_ID, CUST_UBER_ID, Some("EUR"), 1_050).await;
    fund(store, TENANT_ID, CUST_UBER_ID, Some("EUR"), 2_000).await;
    let wallet_id = store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .unwrap()[0]
        .id;
    let mut client = fixture.clients.wallets.clone();
    let token = client
        .export_customer_ledger(wallets::ExportCustomerLedgerRequest {
            customer_id: CUST_UBER_ID.as_proto(),
            wallet_id: Some(wallet_id.as_proto()),
            subscription_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .token;

    let download = |customer: CustomerId| {
        fixture.client.get(fixture.url(&format!(
            "/files/v1/credit-ledger/{}?token={token}",
            customer.as_proto()
        )))
    };
    let response = download(CUST_UBER_ID).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/csv; charset=utf-8"
    );
    let body = response.text().await.unwrap();
    let lines = body.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "{body}");
    assert!(
        lines[0].starts_with("Date (UTC),Wallet,Movement,"),
        "{body}"
    );
    assert!(lines[1].contains(",20.00,30.50,EUR,"), "{body}");

    let other = download(CUST_SPOTIFY_ID).send().await.unwrap();
    assert_eq!(other.status(), StatusCode::FORBIDDEN);

    let refused = client
        .export_customer_ledger(wallets::ExportCustomerLedgerRequest {
            customer_id: CUST_SPOTIFY_ID.as_proto(),
            wallet_id: Some(wallet_id.as_proto()),
            subscription_id: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::NotFound);
}

/// The customer's agreed price per credit is read and written on the wallet: set in the
/// customer's currency, kept when omitted, cleared by `null`, a decimal string only, and refused
/// on a money wallet.
#[tokio::test]
async fn rest_patch_wallet_sets_and_clears_the_agreed_unit_price() {
    use meteroid_store::domain::credit_units::CreditUnitNew;
    use meteroid_store::repositories::CreditUnitsInterface;
    let fixture = fixture().await;
    fixture
        .setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(
                TENANT_ID,
                "HALYARD",
                "Halyard credit",
                "Halyard credits",
                None,
                0,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let path = customer_path("uber", "wallets/HALYARD");

    let (status, opened) = fixture.patch(&path, Some("price-0"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert!(opened["unit_price"].is_null(), "{opened}");

    let (status, set) = fixture
        .patch(&path, Some("price-1"), json!({"unit_price": "0.00080"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set["unit_price"], "0.0008");
    assert_eq!(set["unit_price_currency"], "EUR");

    let (status, kept) = fixture.patch(&path, Some("price-2"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["unit_price"], "0.0008", "omitted: unchanged");
    let (_, wallets) = fixture.get(&customer_path("uber", "wallets")).await;
    let unit = wallets["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|wallet| wallet["credit_unit"] == "HALYARD")
        .cloned()
        .unwrap();
    assert_eq!(unit["unit_price"], "0.0008", "GET agrees: {unit}");

    let (status, cleared) = fixture
        .patch(&path, Some("price-3"), json!({"unit_price": null}))
        .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert!(cleared["unit_price"].is_null());
    assert!(cleared["unit_price_currency"].is_null());

    let (status, number) = fixture
        .patch(&path, Some("price-4"), json!({"unit_price": 0.0008}))
        .await;
    assert!(
        status.is_client_error(),
        "a float price is refused: {number}"
    );
    let (status, zero) = fixture
        .patch(&path, Some("price-5"), json!({"unit_price": "0"}))
        .await;
    assert!(status.is_client_error(), "{zero}");
    let (status, money) = fixture
        .patch(
            &customer_path("uber", "wallets/EUR"),
            Some("price-6"),
            json!({"unit_price": "0.9"}),
        )
        .await;
    assert!(status.is_client_error(), "{money}");
}

/// Without the usage controls, a PATCH carrying any of their fields is refused before the wallet
/// is opened; a patch of core's fields still applies.
#[tokio::test]
async fn rest_patch_wallet_refuses_usage_controls_without_the_module() {
    let fixture = fixture().await;
    let path = customer_path("uber", "wallets/EUR");

    for (key, body) in [
        ("controls-1", json!({"low_balance_thresholds": [1_000]})),
        ("controls-2", json!({"usage_at_zero": "ALWAYS_PAUSE"})),
        ("controls-3", json!({"auto_top_up": null})),
    ] {
        let (status, refused) = fixture.patch(&path, Some(key), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert!(
            refused
                .to_string()
                .contains("Usage controls are not available in this edition"),
            "{refused}"
        );
    }

    let (status, opened) = fixture.patch(&path, Some("controls-4"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["usage_at_zero"], "FOLLOW_PLAN");
    assert_eq!(opened["low_balance_thresholds"], json!([]));
    assert_eq!(opened["auto_top_up"], Value::Null);
}

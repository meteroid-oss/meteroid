use crate::data::ids::{CUST_UBER_CONNECTION_ID, CUST_UBER_ID, TENANT_ID};
use crate::data::payment::{run_customer_payment_methods_seed, run_mock_payment_provider_seed};
use crate::helpers::invoice_lines::line;
use crate::helpers::invoices::{draft, finalize};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::pgmq::PgmqInterface;
use reqwest::StatusCode;
use serde_json::{Value, json};

struct Fixture {
    setup: MeteroidSetup,
    client: reqwest::Client,
    api_key: String,
}

impl Fixture {
    async fn new() -> Self {
        crate::helpers::init::logging();
        let database =
            meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
        let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await;
        let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
        let mut clients = AllClients::from_channel(
            setup.channel.clone(),
            auth.token.as_str(),
            "TESTORG",
            "testslug",
        );
        let api_key = clients
            .api_tokens
            .create_api_token(CreateApiTokenRequest {
                name: "invoice-payments".to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .api_key;
        Self {
            setup,
            client: http_client_tracing::client(),
            api_key,
        }
    }

    async fn draft_invoice(&self) -> InvoiceId {
        draft(&self.setup, vec![line("Service", 1_000, false)])
            .await
            .unwrap()
            .id
    }

    fn url(&self, invoice: InvoiceId) -> String {
        format!(
            "{}/api/v1/invoices/{}",
            self.setup.config.rest_api_external_url,
            invoice.as_base62()
        )
    }

    async fn get_invoice(&self, invoice: InvoiceId) -> Value {
        self.client
            .get(self.url(invoice))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn post(
        &self,
        invoice: InvoiceId,
        action: &str,
        key: Option<&str>,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let mut request = self
            .client
            .post(format!("{}/{action}", self.url(invoice)))
            .bearer_auth(&self.api_key);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, expected, "{action}: {body}");
        if status == StatusCode::NO_CONTENT {
            assert!(body.is_empty());
            return Value::Null;
        }
        serde_json::from_str(&body).unwrap()
    }

    async fn webhook_events(&self) -> Vec<OutboxEvent> {
        self.setup
            .store
            .pgmq_read(
                PgmqQueue::WebhookOut,
                MessageReadQty(1000),
                MessageReadVtSec(0),
            )
            .await
            .unwrap()
            .into_iter()
            .filter_map(|message| message.message)
            .map(|message| serde_json::from_value(message.0).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn records_and_reverses_external_payments_once() {
    let fixture = Fixture::new().await;
    let invoice = fixture.draft_invoice().await;
    let other = fixture.draft_invoice().await;
    let due = finalize(&fixture.setup, invoice)
        .await
        .unwrap()
        .invoice
        .amount_due;
    let partial = due / 4;
    let payment = json!({
        "amount": partial,
        "payment_date": "2026-01-31T23:00:00-05:00",
        "reference": "wire-123",
    });
    fixture
        .post(
            invoice,
            "payments",
            None,
            Some(payment.clone()),
            StatusCode::BAD_REQUEST,
        )
        .await;

    let record = || {
        fixture.post(
            invoice,
            "payments",
            Some("partial"),
            Some(payment.clone()),
            StatusCode::CREATED,
        )
    };
    let (first, replay) = tokio::join!(record(), record());
    assert_eq!(first, replay);
    assert_eq!(first["amount"], partial);
    assert_eq!(first["provider_transaction_id"], "wire-123");
    assert_eq!(first["processed_at"], "2026-02-01T04:00:00.000Z");

    let partially_paid = fixture.get_invoice(invoice).await;
    assert_eq!(partially_paid["amount_due"], due - partial);
    assert_eq!(partially_paid["payment_status"], "PARTIALLY_PAID");
    assert_eq!(partially_paid["transactions"].as_array().unwrap().len(), 1);

    fixture
        .post(
            invoice,
            "payments",
            Some("partial"),
            Some(json!({"amount": partial + 1})),
            StatusCode::CONFLICT,
        )
        .await;
    fixture
        .post(
            invoice,
            "mark-paid",
            Some("remainder"),
            Some(json!({"amount": partial})),
            StatusCode::BAD_REQUEST,
        )
        .await;

    // The rejected amount rolled back its claim, so the corrected request can use the key.
    let mark_paid = || {
        fixture.post(
            invoice,
            "mark-paid",
            Some("remainder"),
            Some(json!({"amount": due - partial, "payment_date": "2026-02-01T08:00:00Z"})),
            StatusCode::CREATED,
        )
    };
    let paid = mark_paid().await;
    assert_eq!(paid, mark_paid().await);
    assert_eq!(fixture.get_invoice(invoice).await["payment_status"], "PAID");

    let reversal = format!("payments/{}/reverse", paid["id"].as_str().unwrap());
    let reverse = |id, status| {
        fixture.post(
            id,
            &reversal,
            Some("bank-return"),
            Some(json!({"kind": "RETURNED", "reversed_at": "2026-02-02T00:00:00.123100Z"})),
            status,
        )
    };
    reverse(other, StatusCode::NOT_FOUND).await;
    let reversed = reverse(invoice, StatusCode::OK).await;
    assert_eq!(reversed, reverse(invoice, StatusCode::OK).await);
    fixture
        .post(
            invoice,
            &reversal,
            Some("bank-return"),
            Some(json!({"kind": "RETURNED", "reversed_at": "2026-02-02T00:00:00.123900Z"})),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(reversed["amount_reversed"], due - partial);
    let reopened = fixture.get_invoice(invoice).await;
    assert_eq!(reopened["amount_due"], due - partial);
    assert_eq!(reopened["payment_status"], "PARTIALLY_PAID");

    let events = fixture.webhook_events().await;
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, OutboxEvent::InvoicePaid(event) if event.invoice_id == invoice))
            .count(),
        1
    );
    assert_eq!(events.iter().filter(|e| matches!(e, OutboxEvent::PaymentReversed(event) if event.invoice_id == Some(invoice))).count(), 1);
}

#[tokio::test]
async fn marks_only_finalized_invoices_uncollectible() {
    let fixture = Fixture::new().await;
    let invoice = fixture.draft_invoice().await;
    let other = fixture.draft_invoice().await;
    let mark = |id, status| fixture.post(id, "mark-uncollectible", Some("write-off"), None, status);
    mark(invoice, StatusCode::BAD_REQUEST).await;
    finalize(&fixture.setup, invoice).await.unwrap();
    mark(invoice, StatusCode::NO_CONTENT).await;
    let uncollectible = fixture.get_invoice(invoice).await;
    assert_eq!(uncollectible["status"], "UNCOLLECTIBLE");
    assert!(uncollectible["marked_as_uncollectible_at"].is_string());

    mark(invoice, StatusCode::NO_CONTENT).await;
    assert_eq!(fixture.get_invoice(invoice).await, uncollectible);
    mark(other, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn does_not_disturb_an_in_flight_hosted_payment() {
    let fixture = Fixture::new().await;
    run_mock_payment_provider_seed(&fixture.setup.store.pool, false).await;
    run_customer_payment_methods_seed(&fixture.setup.store.pool).await;
    let invoice_id = fixture.draft_invoice().await;
    let invoice = finalize(&fixture.setup, invoice_id).await.unwrap().invoice;
    let transaction_id = PaymentTransactionId::new();
    let mut conn = fixture.setup.store.get_conn().await.unwrap();
    PaymentTransactionRowNew {
        id: transaction_id,
        tenant_id: TENANT_ID,
        invoice_id: Some(invoice_id),
        provider_transaction_id: None,
        amount: invoice.amount_due,
        currency: invoice.currency,
        payment_method_id: None,
        status: PaymentStatusEnum::Pending,
        payment_type: PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: None,
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: Some(CUST_UBER_ID),
        pending_provider_intent_id: Some("hosted-intent".into()),
        pending_connection_id: Some(CUST_UBER_CONNECTION_ID),
        parent_transaction_id: None,
        credit_note_id: None,
        refund_mode: None,
        reversal_reason: None,
    }
    .insert(&mut conn)
    .await
    .unwrap();

    fixture
        .post(
            invoice_id,
            &format!("payments/{transaction_id}/reverse"),
            Some("reverse-pending"),
            Some(json!({"kind": "RETURNED"})),
            StatusCode::BAD_REQUEST,
        )
        .await;
    fixture
        .post(
            invoice_id,
            "payments",
            Some("record-while-pending"),
            Some(json!({"amount": invoice.amount_due})),
            StatusCode::BAD_REQUEST,
        )
        .await;
    let transaction = PaymentTransactionRow::get_by_id(&mut conn, transaction_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(transaction.status, PaymentStatusEnum::Pending);
    let invoice = fixture.get_invoice(invoice_id).await;
    assert_eq!(invoice["transactions"].as_array().unwrap().len(), 1);
}

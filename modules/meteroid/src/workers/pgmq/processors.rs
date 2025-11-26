use crate::services::invoice_rendering::PdfRenderingService;
use crate::services::storage::ObjectStoreService;
use crate::workers::pgmq::billable_metric_sync::BillableMetricSync;
use crate::workers::pgmq::hubspot_sync::HubspotSync;
use crate::workers::pgmq::invoice_orchestration::InvoiceOrchestration;
use crate::workers::pgmq::outbox::{PgmqOutboxDispatch, PgmqOutboxProxy};
use crate::workers::pgmq::payment_request::PaymentRequest;
use crate::workers::pgmq::pdf_render::PdfRender;
use crate::workers::pgmq::pennylane_sync::PennylaneSync;
use crate::workers::pgmq::processor::{ProcessorConfig, run};
use crate::workers::pgmq::send_email::EmailSender;
use crate::workers::pgmq::webhook_out::WebhookOut;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec, ReadCt};
use hubspot_client::client::HubspotClient;
use meteroid_mailer::service::MailerService;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::{Services, Store};
use pennylane_client::client::PennylaneClient;
use rand::Rng;
use std::sync::Arc;

pub async fn run_outbox_dispatch(store: Arc<Store>) {
    let queue = PgmqQueue::OutboxEvent;
    let processor = Arc::new(PgmqOutboxDispatch::new(store.clone()));

    run(ProcessorConfig {
        name: processor_name("OutboxDispatch"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(100),
        vt: MessageReadVtSec(10),
        delete_succeeded: false,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_pdf_render(store: Arc<Store>, pdf_service: Arc<PdfRenderingService>) {
    let queue = PgmqQueue::InvoicePdfRequest;
    let processor = Arc::new(PdfRender::new(pdf_service));

    run(ProcessorConfig {
        name: processor_name("PdfRender"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_webhook_out(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::WebhookOut;
    let processor = Arc::new(PgmqOutboxProxy::new(
        store.clone(),
        Arc::new(WebhookOut::new(services.clone())),
    ));

    run(ProcessorConfig {
        name: processor_name("WebhookOut"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_hubspot_sync(store: Arc<Store>, hubspot_client: Arc<HubspotClient>) {
    let queue = PgmqQueue::HubspotSync;
    let processor = Arc::new(HubspotSync::new(store.clone(), hubspot_client));

    run(ProcessorConfig {
        name: processor_name("HubspotSync"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(50),
        vt: MessageReadVtSec(20),
        delete_succeeded: false,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_pennylane_sync(
    store: Arc<Store>,
    pennylane_client: Arc<PennylaneClient>,
    storage: Arc<dyn ObjectStoreService>,
) {
    let queue = PgmqQueue::PennylaneSync;
    let processor = Arc::new(PennylaneSync::new(store.clone(), pennylane_client, storage));

    run(ProcessorConfig {
        name: processor_name("PennylaneSync"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: false,
        sleep_duration: std::time::Duration::from_millis(2000),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_metric_sync(store: Arc<Store>, usage_client: Arc<dyn UsageClient>) {
    let queue = PgmqQueue::BillableMetricSync;
    let processor = Arc::new(BillableMetricSync::new(
        usage_client.clone(),
        (*store).clone(),
    ));

    run(ProcessorConfig {
        name: processor_name("BillableMetricSync"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_invoice_orchestration(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::InvoiceOrchestration;
    let processor = Arc::new(PgmqOutboxProxy::new(
        store.clone(),
        Arc::new(InvoiceOrchestration::new(store.clone(), services)),
    ));
    run(ProcessorConfig {
        name: processor_name("InvoiceOrchestration"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_email_sender(
    store: Arc<Store>,
    mailer: Arc<dyn MailerService>,
    object_store: Arc<dyn ObjectStoreService>,
    public_url: String,
    rest_api_url: String,
    jwt_secret: secrecy::SecretString,
) {
    let queue = PgmqQueue::SendEmailRequest;
    let processor = Arc::new(EmailSender::new(
        mailer,
        object_store,
        public_url,
        rest_api_url,
        jwt_secret,
        store.clone(),
    ));

    run(ProcessorConfig {
        name: processor_name("EmailSender"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: false,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_payment_request(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::PaymentRequest;
    let processor = Arc::new(PaymentRequest::new(services));

    run(ProcessorConfig {
        name: processor_name("PaymentRequest"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(180),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(2000),
        max_read_count: ReadCt(3), // 3 retries. TODO applicative payment retry with mails
    })
    .await;
}

fn processor_name(prefix: &str) -> String {
    format!("{}-{}", prefix, rand::rng().random::<u16>())
}

use crate::pgmq::PgmqResult;
use crate::pgmq::credit_note_pdf_render::CreditNotePdfRender;
use crate::pgmq::hubspot_sync::HubspotSync;
use crate::pgmq::invoice_orchestration::InvoiceOrchestration;
use crate::pgmq::payment_request::{PAYMENT_REQUEST_MAX_READS, PaymentRequest};
use crate::pgmq::pdf_render::PdfRender;
use crate::pgmq::pennylane_sync::PennylaneSync;
use crate::pgmq::processor::{Noop, PgmqHandler, ProcessorConfig, run};
use crate::pgmq::quote_conversion::QuoteConversion;
use crate::pgmq::send_email::EmailSender;
use crate::pgmq::vat_validation::VatValidation;
use crate::pgmq::webhook_in::WebhookIn;
use crate::pgmq::webhook_out::WebhookOut;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec, ReadCt};
use hubspot_client::client::HubspotClient;
use meteroid_app::features::{NoWhitelabel, WhitelabelResolver};
use meteroid_app::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid_app::services::invoice_rendering::PdfRenderingService;
use meteroid_app::services::storage::ObjectStoreService;
use meteroid_mailer::service::MailerService;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::{Services, Store};
use pennylane_client::client::PennylaneClient;
use std::sync::Arc;

pub async fn run_pdf_render(store: Arc<Store>, pdf_service: Arc<PdfRenderingService>) {
    let queue = PgmqQueue::InvoicePdfRequest;
    let processor = Arc::new(PdfRender::new(pdf_service));

    run(ProcessorConfig {
        name: processor_name("PdfRender"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        // Sequential renders plus S3 uploads.
        vt: MessageReadVtSec(120),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(250),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_credit_note_pdf_render(
    store: Arc<Store>,
    pdf_service: Arc<CreditNotePdfRenderingService>,
) {
    let queue = PgmqQueue::CreditNotePdfRequest;
    let processor = Arc::new(CreditNotePdfRender::new(store.clone(), pdf_service));

    run(ProcessorConfig {
        name: processor_name("CreditNotePdfRender"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(120),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(10),
    })
    .await;
}

// Used in tests
pub async fn run_once_credit_note_pdf_render(
    store: Arc<Store>,
    pdf_service: Arc<CreditNotePdfRenderingService>,
) {
    let queue = PgmqQueue::CreditNotePdfRequest;
    let processor = Arc::new(CreditNotePdfRender::new(store.clone(), pdf_service));
    let _ = crate::pgmq::processor::run_once(
        queue,
        processor,
        store,
        MessageReadQty(10),
        MessageReadVtSec(20),
        true,
        ReadCt(10),
    )
    .await;
}

/// Turns outbox events into stored webhook messages and pending deliveries.
pub async fn run_webhook_out(store: Arc<Store>) {
    let processor = Arc::new(WebhookOut::new(store.clone()));

    run(ProcessorConfig {
        name: processor_name("WebhookOut"),
        queue: PgmqQueue::WebhookOut,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(60),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1000),
        max_read_count: ReadCt(10),
    })
    .await;
}

// Used in tests
pub async fn run_once_webhook_out(store: Arc<Store>) {
    let processor = Arc::new(WebhookOut::new(store.clone()));
    let _ = crate::pgmq::processor::run_once(
        PgmqQueue::WebhookOut,
        processor,
        store,
        MessageReadQty(10),
        MessageReadVtSec(20),
        true,
        ReadCt(10),
    )
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
        // Sequential, rate-limited API calls per tenant.
        vt: MessageReadVtSec(300),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(3000),
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
        // Sequential, rate-limited API calls and PDF uploads per tenant.
        vt: MessageReadVtSec(300),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(3000),
        max_read_count: ReadCt(10),
    })
    .await;
}

pub async fn run_invoice_orchestration(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::InvoiceOrchestration;
    let processor = Arc::new(InvoiceOrchestration::new(store.clone(), services));
    run(ProcessorConfig {
        name: processor_name("InvoiceOrchestration"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(250),
        max_read_count: ReadCt(10),
    })
    .await;
}

// Used in tests
pub async fn run_once_invoice_orchestration(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::InvoiceOrchestration;
    let processor = Arc::new(InvoiceOrchestration::new(store.clone(), services));
    let _ = crate::pgmq::processor::run_once(
        queue,
        processor,
        store,
        MessageReadQty(10),
        MessageReadVtSec(20),
        true,
        ReadCt(10),
    )
    .await;
}

// Used in tests
pub async fn run_once_payment_request(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::PaymentRequest;
    let processor = Arc::new(PaymentRequest::new(services));
    let _ = crate::pgmq::processor::run_once(
        queue,
        processor,
        store,
        MessageReadQty(10),
        MessageReadVtSec(180),
        true,
        PAYMENT_REQUEST_MAX_READS,
    )
    .await;
}

/// Rendering an email needs the PDF, the DB and SMTP: 120 s before a slow send is redelivered.
const EMAIL_SENDER_VT: MessageReadVtSec = MessageReadVtSec(120);

/// Used in tests. One batch of the email queue; `Ok(0)` means the queue is empty.
#[allow(clippy::too_many_arguments)]
pub async fn run_once_send_email(
    store: Arc<Store>,
    services: Arc<Services>,
    mailer: Arc<dyn MailerService>,
    object_store: Arc<dyn ObjectStoreService>,
    public_url: String,
    rest_api_url: String,
    jwt_secret: secrecy::SecretString,
) -> PgmqResult<usize> {
    let queue = PgmqQueue::SendEmailRequest;
    let processor = Arc::new(EmailSender::new(
        mailer,
        object_store,
        public_url,
        rest_api_url,
        jwt_secret,
        store.clone(),
        services,
        Arc::new(NoWhitelabel),
    ));
    crate::pgmq::processor::run_once(
        queue,
        processor,
        store,
        MessageReadQty(10),
        EMAIL_SENDER_VT,
        true,
        ReadCt(10),
    )
    .await
    .map(|read| read.0 as usize)
}

/// Used in tests: hands already-read messages to a fresh sender, so the same message can be
/// replayed or processed concurrently. Returns how many messages the handler succeeded on.
#[allow(clippy::too_many_arguments)]
pub async fn handle_send_email_messages(
    store: Arc<Store>,
    services: Arc<Services>,
    mailer: Arc<dyn MailerService>,
    object_store: Arc<dyn ObjectStoreService>,
    public_url: String,
    rest_api_url: String,
    jwt_secret: secrecy::SecretString,
    msgs: &[meteroid_store::domain::pgmq::PgmqMessage],
) -> PgmqResult<usize> {
    let sender = EmailSender::new(
        mailer,
        object_store,
        public_url,
        rest_api_url,
        jwt_secret,
        store,
        services,
        Arc::new(NoWhitelabel),
    );
    sender.handle(msgs).await.map(|r| r.succeeded.len())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_email_sender(
    store: Arc<Store>,
    services: Arc<Services>,
    mailer: Arc<dyn MailerService>,
    object_store: Arc<dyn ObjectStoreService>,
    public_url: String,
    rest_api_url: String,
    jwt_secret: secrecy::SecretString,
    whitelabel: Arc<dyn WhitelabelResolver>,
) {
    let queue = PgmqQueue::SendEmailRequest;
    let processor = Arc::new(EmailSender::new(
        mailer,
        object_store,
        public_url,
        rest_api_url,
        jwt_secret,
        store.clone(),
        services,
        whitelabel,
    ));

    run(ProcessorConfig {
        name: processor_name("EmailSender"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: EMAIL_SENDER_VT,
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(500),
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
        // Charges run one by one, each up to the 45s provider timeout: keep a batch within VT.
        qty: MessageReadQty(3),
        vt: MessageReadVtSec(180),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(250),
        max_read_count: PAYMENT_REQUEST_MAX_READS,
    })
    .await;
}

pub async fn run_webhook_in(
    store: Arc<Store>,
    services: Arc<Services>,
    object_store: Arc<dyn ObjectStoreService>,
) {
    let queue = PgmqQueue::WebhookIn;
    let processor = Arc::new(WebhookIn::new(services, store.clone(), object_store));

    run(ProcessorConfig {
        name: processor_name("WebhookIn"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(180),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1000),
        max_read_count: ReadCt(10),
    })
    .await;
}

// Used in tests
pub async fn run_once_webhook_in(
    store: Arc<Store>,
    services: Arc<Services>,
    object_store: Arc<dyn ObjectStoreService>,
) {
    let queue = PgmqQueue::WebhookIn;
    let processor = Arc::new(WebhookIn::new(services, store.clone(), object_store));
    let _ = crate::pgmq::processor::run_once(
        queue,
        processor,
        store,
        MessageReadQty(10),
        MessageReadVtSec(60),
        false,
        ReadCt(10),
    )
    .await;
}

pub async fn run_quote_conversion(store: Arc<Store>, services: Arc<Services>) {
    let queue = PgmqQueue::QuoteConversion;
    let processor = Arc::new(QuoteConversion::new(services));

    run(ProcessorConfig {
        name: processor_name("QuoteConversion"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(10),
        vt: MessageReadVtSec(30),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(1500),
        max_read_count: ReadCt(3),
    })
    .await;
}

pub async fn run_vat_validation(store: Arc<Store>) {
    let queue = PgmqQueue::VatValidation;
    let processor = Arc::new(VatValidation::new(store.clone()));

    // Low throughput on purpose: VIES rate-limits hard and jobs make blocking
    // network calls. VIES unavailability is retried via delayed re-enqueues
    // (see RETRY_DELAYS_SECS in the handler); the visibility timeout and
    // `max_read_count` only cover transient store errors and crashes.
    run(ProcessorConfig {
        name: processor_name("VatValidation"),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(5),
        vt: MessageReadVtSec(120),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_millis(2000),
        max_read_count: ReadCt(6),
    })
    .await;
}

pub use meteroid_app::workers::pgmq::processor::processor_name;

pub async fn run_noop(store: Arc<Store>, queue: PgmqQueue) {
    let processor = Arc::new(Noop);

    run(ProcessorConfig {
        name: processor_name(&format!("Noop{}", queue)),
        queue,
        handler: processor,
        store,
        qty: MessageReadQty(100),
        vt: MessageReadVtSec(20),
        delete_succeeded: true,
        sleep_duration: std::time::Duration::from_secs(10),
        max_read_count: ReadCt(10),
    })
    .await;
}

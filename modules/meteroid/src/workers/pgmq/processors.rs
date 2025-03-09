use crate::services::invoice_rendering::PdfRenderingService;
use crate::workers::pgmq::outbox::{PgmqOutboxDispatch, PgmqOutboxProxy};
use crate::workers::pgmq::pdf_render::PdfRender;
use crate::workers::pgmq::processor::{ProcessorConfig, run};
use crate::workers::pgmq::webhook_out::WebhookOut;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec, ReadCt};
use meteroid_store::Store;
use meteroid_store::domain::pgmq::PgmqQueue;
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

pub async fn run_pdf_render(store: Arc<Store>, pdf_service: PdfRenderingService) {
    let queue = PgmqQueue::InvoicePdfRequest;
    let processor = Arc::new(PdfRender::new(pdf_service));

    run(ProcessorConfig {
        name: processor_name("PdfRender"),
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

pub async fn run_webhook_out(store: Arc<Store>) {
    let queue = PgmqQueue::WebhookOut;
    let processor = Arc::new(PgmqOutboxProxy::new(
        store.clone(),
        Arc::new(WebhookOut::new(store.clone())),
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

fn processor_name(prefix: &str) -> String {
    format!("{}-{}", prefix, rand::rng().random::<u16>())
}

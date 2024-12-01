use crate::workers::kafka::processor::run_message_processor;
use crate::workers::kafka::webhook::WebhookHandler;
use kafka::config::KafkaConnectionConfig;
use meteroid_store::Store;
use std::sync::Arc;

pub async fn run_webhook_outbox_processor(kafka_config: &KafkaConnectionConfig, store: Arc<Store>) {
    let topics = vec!["outbox.event.customer"];
    let group_id = "webhook_outbox_processor";

    let handler = Arc::new(WebhookHandler::new(store));

    run_message_processor(kafka_config, &topics, group_id, handler).await;
}

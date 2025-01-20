use async_trait::async_trait;
use kafka::config::KafkaConnectionConfig;
use kafka::consumer::create_kafka_consumer;
use rdkafka::consumer::{CommitMode, Consumer};
use rdkafka::message::BorrowedMessage;
use std::sync::Arc;

#[async_trait]
pub trait MessageHandler: Send + Sync {
    async fn handle(&self, message: &BorrowedMessage<'_>)
        -> Result<(), Box<dyn std::error::Error>>;
}

pub(crate) async fn run_message_processor<H>(
    conn_config: &KafkaConnectionConfig,
    topics: &[&str],
    group_id: &str,
    handler: Arc<H>,
) where
    H: MessageHandler,
{
    if conn_config.is_none() {
        log::warn!("Kafka connection config is not defined");
        return;
    }

    let kafka_consumer = create_kafka_consumer(conn_config, topics, group_id);

    loop {
        match kafka_consumer.recv().await {
            Err(e) => log::warn!("Kafka consumer error: {}", e),
            Ok(m) => {
                match handler.handle(&m).await {
                    Err(e) => {
                        // todo introduce dlq
                        log::warn!("Failed to process kafka message: {:?}", e)
                    }
                    Ok(_) => log::debug!("Message processed"),
                }

                match kafka_consumer.commit_message(&m, CommitMode::Async) {
                    Err(e) => log::warn!("Failed to commit kafka message: {}", e),
                    Ok(_) => log::debug!("Message committed"),
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rdkafka::message::BorrowedMessage;
    use rdkafka::mocking::MockCluster;
    use rdkafka::producer::{FutureProducer, FutureRecord};
    use rdkafka::{ClientConfig, Message};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::task;

    pub struct TestMessageHandler {
        captured_messages: Arc<Mutex<Vec<String>>>,
    }

    impl TestMessageHandler {
        pub fn new() -> Self {
            Self {
                captured_messages: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn get_captured_messages(&self) -> Vec<String> {
            self.captured_messages.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessageHandler for TestMessageHandler {
        async fn handle(
            &self,
            message: &BorrowedMessage<'_>,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let Some(payload) = message.payload_view::<str>().transpose()? {
                log::info!("Processing message: {}", payload);
                self.captured_messages
                    .lock()
                    .unwrap()
                    .push(payload.to_string());
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_run_message_processor() {
        env_logger::builder().is_test(true).try_init().unwrap();

        const TOPIC: &str = "test_topic";
        let mock_cluster = MockCluster::new(3).unwrap();

        log::info!("servers: {}", mock_cluster.bootstrap_servers());

        mock_cluster
            .create_topic(TOPIC, 5, 3)
            .expect("Failed to create topic");

        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", mock_cluster.bootstrap_servers())
            .create()
            .expect("Producer creation error");

        let conn_config = KafkaConnectionConfig {
            bootstrap_servers: Some(mock_cluster.bootstrap_servers()),
            security_protocol: None,
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
        };

        for i in 1..=10 {
            producer
                .send_result(
                    FutureRecord::to(TOPIC)
                        .key(&i.to_string())
                        .payload(format!("dummy-{}", &i).as_str()),
                )
                .unwrap()
                .await
                .unwrap()
                .unwrap();
        }

        let group_id = "test-group";

        let handler = Arc::new(TestMessageHandler::new());
        let handler_clone = handler.clone();

        let task = task::spawn(async move {
            run_message_processor(&conn_config, &[TOPIC], group_id, handler).await;
        });

        tokio::time::sleep(Duration::from_secs(5)).await;
        task.abort();

        assert_eq!(handler_clone.get_captured_messages().len(), 10);
    }
}

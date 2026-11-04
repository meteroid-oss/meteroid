//! Store primitives behind the pgmq processor's retry backoff and dead-lettering.

use common_domain::pgmq::{Message, MessageReadQty, MessageReadVtSec};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::schema::dead_letter_message as dlm;
use rstest::rstest;

use meteroid_store::domain::dead_letter::DeadLetterMessageNew;
use meteroid_store::domain::pgmq::{PgmqMessage, PgmqMessageNew, PgmqQueue};
use meteroid_store::repositories::dead_letter::DeadLetterInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;

use crate::harness::{TestEnv, test_env_minimal};

const QUEUE: PgmqQueue = PgmqQueue::QuoteConversion;

async fn send_and_read(env: &TestEnv) -> PgmqMessage {
    env.store()
        .pgmq_send_batch(
            QUEUE,
            vec![PgmqMessageNew {
                message: Some(Message(serde_json::json!({ "k": "v" }))),
                headers: None,
                tenant_id: None,
            }],
        )
        .await
        .expect("send");

    read_visible(env).await.pop().expect("message is visible")
}

async fn read_visible(env: &TestEnv) -> Vec<PgmqMessage> {
    env.store()
        .pgmq_read(QUEUE, MessageReadQty(10), MessageReadVtSec(0))
        .await
        .expect("read")
}

#[rstest]
#[tokio::test]
async fn test_set_vt_hides_message_until_delay(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let msg = send_and_read(&env).await;

    env.store()
        .pgmq_set_vt(QUEUE, vec![(msg.msg_id, 3600)])
        .await
        .expect("set_vt");

    assert!(read_visible(&env).await.is_empty());

    env.store()
        .pgmq_set_vt(QUEUE, vec![(msg.msg_id, 0)])
        .await
        .expect("set_vt");

    let visible = read_visible(&env).await;
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].msg_id, msg.msg_id);
}

#[rstest]
#[tokio::test]
async fn test_dead_letter_moves_message_out_of_queue(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let msg = send_and_read(&env).await;

    env.store()
        .dead_letter_messages(
            QUEUE,
            vec![DeadLetterMessageNew {
                tenant_id: None,
                queue: QUEUE.as_str().to_string(),
                pgmq_msg_id: msg.msg_id.0,
                message: msg.message.as_ref().map(|m| m.0.clone()),
                headers: None,
                read_ct: msg.read_ct.0,
                enqueued_at: msg.enqueued_at,
                last_error: Some("boom".to_string()),
            }],
        )
        .await
        .expect("dead-letter");

    assert!(read_visible(&env).await.is_empty());

    let last_error: Option<String> = dlm::table
        .filter(dlm::queue.eq(QUEUE.as_str()))
        .filter(dlm::pgmq_msg_id.eq(msg.msg_id.0))
        .select(dlm::last_error)
        .first(&mut env.conn().await)
        .await
        .expect("dead letter row exists");
    assert_eq!(last_error.as_deref(), Some("boom"));
}

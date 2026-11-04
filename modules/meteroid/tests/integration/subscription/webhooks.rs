//! Outgoing subscription-lifecycle webhook emission tests.
//!
//! These assert on the events landing on the `webhook_out` pgmq queue (which is
//! bound to every routing key), i.e. exactly what the webhook fan-out consumes.

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use meteroid_store::domain::SubscriptionPatch;
use meteroid_store::domain::outbox_event::{OutboxEvent, SubscriptionUpdateType};
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

/// Read every event currently on the `webhook_out` queue and decode it back to the
/// `OutboxEvent` that was published (the message body is the outbox payload).
async fn webhook_events(env: &TestEnv) -> Vec<OutboxEvent> {
    let msgs = env
        .store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(200),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out queue");

    msgs.into_iter()
        .filter_map(|m| m.message)
        .filter_map(|m| serde_json::from_value::<OutboxEvent>(m.0).ok())
        .collect()
}

/// Patching billing configuration emits a single `subscription.updated` carrying the
/// `billing_configuration_updated` discriminator and the new config in its payload.
#[rstest]
#[tokio::test]
async fn patch_emits_subscription_updated_billing_config(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.store()
        .patch_subscription(
            TENANT_ID,
            SubscriptionPatch {
                id: sub_id,
                charge_automatically: None,
                auto_advance_invoices: Some(false),
                net_terms: None,
                invoice_memo: None,
                purchase_order: None,
                payment_methods_config: None,
                usage_filter: None,
                custom_properties: None,
            },
            Actor::System,
        )
        .await
        .expect("patch subscription");

    let config_updates: Vec<_> = webhook_events(&env)
        .await
        .into_iter()
        .filter_map(|e| match e {
            OutboxEvent::SubscriptionUpdated(s)
                if s.change_type == Some(SubscriptionUpdateType::BillingConfigurationUpdated) =>
            {
                Some(s)
            }
            _ => None,
        })
        .collect();

    assert_eq!(
        config_updates.len(),
        1,
        "exactly one billing-configuration subscription.updated event"
    );
    assert_eq!(config_updates[0].subscription_id, sub_id);
    assert!(
        !config_updates[0].auto_advance_invoices,
        "payload reflects the patched auto_advance_invoices=false"
    );
}

/// Cancelling an active subscription emits `subscription.cancelled` carrying the reason.
#[rstest]
#[tokio::test]
async fn cancel_emits_subscription_cancelled_with_reason(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Effective mid-period; the scheduled cancel is due (relative to now) and fires below.
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            Some("customer_request".to_string()),
            CancellationEffectiveAt::Date(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
        )
        .await
        .expect("schedule cancellation");

    env.process_cycles().await;

    let cancelled: Vec<_> = webhook_events(&env)
        .await
        .into_iter()
        .filter_map(|e| match e {
            OutboxEvent::SubscriptionCancelled(s) => Some(s),
            _ => None,
        })
        .collect();

    assert!(
        cancelled.iter().any(|s| s.subscription_id == sub_id
            && s.cancellation_reason.as_deref() == Some("customer_request")),
        "cancellation emits subscription.cancelled carrying the reason (got {cancelled:?})"
    );
}

/// A fixed-term subscription reaching its end date emits the dedicated `subscription.ended`
/// event — and, crucially, never a `subscription.cancelled` (completion is not a cancellation).
#[rstest]
#[tokio::test]
async fn fixed_term_completion_emits_ended(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .end_date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Completion takes two transitions: the renew detects the end date and arms
    // `EndSubscription`, the next transition actually completes.
    for _ in 0..3 {
        env.process_cycles().await;
    }

    let events = webhook_events(&env).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, OutboxEvent::SubscriptionEnded(s) if s.subscription_id == sub_id)),
        "natural completion emits subscription.ended"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, OutboxEvent::SubscriptionCancelled(_)))
            .count(),
        0,
        "completion must not emit subscription.cancelled"
    );
}

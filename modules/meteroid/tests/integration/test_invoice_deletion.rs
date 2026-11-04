//! Audit-trail and webhook coverage for draft invoice deletion.

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::BaseId;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::ActorTypeEnum;
use rstest::rstest;

use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

async fn invoice_activities(
    env: &TestEnv,
    invoice_id: common_domain::ids::InvoiceId,
) -> Vec<EntityActivityRow> {
    let mut conn = env.conn().await;
    EntityActivityRow::list_by_entity(
        &mut conn,
        TENANT_ID,
        "invoice",
        invoice_id.as_uuid(),
        None,
        50,
    )
    .await
    .expect("list activities")
}

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

/// Deleting a draft records an `invoice.deleted` audit row naming the actor, and emits
/// the matching webhook — the deleted row is otherwise unrecoverable.
#[rstest]
#[tokio::test]
async fn test_delete_draft_invoice_is_audited_and_webhooked(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let draft = env
        .insert_draft_invoice(sub_id, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .await;

    env.store()
        .delete_invoice(Actor::User { id: USER_ID }, draft.id, TENANT_ID)
        .await
        .expect("delete draft invoice");

    assert!(
        !env.get_invoices(sub_id)
            .await
            .iter()
            .any(|i| i.id == draft.id),
        "the draft should be gone"
    );

    let activities = invoice_activities(&env, draft.id).await;
    let deleted = activities
        .iter()
        .find(|a| a.activity_type == "invoice.deleted")
        .expect("expected an invoice.deleted audit row");

    assert_eq!(deleted.actor_type, ActorTypeEnum::User);
    assert_eq!(deleted.actor_uuid, Some(USER_ID.as_uuid()));
    assert_eq!(deleted.agg_customer_id, Some(draft.customer_id.as_uuid()));
    assert_eq!(deleted.agg_subscription_id, Some(sub_id.as_uuid()));

    let events = webhook_events(&env).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, OutboxEvent::InvoiceDeleted(d) if d.invoice_id == draft.id)),
        "expected an InvoiceDeleted webhook event"
    );
}

/// Only drafts can be deleted: the SQL delete filters on `Draft`, so anything else has
/// to be rejected rather than reported as a success that deleted nothing.
#[rstest]
#[tokio::test]
async fn test_delete_non_draft_invoice_is_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    let finalized = invoices
        .first()
        .expect("subscription should have an invoice");

    let err = env
        .store()
        .delete_invoice(Actor::User { id: USER_ID }, finalized.id, TENANT_ID)
        .await;
    assert!(err.is_err(), "a finalized invoice must not be deletable");

    assert!(
        env.get_invoices(sub_id)
            .await
            .iter()
            .any(|i| i.id == finalized.id),
        "the invoice should still be there"
    );
    assert!(
        invoice_activities(&env, finalized.id)
            .await
            .iter()
            .all(|a| a.activity_type != "invoice.deleted"),
        "a rejected delete must not be audited"
    );
}

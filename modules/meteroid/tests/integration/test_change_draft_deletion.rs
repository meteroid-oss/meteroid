//! Deleting a change's draft adjustment waives it (R-124, R-166): the change stays, what the draft
//! bills is forgiven, and its notes stand. Each case runs beside an auto-advancing twin.
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use chrono::{NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId};
use meteroid_store::domain::Invoice;
use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use rstest::rstest;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

async fn started(env: &TestEnv, customer: CustomerId, auto_advance: bool) -> SubscriptionId {
    let builder = subscription()
        .customer(customer)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial();
    let builder = if auto_advance {
        builder
    } else {
        builder.no_auto_advance()
    };
    builder.create(env.services()).await
}

async fn of_kind(
    env: &TestEnv,
    sub: SubscriptionId,
    kind: InvoiceType,
    from: NaiveDate,
) -> Invoice {
    env.get_invoices(sub)
        .await
        .into_iter()
        .find(|invoice| invoice.invoice_type == kind && invoice.billing_period_start == from)
        .unwrap_or_else(|| panic!("no {kind:?} invoice from {from}"))
}

async fn upgrade(env: &TestEnv, sub: SubscriptionId) -> Option<InvoiceId> {
    env.services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            day(1, 16),
        )
        .await
        .expect("upgrade")
        .adjustment_invoice_id
}

async fn noted(env: &TestEnv, invoice: InvoiceId) -> i64 {
    env.store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice)
        .await
        .unwrap()
        .iter()
        .map(|note| note.total)
        .sum()
}

/// What the subscription's issued invoices bill, less the notes against them.
async fn billed(env: &TestEnv, sub: SubscriptionId) -> i64 {
    let mut net = 0;
    for invoice in env.get_invoices(sub).await {
        if matches!(
            invoice.status,
            InvoiceStatusEnum::Void | InvoiceStatusEnum::Closed | InvoiceStatusEnum::Draft
        ) {
            continue;
        }
        net += invoice.total - noted(env, invoice.id).await.abs();
    }
    net
}

/// Starter from Jan 1, upgraded to Pro on Jan 16 on both subscriptions; the drafted one's
/// adjustment is deleted, its January issued first when `january_first`, then February issued.
async fn waived_beside_twin(env: &TestEnv, january_first: bool) {
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap());
    let twin = started(env, CUST_SPOTIFY_ID, true).await;
    let drafted = started(env, CUST_UBER_ID, false).await;
    let january = of_kind(env, drafted, InvoiceType::Recurring, day(1, 1)).await;
    if january_first {
        env.services()
            .finalize_invoice(Actor::System, january.id, TENANT_ID)
            .await
            .expect("January issued");
    }

    env.set_now(Utc.with_ymd_and_hms(2024, 1, 16, 12, 0, 0).unwrap());
    let twin_adjustment = upgrade(env, twin).await.expect("the twin's upgrade bills");
    let adjustment = upgrade(env, drafted)
        .await
        .expect("the upgrade bills the new plan");
    let forgiven = env
        .store()
        .get_invoice_by_id(TENANT_ID, adjustment)
        .await
        .unwrap();
    assert_eq!(forgiven.status, InvoiceStatusEnum::Draft);
    assert!(forgiven.total > 0);
    env.store()
        .delete_invoice(Actor::User { id: USER_ID }, adjustment, TENANT_ID)
        .await
        .expect("waived");
    let closed = env
        .store()
        .get_invoice_by_id(TENANT_ID, adjustment)
        .await
        .expect("the waived draft stays");
    assert_eq!(closed.status, InvoiceStatusEnum::Closed);
    if !january_first {
        env.services()
            .finalize_invoice(Actor::System, january.id, TENANT_ID)
            .await
            .expect("January issued");
    }

    env.run_until(day(2, 4)).await;
    let february = of_kind(env, drafted, InvoiceType::Recurring, day(2, 1)).await;
    if february.status == InvoiceStatusEnum::Draft {
        env.services()
            .finalize_invoice(Actor::System, february.id, TENANT_ID)
            .await
            .expect("February issued");
    }
    let twin_february = of_kind(env, twin, InvoiceType::Recurring, day(2, 1)).await;

    let twin_january = of_kind(env, twin, InvoiceType::Recurring, day(1, 1)).await;
    assert_eq!(
        noted(env, january.id).await,
        noted(env, twin_january.id).await,
        "the upgrade's give-back stands"
    );
    let february = env
        .store()
        .get_invoice_by_id(TENANT_ID, february.id)
        .await
        .unwrap();
    assert_eq!(
        february.total, twin_february.total,
        "February bills Pro alone, never the waived stub"
    );
    let twin_adjustment = env
        .store()
        .get_invoice_by_id(TENANT_ID, twin_adjustment)
        .await
        .unwrap();
    assert_eq!(twin_adjustment.total, forgiven.total);
    assert_eq!(
        billed(env, twin).await - billed(env, drafted).await,
        forgiven.total,
        "only the waived stub goes unbilled"
    );
    let refreshed = env
        .store()
        .get_invoice_by_id(TENANT_ID, adjustment)
        .await
        .unwrap();
    assert_eq!(
        refreshed.status,
        InvoiceStatusEnum::Closed,
        "nothing reissues it"
    );
}

/// January issued, so the upgrade's note is issued at the change; its draft is then waived.
/// Rules: R-124, R-166, R-078
#[rstest]
#[tokio::test]
async fn test_a_change_draft_deleted_after_its_notes_is_waived(#[future] test_env: TestEnv) {
    let env = test_env.await;
    waived_beside_twin(&env, true).await;
}

/// January still a draft: the note comes once January is issued, after the waiver, and matches.
/// Rules: R-124, R-166, R-078
#[rstest]
#[tokio::test]
async fn test_a_change_draft_deleted_before_its_notes_is_waived(#[future] test_env: TestEnv) {
    let env = test_env.await;
    waived_beside_twin(&env, false).await;
}

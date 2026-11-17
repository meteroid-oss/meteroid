//! An invoice's deadlines: `finalize_after` while it waits out its grace,
//! `next_payment_attempt_at` while a payment is still to be tried; each is cleared once its
//! invoice no longer waits for it.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::InvoiceId;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::invoices::InvoiceRow;
use diesel_models::schema::invoice::dsl as i;
use meteroid_store::domain::enums::InvoiceStatusEnum;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

async fn deadlines(
    env: &TestEnv,
    invoice: InvoiceId,
) -> (Option<NaiveDateTime>, Option<NaiveDateTime>) {
    let row = InvoiceRow::find_by_id(&mut env.conn().await, TENANT_ID, invoice)
        .await
        .expect("the invoice");
    (row.finalize_after, row.next_payment_attempt_at)
}

/// Feb 1's renewal waits out a day's grace as a draft; finalized, it waits for nothing more. A
/// payment attempt still to come on it is dropped when it is voided.
#[tokio::test]
async fn an_invoice_clears_its_deadlines_once_it_no_longer_waits_for_them() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    env.set_consolidate_and_grace(false, 24).await;
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let renewal = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2, 1))
        .expect("the Feb 1 renewal");
    assert_eq!(renewal.status, InvoiceStatusEnum::Draft);
    let (finalize_after, _) = deadlines(&env, renewal.id).await;
    assert!(finalize_after.is_some(), "a draft waits out its grace");

    env.process_due_events().await;
    let finalized = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|invoice| invoice.id == renewal.id)
        .expect("the renewal");
    assert_eq!(finalized.status, InvoiceStatusEnum::Finalized);
    assert_eq!(deadlines(&env, renewal.id).await, (None, None));

    let attempt = day(2, 5).and_hms_opt(9, 0, 0).unwrap();
    diesel::update(i::invoice.filter(i::id.eq(renewal.id)))
        .set(i::next_payment_attempt_at.eq(Some(attempt)))
        .execute(&mut env.conn().await)
        .await
        .expect("an attempt still to come");
    env.try_void_invoice(renewal.id).await.expect("void");
    assert_eq!(deadlines(&env, renewal.id).await, (None, None));
}

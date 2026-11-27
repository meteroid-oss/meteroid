//! An amount due below the minimum charge is not collected: it is carried, after tax, to the
//! customer's next invoice, which collects it once the sum reaches the minimum.
use chrono::NaiveDate;
use common_domain::actor::Actor;
use meteroid_store::domain::enums::CreditType;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::{CarrySummary, Invoice, InvoicingEntityPatch};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditNoteInterface,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env};

async fn carry(env: &TestEnv, invoice: &Invoice) -> CarrySummary {
    let links = env
        .store()
        .list_carry_links(TENANT_ID, invoice.id)
        .await
        .expect("carry links");
    CarrySummary::of(invoice, &links)
}

async fn set_min_charge(env: &TestEnv, amount: Option<i64>) {
    env.store()
        .patch_invoicing_entity(
            Actor::User { id: USER_ID },
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                min_charge_amount: Some(amount),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("set the minimum charge");
}

/// LeetCode bills €35 a month in advance: under a €50 minimum, January is carried to February,
/// which collects both; March, alone again under the minimum, is carried on.
#[rstest]
#[tokio::test]
async fn a_small_amount_due_is_carried_to_the_next_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(january.status, InvoiceStatusEnum::Finalized);
    assert_eq!(january.total, 3_500);
    assert_eq!(carry(&env, &january).await.carried_forward, 3_500);
    assert_eq!(january.amount_due, 0);
    env.run_outbox_and_orchestration().await;
    let january = env
        .store()
        .get_invoice_by_id(TENANT_ID, january.id)
        .await
        .expect("invoice");
    assert_eq!(
        january.payment_status,
        InvoicePaymentStatus::Carried,
        "nothing was collected: not paid until February is"
    );

    env.process_cycles().await;
    let february = env.get_invoices(sub_id).await.remove(1);
    assert_eq!(
        february.total, 3_500,
        "the carried debt is not billed again"
    );
    assert_eq!(carry(&env, &february).await.previous_balance, 3_500);
    assert_eq!(carry(&env, &february).await.carried_forward, 0);
    assert_eq!(february.amount_due, 7_000);
    assert_eq!(february.payment_status, InvoicePaymentStatus::Unpaid);

    let links = env
        .store()
        .list_carry_links(TENANT_ID, january.id)
        .await
        .expect("links");
    assert_eq!(links.len(), 1);
    assert!(links[0].outgoing);
    assert_eq!(links[0].invoice_id, Some(february.id));

    let paid_on = chrono::Utc::now().fixed_offset() - chrono::Duration::days(1);
    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            february.id,
            rust_decimal::Decimal::new(7_000, 2),
            paid_on,
            None,
        )
        .await
        .expect("pay February");
    env.run_outbox_and_orchestration().await;
    let january = env
        .store()
        .get_invoice_by_id(TENANT_ID, january.id)
        .await
        .expect("invoice");
    let february = env
        .store()
        .get_invoice_by_id(TENANT_ID, february.id)
        .await
        .expect("invoice");
    assert_eq!(february.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(
        january.payment_status,
        InvoicePaymentStatus::Paid,
        "paid with February"
    );
    assert_eq!(january.paid_at, february.paid_at);

    env.process_cycles().await;
    let march = env.get_invoices(sub_id).await.remove(2);
    assert_eq!(
        carry(&env, &march).await.previous_balance,
        0,
        "February's debt was collected there"
    );
    assert_eq!(carry(&env, &march).await.carried_forward, 3_500);
    assert_eq!(march.amount_due, 0);
}

/// Voiding the invoice that took a carried debt hands it to the next invoice; the invoice that
/// carried it cannot be voided while a later invoice bills its debt.
#[rstest]
#[tokio::test]
async fn a_void_gives_the_carried_debt_back_to_the_next_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);
    env.process_cycles().await;
    let february = env.get_invoices(sub_id).await.remove(1);
    assert_eq!(carry(&env, &february).await.previous_balance, 3_500);

    let refused = env
        .try_void_invoice(january.id)
        .await
        .expect_err("January's debt is billed on February");
    assert!(
        format!("{refused:?}").contains("carried to a later invoice"),
        "{refused:?}"
    );
    env.try_void_invoice(february.id)
        .await
        .expect("void February");

    env.process_cycles().await;
    let march = env.get_invoices(sub_id).await.remove(2);
    assert_eq!(
        carry(&env, &march).await.previous_balance,
        3_500,
        "January's debt moves on to March"
    );
    assert_eq!(march.amount_due, 7_000);
}

/// Without a minimum (no payment provider, no entity setting) every amount is collected.
#[rstest]
#[tokio::test]
async fn nothing_is_carried_without_a_minimum(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(carry(&env, &january).await.carried_forward, 0);
    assert_eq!(january.amount_due, 3_500);
}

/// The minimum is only carried when a later invoice will take it: an ending subscription
/// collects what it bills.
#[rstest]
#[tokio::test]
async fn an_ending_subscription_collects_its_last_amount(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .end_date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(carry(&env, &january).await.carried_forward, 0);
    assert_eq!(january.amount_due, 3_500);

    let fetched = env
        .store()
        .get_invoice_by_id(TENANT_ID, january.id)
        .await
        .expect("invoice");
    assert_eq!(carry(&env, &fetched).await.previous_balance, 0);
}

/// The seeded USD plan bills `cents` a month in advance, from January 2024, for the EUR entity's
/// customer. The rates in force then make 1.00 EUR worth 1.10 USD.
async fn usd_subscription(
    env: &TestEnv,
    cents: i64,
    end_date: Option<NaiveDate>,
) -> common_domain::ids::SubscriptionId {
    crate::helpers::plans::as_draft(
        env.store(),
        PLAN_VERSION_USD_ID,
        env.store().create_price_component_from_ref(
            "Fee".to_owned(),
            ProductRef::New {
                name: "USD fee".to_owned(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Rate {
                    rate: Decimal::new(cents, 2),
                },
                credit_unit: None,
            })],
            PLAN_VERSION_USD_ID,
            TENANT_ID,
        ),
    )
    .await
    .expect("price the USD plan");
    let builder = subscription()
        .plan_version(PLAN_VERSION_USD_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    match end_date {
        Some(end) => builder.end_date(end),
        None => builder,
    }
    .on_start()
    .no_trial()
    .create(env.services())
    .await
}

/// The entity's 1.00 EUR floor holds a USD invoice to 1.10 USD.
#[rstest]
#[tokio::test]
async fn the_entity_minimum_converts_into_the_invoice_currency(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(100)).await;
    let sub_id = usd_subscription(&env, 105, None).await;

    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(january.currency, "USD");
    assert_eq!(
        carry(&env, &january).await.carried_forward,
        105,
        "1.05 USD is below 1.00 EUR"
    );
    assert_eq!(january.amount_due, 0);

    env.process_cycles().await;
    let february = env.get_invoices(sub_id).await.remove(1);
    assert_eq!(carry(&env, &february).await.previous_balance, 105);
    assert_eq!(february.amount_due, 210);
}

async fn collect_cards_with_stripe(env: &TestEnv) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::connectors::ConnectorRowNew;
    use diesel_models::enums::{ConnectorProviderEnum, ConnectorStatusEnum, ConnectorTypeEnum};
    use diesel_models::schema::invoicing_entity::dsl as ie;

    let mut conn = env.store().get_conn().await.expect("connection");
    let stripe = ConnectorRowNew {
        id: <common_domain::ids::ConnectorId as common_domain::ids::BaseId>::new(),
        tenant_id: TENANT_ID,
        alias: "stripe".to_owned(),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Stripe,
        data: None,
        sensitive: None,
        status: ConnectorStatusEnum::Active,
        account: None,
        webhook_route: None,
    }
    .insert(&mut conn)
    .await
    .expect("a Stripe connector");
    diesel::update(ie::invoicing_entity.filter(ie::id.eq(INVOICING_ENTITY_ID)))
        .set(ie::card_provider_id.eq(stripe.id))
        .execute(&mut conn)
        .await
        .expect("collect cards with Stripe");
    drop(conn);
}

/// With Stripe collecting for a EUR entity, a USD charge must reach Stripe's 0.50 EUR, about
/// 0.55 USD, though Stripe's own USD minimum is 0.50.
#[rstest]
#[tokio::test]
async fn the_provider_minimum_of_the_entity_currency_converts_too(#[future] test_env: TestEnv) {
    let env = test_env.await;
    collect_cards_with_stripe(&env).await;

    let sub_id = usd_subscription(&env, 52, None).await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(carry(&env, &january).await.carried_forward, 52);
    assert_eq!(january.amount_due, 0);
}

/// A part payment of an invoice that took a debt leaves it part paid, so it cannot be voided and
/// its payment is never lost with the debt handed on.
#[rstest]
#[tokio::test]
async fn a_part_paid_invoice_that_took_a_debt_is_part_paid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let february = env.get_invoices(sub_id).await.remove(1);
    assert_eq!(february.amount_due, 7_000);

    env.services()
        .add_manual_payment_transaction(
            Actor::System,
            TENANT_ID,
            february.id,
            rust_decimal::Decimal::new(2_000, 2),
            chrono::Utc::now().fixed_offset() - chrono::Duration::days(1),
            None,
        )
        .await
        .expect("part payment");
    env.run_outbox_and_orchestration().await;
    let february = env
        .store()
        .get_invoice_by_id(TENANT_ID, february.id)
        .await
        .expect("invoice");
    assert_eq!(february.amount_due, 5_000);
    assert_eq!(
        february.payment_status,
        InvoicePaymentStatus::PartiallyPaid,
        "2000 of the 7000 it asks"
    );
    assert!(env.try_void_invoice(february.id).await.is_err());
}

/// A debt cancellation of `cents` on `invoice`'s first line, finalized.
async fn cancel_debt(env: &TestEnv, invoice: &Invoice, cents: i64) {
    let note = env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: invoice.line_items[0].local_id.clone(),
                    amount_cents: cents,
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("create the note");
    env.store()
        .finalize_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("finalize the note");
}

async fn reload(env: &TestEnv, invoice: &Invoice) -> Invoice {
    env.store()
        .get_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .expect("invoice")
}

/// A note on a carried invoice cancels its debt where it stands: still waiting, or on the invoice
/// that took it. A note on that invoice credits its own lines only, never the debt it took.
#[rstest]
#[tokio::test]
async fn credit_notes_cancel_a_carried_debt_where_it_stands(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);

    cancel_debt(&env, &january, 1_000).await;
    let january = reload(&env, &january).await;
    assert_eq!(january.payment_status, InvoicePaymentStatus::Carried);
    assert_eq!(carry(&env, &january).await.carried_forward, 2_500);

    env.process_cycles().await;
    let february = env.get_invoices(sub_id).await.remove(1);
    assert_eq!(carry(&env, &february).await.previous_balance, 2_500);
    assert_eq!(february.amount_due, 6_000);

    cancel_debt(&env, &january, 500).await;
    let february = reload(&env, &february).await;
    assert_eq!(
        february.amount_due, 5_500,
        "January's note lowers what February asks"
    );
    assert_eq!(february.carried_balance, 2_000);

    cancel_debt(&env, &february, 3_500).await;
    let february = reload(&env, &february).await;
    assert_eq!(february.amount_due, 2_000, "January's debt is still owed");

    cancel_debt(&env, &january, 2_000).await;
    let (january, february) = (reload(&env, &january).await, reload(&env, &february).await);
    assert_eq!(
        january.payment_status,
        InvoicePaymentStatus::Paid,
        "nothing left owed"
    );
    assert_eq!(february.amount_due, 0);
    assert_eq!(february.carried_balance, 0);
}

/// A debt carried to an invoice that never comes, the subscription cancelled at the end of the
/// period that carried it, is open again on the invoice that carried it.
#[rstest]
#[tokio::test]
async fn a_debt_no_invoice_will_take_reopens_where_it_was_carried(#[future] test_env: TestEnv) {
    use chrono::TimeZone;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 10, 10, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("cancel at the end of January");
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 2, 5, 10, 0, 0).unwrap());
    env.process_cycles().await;
    env.services()
        .run_invoice_deadlines()
        .await
        .expect("deadlines");
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1, "nothing billed after January");
    let january = &invoices[0];
    assert_eq!(january.payment_status, InvoicePaymentStatus::Unpaid);
    assert_eq!(january.amount_due, 3_500);
    assert_eq!(january.carried_balance, 0);
    assert_eq!(carry(&env, january).await.carried_forward, 0);
}

/// What is left on a last invoice below the provider's minimum is not chased: the operator
/// settles it or writes it off.
#[rstest]
#[tokio::test]
async fn a_last_amount_below_the_provider_minimum_is_not_chased(#[future] test_env: TestEnv) {
    use meteroid_store::domain::payment_reminders::NotEligibleReason;
    use meteroid_store::repositories::PaymentRemindersInterface;

    let env = test_env.await;
    collect_cards_with_stripe(&env).await;
    let sub_id = usd_subscription(&env, 30, NaiveDate::from_ymd_opt(2024, 2, 1)).await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(january.amount_due, 30, "no later invoice to carry it to");
    env.run_outbox_and_orchestration().await;

    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, january.id, chrono::Utc::now().naive_utc())
        .await
        .expect("reminder state");
    assert_eq!(
        state.not_eligible_reason,
        Some(NotEligibleReason::BelowProviderMinimum)
    );
}

/// A free plan of the customer's, billing nothing, writes no invoice (R-104): it is no later
/// invoice to carry an amount to.
async fn free_subscription(env: &TestEnv) {
    let version = free_plan(env).await;
    subscription()
        .plan_version(version)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
}

/// A plan billing nothing, in the seeded product family.
async fn free_plan(env: &TestEnv) -> common_domain::ids::PlanVersionId {
    use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Free", version)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Free Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            Decimal::ZERO,
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan");
    version
}

/// An ending subscription collects its last amount, though the customer also holds a free plan.
#[rstest]
#[tokio::test]
async fn a_free_plan_takes_no_carried_amount(#[future] test_env: TestEnv) {
    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    free_subscription(&env).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .end_date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(carry(&env, &january).await.carried_forward, 0);
    assert_eq!(january.amount_due, 3_500);
}

/// A debt carried in January reopens where it was carried once its subscription ended, though
/// the customer also holds a free plan.
#[rstest]
#[tokio::test]
async fn a_debt_a_free_plan_will_not_take_reopens(#[future] test_env: TestEnv) {
    use chrono::TimeZone;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 10, 10, 0, 0).unwrap());
    free_subscription(&env).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("cancel at the end of January");
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 2, 5, 10, 0, 0).unwrap());
    env.process_cycles().await;
    env.services()
        .run_invoice_deadlines()
        .await
        .expect("deadlines");
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1, "nothing billed after January");
    let january = &invoices[0];
    assert_eq!(january.payment_status, InvoicePaymentStatus::Unpaid);
    assert_eq!(january.amount_due, 3_500);
    assert_eq!(carry(&env, january).await.carried_forward, 0);
}

/// A debt carried in January to the next invoice reopens where it was carried once a downgrade
/// to a free plan applies on Feb 1: the free plan writes no invoice to take it.
#[rstest]
#[tokio::test]
async fn a_downgrade_to_a_free_plan_reopens_a_carried_debt(#[future] test_env: TestEnv) {
    use chrono::TimeZone;

    let env = test_env.await;
    set_min_charge(&env, Some(5_000)).await;
    let free = free_plan(&env).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 1, 10, 10, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(
        carry(&env, &january).await.carried_forward,
        3_500,
        "carried"
    );
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, free, vec![])
        .await
        .expect("a downgrade to the free plan at the period's end");
    env.set_now(chrono::Utc.with_ymd_and_hms(2024, 2, 5, 10, 0, 0).unwrap());
    env.process_cycles().await;
    env.services()
        .run_invoice_deadlines()
        .await
        .expect("deadlines");
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1, "nothing billed on the free plan");
    let january = &invoices[0];
    assert_eq!(january.payment_status, InvoicePaymentStatus::Unpaid);
    assert_eq!(january.amount_due, 3_500);
    assert_eq!(carry(&env, january).await.carried_forward, 0);
}

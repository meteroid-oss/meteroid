//! A preview of a change not written yet prices the subscription's facts with the rows the change
//! would write laid over them, through the pipeline the change bills with: each preview here is
//! what applying the change then issues, to the cent, and writes nothing.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CheckoutSessionId, CreditNoteId, InvoiceId, SubscriptionId};
use meteroid_grpc::meteroid::portal::checkout::v1 as portal;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::InvoiceType;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::{ChangeDocument, PlanChangeMode};
use meteroid_store::domain::{CreateCheckoutSession, Invoice, SlotUpgradeBillingMode};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use rstest::rstest;
use rust_decimal_macros::dec;

use super::amendment::create_rate_addon;
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit::{Grant, fund_grant};

fn jan(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
}

fn apr(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 4, day).unwrap()
}

/// During the day, as a person previewing a change is.
fn at(day: NaiveDate) -> DateTime<Utc> {
    day.and_hms_opt(9, 0, 0).unwrap().and_utc()
}

/// Starter from `start`: Platform Fee 29.00 and one seat at 10.00, a month in advance. Its first
/// invoice is finalized and left unpaid, so what a change takes back cancels debt.
async fn starter_from(env: &TestEnv, start: NaiveDate) -> SubscriptionId {
    env.set_now(at(start));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    sub_id
}

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// The rows a change writes, per table: the subscription's documents, facts and events, and the
/// customer's credit notes and ledger.
async fn written(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(&'static str, i64)> {
    use diesel_async::RunQueryDsl;

    let customer = env.get_subscription(sub_id).await.customer_id.as_uuid();
    let sub = sub_id.as_uuid();
    let tables = [
        (
            "invoice",
            format!("FROM invoice WHERE subscription_id = '{sub}'"),
        ),
        (
            "credit_note",
            format!("FROM credit_note WHERE customer_id = '{customer}'"),
        ),
        (
            "subscription_component",
            format!("FROM subscription_component WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_add_on",
            format!("FROM subscription_add_on WHERE subscription_id = '{sub}'"),
        ),
        (
            "slot_transaction",
            format!("FROM slot_transaction WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_plan_window",
            format!("FROM subscription_plan_window WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_trial",
            format!("FROM subscription_trial WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_adjustment",
            format!("FROM subscription_adjustment WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_event",
            format!("FROM subscription_event WHERE subscription_id = '{sub}'"),
        ),
        (
            "subscription_change",
            format!("FROM subscription_change WHERE subscription_id = '{sub}'"),
        ),
        (
            "customer_credit_grant",
            format!("FROM customer_credit_grant WHERE customer_id = '{customer}'"),
        ),
        (
            "customer_credit_entry",
            format!("FROM customer_credit_entry WHERE customer_id = '{customer}'"),
        ),
    ];
    let mut conn = env.conn().await;
    let mut counts = Vec::with_capacity(tables.len());
    for (table, from) in tables {
        let rows: Vec<Count> = diesel::sql_query(format!("SELECT count(*) AS count {from}"))
            .load(&mut conn)
            .await
            .expect(table);
        counts.push((table, rows[0].count));
    }
    counts
}

/// Each line's subtotal, smallest first.
fn amounts(lines: &[meteroid_store::domain::LineItem]) -> Vec<i64> {
    let mut amounts: Vec<i64> = lines.iter().map(|line| line.amount_subtotal).collect();
    amounts.sort();
    amounts
}

fn totals(content: &ComputedInvoiceContent) -> (i64, i64, i64, i64) {
    (
        content.subtotal,
        content.total,
        content.applied_credits,
        content.amount_due,
    )
}

async fn invoice(env: &TestEnv, sub_id: SubscriptionId, id: InvoiceId) -> Invoice {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.id == id)
        .expect("the invoice")
}

/// The subtotals of the lines of the notes `ids`, smallest first.
async fn noted(env: &TestEnv, sub_id: SubscriptionId, ids: &[CreditNoteId]) -> Vec<i64> {
    let mut credited = Vec::new();
    for invoice in env.get_invoices(sub_id).await {
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
            .into_iter()
            .filter(|note| ids.contains(&note.id))
        {
            credited.extend(note.line_items.iter().map(|line| line.amount_subtotal));
        }
    }
    credited.sort();
    credited
}

fn add_on_amendment(add_on: common_domain::ids::AddOnId) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id: add_on,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
            edited: vec![],
            removed: vec![],
        },
    }
}

/// Starter changed to Pro (99.00 and a seat at 25.00) on Jan 16, 16 of 31 days left: the quote and
/// the summary are what the change issues, Pro's stubs billed, Starter's unused days noted.
/// Rules: R-079, R-080
#[rstest]
#[tokio::test]
async fn a_plan_change_preview_is_what_the_change_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from(&env, jan(1)).await;
    env.set_now(at(jan(16)));

    let before = written(&env, sub_id).await;
    let quote = env
        .services()
        .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, jan(16))
        .await
        .expect("the quote");
    let preview = env
        .services()
        .preview_plan_change(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            Some(PlanChangeMode::Immediate),
        )
        .await
        .expect("the preview");
    assert_eq!(
        written(&env, sub_id).await,
        before,
        "a preview writes nothing"
    );

    // Pro's seat: 2500 × 16/31 = 1290.32 → 1290; Pro's Platform Fee: 9900 × 16/31 = 5109.68 → 5110.
    assert_eq!(amounts(&quote.invoice_lines), vec![1_290, 5_110]);
    // 1290 + 5110 = 6400, untaxed; the note cancels debt, so nothing lands on the balance to draw.
    assert_eq!(totals(&quote), (6_400, 6_400, 0, 6_400));
    // Starter's unused days back: 1000 × 16/31 = 516.13 → 516 and 2900 × 16/31 = 1496.77 → 1497,
    // 2013 in all; net 6400 − 2013 = 4387.
    let summary = preview.proration.expect("an immediate change's summary");
    assert_eq!(
        (
            summary.charges_total_cents,
            summary.credits_total_cents,
            summary.net_amount_cents
        ),
        (6_400, -2_013, 4_387)
    );

    let applied = env
        .services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            jan(16),
        )
        .await
        .expect("the change");
    let adjustment = invoice(
        &env,
        sub_id,
        applied
            .adjustment_invoice_id
            .expect("an adjustment invoice"),
    )
    .await;
    assert_eq!(amounts(&adjustment.line_items), vec![1_290, 5_110]);
    assert_eq!(
        (adjustment.subtotal, adjustment.total, adjustment.amount_due),
        (6_400, 6_400, 6_400)
    );
    assert_eq!(
        noted(&env, sub_id, &applied.credit_note_ids).await,
        vec![-1_497, -516]
    );
}

/// A 14-day free trial from Jan 1 changed to Starter on Jan 8 ends there: billing starts that
/// day, Platform Fee 2900 and a 1000 seat over Jan 8–Feb 8 on the opening invoice, 3900. The
/// preview names that invoice and charges what it bills; no adjustment is written.
/// Rules: R-011
#[rstest]
#[tokio::test]
async fn a_plan_change_in_a_free_trial_previews_its_opening_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(jan(1)));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(jan(1))
        .on_start()
        .trial_days(14)
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    env.set_now(at(jan(8)));
    let preview = env
        .services()
        .preview_plan_change(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![],
            Some(PlanChangeMode::Immediate),
        )
        .await
        .expect("the preview");
    let summary = preview.proration.expect("an immediate change's summary");
    assert_eq!(
        (
            summary.document,
            summary.charges_total_cents,
            summary.credits_total_cents
        ),
        (ChangeDocument::OpeningInvoice, 3_900, 0)
    );

    let applied = env
        .services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![],
            jan(8),
        )
        .await
        .expect("the change");
    assert_eq!(applied.adjustment_invoice_id, None);
    let opening = invoice(
        &env,
        sub_id,
        applied.first_invoice_id.expect("the opening invoice"),
    )
    .await;
    assert_eq!(opening.invoice_type, InvoiceType::Recurring);
    assert_eq!(
        (opening.billing_period_start, opening.subtotal),
        (jan(8), 3_900)
    );
}

/// A 20.00 add-on added on Jan 16 (16 of 31 days left), then removed on Jan 20 (12 left). Each
/// preview's adjustment invoice and credit note are the documents the amendment then issues.
/// Rules: R-079, R-080
#[rstest]
#[tokio::test]
async fn an_add_on_amendment_preview_is_what_the_amendment_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from(&env, jan(1)).await;
    let add_on = create_rate_addon(&env, "Premium Support", 2_000).await;

    env.set_now(at(jan(16)));
    let before = written(&env, sub_id).await;
    let added = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, add_on_amendment(add_on))
        .await
        .expect("the preview of the addition");
    assert_eq!(
        written(&env, sub_id).await,
        before,
        "a preview writes nothing"
    );
    let adjustment = added.adjustment_invoice.expect("an adjustment invoice");
    // 2000 × 16/31 = 1032.26 → 1032, untaxed and unpaid by credit.
    assert_eq!(amounts(&adjustment.invoice_lines), vec![1_032]);
    assert_eq!(totals(&adjustment), (1_032, 1_032, 0, 1_032));
    assert!(added.credit_note.is_none(), "adding takes nothing back");
    let summary = added.proration.expect("an immediate amendment's summary");
    assert_eq!(
        (summary.charges_total_cents, summary.credits_total_cents),
        (1_032, 0)
    );

    let applied = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add_on_amendment(add_on),
            jan(16),
        )
        .await
        .expect("the addition");
    let billed = invoice(
        &env,
        sub_id,
        applied
            .adjustment_invoice_id
            .expect("an adjustment invoice"),
    )
    .await;
    assert_eq!(amounts(&billed.line_items), vec![1_032]);
    assert_eq!((billed.subtotal, billed.total), (1_032, 1_032));

    env.set_now(at(jan(20)));
    let active = {
        let mut conn = env.conn().await;
        diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_by_subscription_id_active(
            &mut conn, &TENANT_ID, &sub_id, None,
        )
        .await
        .expect("active add-ons")
    };
    let removal = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![],
            edited: vec![],
            removed: vec![active[0].id],
        },
    };
    let before = written(&env, sub_id).await;
    let removed = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, removal.clone())
        .await
        .expect("the preview of the removal");
    assert_eq!(
        written(&env, sub_id).await,
        before,
        "a preview writes nothing"
    );
    assert!(
        removed.adjustment_invoice.is_none(),
        "removing bills nothing"
    );
    // Its days to Jan 20 owe 1032 − 2000 × 12/31 (774.19 → 774) = 258 of the 1032 billed: 774 back.
    let note = removed.credit_note.expect("a credit note");
    assert_eq!(amounts(&note.invoice_lines), vec![-774]);
    assert_eq!((note.subtotal, note.total), (-774, -774));
    let summary = removed.proration.expect("an immediate amendment's summary");
    assert_eq!(
        (summary.charges_total_cents, summary.credits_total_cents),
        (0, -774)
    );

    let applied = env
        .services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, removal, jan(20))
        .await
        .expect("the removal");
    assert_eq!(applied.adjustment_invoice_id, None);
    assert_eq!(
        noted(&env, sub_id, &applied.credit_note_ids).await,
        vec![-774]
    );
}

/// Three seats added on Apr 16 to Starter's one, 15 of April's 30 days left: the preview bills
/// what the seat purchase's invoice bills.
/// Rules: R-079
#[rstest]
#[tokio::test]
async fn a_seat_preview_is_what_the_seat_purchase_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from(&env, apr(1)).await;
    env.set_now(at(apr(16)));

    let before = written(&env, sub_id).await;
    let preview = env
        .services()
        .preview_slot_update(TENANT_ID, sub_id, COMP_STARTER_SEATS_ID, 3)
        .await
        .expect("the preview");
    assert_eq!(
        written(&env, sub_id).await,
        before,
        "a preview writes nothing"
    );
    assert_eq!((preview.current_slots, preview.new_slots), (1, 4));
    assert_eq!((preview.days_remaining, preview.days_total), (15, 30));
    assert_eq!(
        preview.current_period_end,
        NaiveDate::from_ymd_opt(2024, 5, 1).unwrap()
    );
    // Four seats from Apr 16 less the one billed: 4000 × 15/30 − 1000 × 15/30 = 2000 − 500 = 1500.
    assert_eq!(preview.prorated_amount, dec!(15.00));
    assert_eq!(preview.full_period_amount, dec!(30.00));
    let adjustment = preview
        .adjustment_invoice
        .expect("an invoice for the seats");
    assert_eq!(amounts(&adjustment.invoice_lines), vec![1_500]);
    assert_eq!(totals(&adjustment), (1_500, 1_500, 0, 1_500));

    let applied = env
        .services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            3,
            SlotUpgradeBillingMode::Optimistic,
            Some(apr(16).and_hms_opt(9, 0, 0).unwrap()),
        )
        .await
        .expect("the seats");
    let billed = invoice(
        &env,
        sub_id,
        applied.invoice_id.expect("the seats' invoice"),
    )
    .await;
    assert_eq!(amounts(&billed.line_items), vec![1_500]);
    assert_eq!((billed.subtotal, billed.total), (1_500, 1_500));
}

/// Three seats added on Apr 17 to a French consumer's Starter seat, 14 of April's 30 days left:
/// 4000 × 14/30 (1866.67 → 1867) − 1000 × 14/30 (466.67 → 467) = 1400, 4.66666667 a seat, and
/// 20% VAT, 280. The purchase is issued as the change's adjustment invoice, its line naming the
/// seats fee, and its preview is that invoice to the cent.
/// Rules: R-075, R-079
#[rstest]
#[tokio::test]
async fn a_seat_purchase_is_issued_as_its_preview_quotes_it(#[future] test_env: TestEnv) {
    use meteroid_store::domain::{InvoiceStatusEnum, LineItem};
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let env = test_env.await;
    crate::test_mrr_tax_inclusive::enable_eu_vat(&env).await;
    let customer_id = crate::test_mrr_tax_inclusive::create_french_consumer(&env).await;
    env.set_now(at(apr(1)));
    let sub_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(apr(1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let seats_fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .into_iter()
        .find(|component| component.price_component_id == Some(COMP_STARTER_SEATS_ID))
        .expect("the seats fee")
        .id;
    env.set_now(at(apr(17)));

    let preview = env
        .services()
        .preview_slot_update(TENANT_ID, sub_id, COMP_STARTER_SEATS_ID, 3)
        .await
        .expect("the preview");
    assert_eq!((preview.days_remaining, preview.days_total), (14, 30));
    assert_eq!(preview.prorated_amount, dec!(14.00));
    let previewed = preview
        .adjustment_invoice
        .expect("an invoice for the seats");

    let applied = env
        .services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            3,
            SlotUpgradeBillingMode::Optimistic,
            Some(apr(17).and_hms_opt(9, 0, 0).unwrap()),
        )
        .await
        .expect("the seats");
    assert_eq!(applied.prorated_amount, Some(dec!(14.00)));
    let billed = invoice(
        &env,
        sub_id,
        applied.invoice_id.expect("the seats' invoice"),
    )
    .await;
    assert_eq!(
        (billed.invoice_type, billed.status, billed.invoice_date),
        (
            InvoiceType::Adjustment,
            InvoiceStatusEnum::Finalized,
            apr(17)
        )
    );

    let shown = |line: &LineItem| {
        (
            line.name.clone(),
            line.description.clone(),
            line.quantity,
            line.unit_price,
            (line.start_date, line.end_date),
            line.is_prorated,
            line.sub_component_id,
            (line.amount_subtotal, line.tax_rate, line.tax_amount),
            line.amount_total,
        )
    };
    let line = (
        "3 Seats upgrade".to_owned(),
        Some("Prorated charge for 3 additional Seats".to_owned()),
        Some(dec!(3)),
        Some(dec!(4.66666667)),
        (apr(17), NaiveDate::from_ymd_opt(2024, 5, 1).unwrap()),
        true,
        Some(seats_fee),
        (1_400, dec!(0.2), 280),
        1_680,
    );
    assert_eq!(
        billed.line_items.iter().map(shown).collect::<Vec<_>>(),
        vec![line.clone()]
    );
    assert_eq!(
        previewed
            .invoice_lines
            .iter()
            .map(shown)
            .collect::<Vec<_>>(),
        vec![line]
    );
    assert_eq!(
        (
            billed.subtotal,
            billed.discount,
            billed.tax_amount,
            billed.total,
            billed.amount_due
        ),
        (1_400, 0, 280, 1_680, 1_680)
    );
    assert_eq!(
        (
            previewed.subtotal,
            previewed.discount,
            previewed.tax_amount,
            previewed.total,
            previewed.amount_due
        ),
        (1_400, 0, 280, 1_680, 1_680)
    );
}

/// A 20.00 add-on bought through the portal on Apr 16, 15 of April's 30 days left: the quote is
/// what the checkout charges and the purchase invoices.
/// Rules: R-079
#[rstest]
#[tokio::test]
async fn an_add_on_purchase_quote_is_what_the_purchase_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub_id = starter_from(&env, apr(1)).await;
    let add_on = create_rate_addon(&env, "Premium Support", 2_000).await;
    env.set_now(at(apr(16)));
    let customer_id = env.get_subscription(sub_id).await.customer_id;
    let session = env
        .services()
        .create_addon_purchase_checkout_session(
            TENANT_ID,
            sub_id,
            add_on,
            PLAN_VERSION_STARTER_ID,
            customer_id,
            None,
        )
        .await
        .expect("the checkout session");

    let before = written(&env, sub_id).await;
    let add_ons = session.add_ons.as_ref().expect("the add-ons bought");
    let quote = env
        .services()
        .compute_addon_purchase_checkout_invoice(TENANT_ID, sub_id, &add_ons.add_ons, apr(16))
        .await
        .expect("the quote")
        .0;
    assert_eq!(
        written(&env, sub_id).await,
        before,
        "a quote writes nothing"
    );
    // 2000 × 15/30 = 1000.
    assert_eq!(amounts(&quote.invoice_lines), vec![1_000]);
    assert_eq!(totals(&quote), (1_000, 1_000, 0, 1_000));

    let completed = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            quote.amount_due as u64,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout takes the quoted amount");
    assert!(
        matches!(completed, CheckoutCompletionResult::Completed { .. }),
        "{completed:?}"
    );
    let bought = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.line_items.iter().any(|l| l.sub_add_on_id.is_some()))
        .expect("the purchase's invoice");
    assert_eq!(amounts(&bought.line_items), vec![1_000]);
    assert_eq!((bought.subtotal, bought.total), (1_000, 1_000));
}

/// The same 20.00 add-on bought on Apr 16 with a charge that settles later: its row is written
/// once, by the settlement. The purchase bills 10.00, May's renewal 20.00 for it, once.
/// Rules: R-079, R-080
#[rstest]
#[tokio::test]
async fn an_add_on_paid_after_its_checkout_is_billed_once(#[future] test_env: TestEnv) {
    use meteroid_store::domain::connectors::MockChargeBehavior;

    let env = test_env.await;
    env.seed_payments().await;
    let sub_id = starter_from(&env, apr(1)).await;
    let add_on = create_rate_addon(&env, "Premium Support", 2_000).await;
    env.set_now(at(apr(16)));
    let customer_id = env.get_subscription(sub_id).await.customer_id;
    let session = env
        .services()
        .create_addon_purchase_checkout_session(
            TENANT_ID,
            sub_id,
            add_on,
            PLAN_VERSION_STARTER_ID,
            customer_id,
            None,
        )
        .await
        .expect("the checkout session");
    let add_ons = session.add_ons.as_ref().expect("the add-ons bought");
    let quote = env
        .services()
        .compute_addon_purchase_checkout_invoice(TENANT_ID, sub_id, &add_ons.add_ons, apr(16))
        .await
        .expect("the quote")
        .0;
    assert_eq!(quote.amount_due, 1_000);

    let before = written(&env, sub_id).await;
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: pending,
        ..
    } = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            quote.amount_due as u64,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout")
    else {
        panic!("the payment is pending")
    };
    let after: Vec<_> = written(&env, sub_id)
        .await
        .into_iter()
        .filter(|(table, _)| *table == "subscription_add_on")
        .collect();
    let rows_before: Vec<_> = before
        .into_iter()
        .filter(|(table, _)| *table == "subscription_add_on")
        .collect();
    assert_eq!(after, rows_before, "an unsettled purchase writes no row");

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let payload = format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{ext}","amount":{amount},"currency":"EUR"}}"#,
        tx = pending.id.as_base62(),
        ext = pending
            .provider_transaction_id
            .clone()
            .expect("the provider's id"),
        amount = quote.amount_due,
    );
    env.deliver_mock_webhook(payload.as_bytes()).await;

    let bought: Vec<Invoice> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.invoice_type == InvoiceType::Adjustment)
        .collect();
    assert_eq!(bought.len(), 1, "one purchase invoice");
    assert_eq!(amounts(&bought[0].line_items), vec![1_000]);
    assert_eq!((bought[0].subtotal, bought[0].total), (1_000, 1_000));

    env.set_now(at(NaiveDate::from_ymd_opt(2024, 5, 1).unwrap()));
    env.process_cycles().await;
    let may = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.invoice_type == InvoiceType::Recurring
                && invoice.invoice_date == NaiveDate::from_ymd_opt(2024, 5, 1).unwrap()
        })
        .expect("May's renewal");
    // Seat 10.00, add-on 20.00, platform fee 29.00.
    assert_eq!(amounts(&may.line_items), vec![1_000, 2_000, 2_900]);
    assert_eq!(may.subtotal, 5_900);
}

/// (total, applied credits, amount due) as the portal quotes the checkout.
async fn portal_quote(env: &TestEnv, session: CheckoutSessionId) -> (u64, u64, u64) {
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &env.setup.config.jwt_secret,
        TENANT_ID,
        meteroid_store::jwt_claims::ResourceAccess::CheckoutSession(session),
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut client = portal::portal_checkout_service_client::PortalCheckoutServiceClient::new(
        env.setup.channel.clone(),
    );
    let mut request = tonic::Request::new(portal::GetCheckoutRequest { coupon_code: None });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    let checkout = client
        .get_checkout(request)
        .await
        .expect("checkout page")
        .into_inner()
        .checkout
        .expect("checkout");
    (
        checkout.total_amount,
        checkout.applied_credits,
        checkout.amount_due,
    )
}

/// A self-serve checkout of LeetCode (35.00 a month from Jan 15) with a 10 % code and 10.00 of
/// credit: the first invoice priced from the facts it would record is what the checkout bills.
/// Rules: R-042
#[rstest]
#[tokio::test]
async fn a_new_subscription_checkout_quote_is_what_its_first_invoice_bills(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_coupon(CouponNew {
        code: "WELCOME10".to_owned(),
        description: "10 % off".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Percentage(dec!(10)),
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    })
    .await;
    fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(1_000),
    )
    .await;
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            resource_id: None,
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            option: None,
            billing_start_date: Some(jan(15)),
            billing_day_anchor: Some(15),
            net_terms: None,
            trial: meteroid_store::domain::trials::TrialRequest::None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: Some("WELCOME10".to_owned()),
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("the checkout session");

    let details = env
        .services()
        .build_preview_subscription_details(&session, TENANT_ID, None)
        .await
        .expect("the subscription the session creates");
    let quote = env
        .services()
        .compute_new_subscription_invoice(&details)
        .await
        .expect("its first invoice");
    // One line, a whole month from Jan 15: 3500.
    assert_eq!(amounts(&quote.invoice_lines), vec![3_500]);
    // 10 % off: 350, so 3150; the 1000 of credit pays part of it: 2150 due.
    assert_eq!(quote.discount, 350);
    assert_eq!(totals(&quote), (3_500, 3_150, 1_000, 2_150));
    assert_eq!(portal_quote(&env, session.id).await, (3_150, 1_000, 2_150));

    let completed = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            2_150,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout takes the quoted amount");
    let CheckoutCompletionResult::Completed {
        subscription_id,
        transaction: Some(transaction),
    } = completed
    else {
        panic!("a settled charge completes the checkout: {completed:?}")
    };
    assert_eq!(transaction.amount, 2_150);
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let first = &invoices[0];
    assert_eq!(first.invoice_type, InvoiceType::Recurring);
    assert_eq!(amounts(&first.line_items), vec![3_500]);
    // Paid: 1000 of credit and the 2150 charged.
    assert_eq!(
        (
            first.subtotal,
            first.discount,
            first.total,
            first.applied_credits,
            first.amount_due
        ),
        (3_500, 350, 3_150, 1_000, 0)
    );
}

/// Starter with a 31.00 add-on bought on Jan 1 and paid, swapped on Jan 11 for a 62.00 one: the
/// note gives the paid 3100 × 21/31 = 2100 back to the balance, which pays as much of the
/// adjustment, 6200 × 21/31 = 4200: 2100 due, as previewed, and charged.
/// Rules: R-078
#[rstest]
#[tokio::test]
async fn a_swap_on_a_paid_invoice_charges_what_its_preview_says_is_due(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::enums::InvoicePaymentStatus;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(jan(1)));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan(1))
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    let basic = create_rate_addon(&env, "Basic Support", 3_100).await;
    let premium = create_rate_addon(&env, "Premium Support", 6_200).await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add_on_amendment(basic),
            jan(1),
        )
        .await
        .expect("the 31.00 add-on");
    env.run_outbox_and_orchestration().await;
    let paid = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .any(|l| l.name.starts_with("Basic Support"))
        })
        .expect("the invoice billing the add-on");
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    // Bought on the period's first day, it bills the whole period: no "(prorated)".
    assert_eq!(
        paid.line_items
            .iter()
            .map(|line| (line.name.as_str(), line.is_prorated, line.amount_subtotal))
            .collect::<Vec<_>>(),
        vec![("Basic Support", false, 3_100)]
    );

    env.set_now(at(jan(11)));
    let active = {
        let mut conn = env.conn().await;
        diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_by_subscription_id_active(
            &mut conn, &TENANT_ID, &sub_id, None,
        )
        .await
        .expect("active add-ons")
    };
    let mut swap = add_on_amendment(premium);
    swap.add_on_changes.removed = vec![active[0].id];
    let preview = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, swap.clone())
        .await
        .expect("the preview of the swap");
    let quoted = preview.adjustment_invoice.expect("an adjustment invoice");
    assert_eq!(amounts(&quoted.invoice_lines), vec![4_200]);
    assert_eq!(totals(&quoted), (4_200, 4_200, 2_100, 2_100));
    let note = preview.credit_note.expect("a credit note");
    assert_eq!(amounts(&note.invoice_lines), vec![-2_100]);

    let applied = env
        .services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, swap, jan(11))
        .await
        .expect("the swap");
    let charged = invoice(
        &env,
        sub_id,
        applied
            .adjustment_invoice_id
            .expect("an adjustment invoice"),
    )
    .await;
    assert_eq!(
        (
            charged.subtotal,
            charged.total,
            charged.applied_credits,
            charged.amount_due
        ),
        (4_200, 4_200, 2_100, 2_100)
    );
    assert_eq!(
        noted(&env, sub_id, &applied.credit_note_ids).await,
        vec![-2_100]
    );
}

/// Starter with a 31.00 add-on bought on Jan 1, 25.00 of its invoice paid by credit and 6.00
/// left due, swapped on Jan 11 for a 62.00 one: the note of 3100 × 21/31 = 2100 cancels the 600
/// due and returns 1500 to the balance, which pays as much of the 4200 adjustment: 2700 due, as
/// previewed, where the preview counted nothing back and said 4200.
/// Rules: R-078
#[rstest]
#[tokio::test]
async fn a_swap_on_an_invoice_paid_by_credit_previews_what_its_note_returns(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(jan(1)));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(jan(1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(2_500),
    )
    .await;
    let basic = create_rate_addon(&env, "Basic Support", 3_100).await;
    let premium = create_rate_addon(&env, "Premium Support", 6_200).await;
    let bought = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add_on_amendment(basic),
            jan(1),
        )
        .await
        .expect("the 31.00 add-on");
    let bought = invoice(
        &env,
        sub_id,
        bought.adjustment_invoice_id.expect("the add-on's invoice"),
    )
    .await;
    assert_eq!(
        (bought.total, bought.applied_credits, bought.amount_due),
        (3_100, 2_500, 600)
    );

    env.set_now(at(jan(11)));
    let active = {
        let mut conn = env.conn().await;
        diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_by_subscription_id_active(
            &mut conn, &TENANT_ID, &sub_id, None,
        )
        .await
        .expect("active add-ons")
    };
    let mut swap = add_on_amendment(premium);
    swap.add_on_changes.removed = vec![active[0].id];
    let preview = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, swap.clone())
        .await
        .expect("the preview of the swap");
    let quoted = preview.adjustment_invoice.expect("an adjustment invoice");
    assert_eq!(totals(&quoted), (4_200, 4_200, 1_500, 2_700));

    let applied = env
        .services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, swap, jan(11))
        .await
        .expect("the swap");
    let charged = invoice(
        &env,
        sub_id,
        applied
            .adjustment_invoice_id
            .expect("an adjustment invoice"),
    )
    .await;
    assert_eq!(
        (
            charged.subtotal,
            charged.total,
            charged.applied_credits,
            charged.amount_due
        ),
        (4_200, 4_200, 1_500, 2_700)
    );
}

/// A 31.00 fee on January's invoice, still a draft, lowered to 15.00 on Jan 11: a draft is never
/// credited, so nothing is then. Issued later, it gets the note the amendment owes: January owes
/// 3100 − 3100 × 21/31 (2100) = 1000 and 1500 × 21/31 = 1016, 2016 of the 3100: 1084 back. A
/// sweep credits nothing more.
/// Rules: R-077, R-078
#[rstest]
#[tokio::test]
async fn an_amendment_while_the_period_invoice_is_a_draft_credits_it_once_issued(
    #[future] test_env: TestEnv,
) {
    use crate::data::plans::{PlanSeed, SeedComp};
    use common_domain::ids::{PlanId, PlanVersionId, PriceComponentId, PriceId};
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::EditComponent;

    let env = test_env.await;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Thirty-one", version)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            diesel_models::enums::BillingPeriodEnum::Monthly,
            dec!(31),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    env.set_now(at(jan(1)));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(jan(1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let january = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.invoice_type == InvoiceType::Recurring)
        .expect("January's invoice");
    assert_eq!(
        january.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );

    let platform = env.get_subscription_components(sub_id).await[0].id;
    env.set_now(at(jan(11)));
    let lowered = |platform| SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: platform,
                name: None,
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Rate { rate: dec!(15) },
                    credit_unit: None,
                }),
            }],
            added: vec![],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    };
    // The preview issues what the amendment does now: nothing taken back of a draft.
    let preview = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, lowered(platform))
        .await
        .expect("the preview");
    assert!(preview.credit_note.is_none());
    let summary = preview.proration.expect("a summary");
    assert_eq!(
        (summary.charges_total_cents, summary.credits_total_cents),
        (0, 0)
    );
    env.services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, lowered(platform), jan(11))
        .await
        .expect("the fee lowered");
    let notes_on = |env: &TestEnv| {
        let store = env.store().clone();
        async move {
            store
                .list_credit_notes_by_invoice_id(TENANT_ID, january.id)
                .await
                .expect("notes")
                .into_iter()
                .flat_map(|note| note.line_items.into_iter().map(|line| line.amount_subtotal))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        notes_on(&env).await,
        Vec::<i64>::new(),
        "a draft is never credited"
    );

    env.manual_finalize_invoice(january.id)
        .await
        .expect("finalized");
    assert_eq!(notes_on(&env).await, vec![-1_084]);
    env.services()
        .settle_missed_plan_change_endings(100)
        .await
        .expect("sweep");
    assert_eq!(notes_on(&env).await, vec![-1_084]);
}

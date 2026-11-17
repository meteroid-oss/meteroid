//! A voided recurring invoice is terminal (FOLLOWUPS 140): the lifecycle never bills its period
//! again nor credits it back; only an explicit regeneration bills the period again.
use chrono::NaiveDate;

use common_domain::actor::Actor;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::repositories::credit_notes::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

/// The Starter plan bills its €29 platform fee and seats for January in advance, at creation.
async fn starter_with_voided_first_period(env: &TestEnv) -> common_domain::ids::SubscriptionId {
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let invoices = env.get_invoices(id).await;
    assert_eq!(invoices.len(), 1);
    assert!(invoices[0].total > 0);
    env.try_void_invoice(invoices[0].id)
        .await
        .expect("void the unpaid period invoice");
    id
}

/// Lago skips the termination credit of a voided period: nothing was billed, nothing comes back.
#[tokio::test]
async fn cancelling_mid_period_after_a_void_credits_nothing_back() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].id;

    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 16)),
        )
        .await
        .expect("cancel mid-period");
    env.process_due_events().await;
    env.process_due_events().await;

    let notes = env
        .store()
        .list_credit_notes_by_customer_id(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("credit notes");
    assert_eq!(
        notes.iter().map(|n| n.invoice_id).collect::<Vec<_>>(),
        vec![voided],
        "only the void's own note"
    );
    let billed: i64 = env
        .get_invoices(id)
        .await
        .iter()
        .filter(|i| i.status != InvoiceStatusEnum::Void)
        .map(|i| i.total)
        .sum();
    assert_eq!(billed, 0, "January is not billed again");
}

async fn mrr_movements(env: &TestEnv) -> i64 {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::tenant_id.eq(TENANT_ID))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("MRR movements")
}

/// Regeneration bills the voided period again on current data, as the period's own invoice, and
/// books no MRR movement a second time (the voided invoice booked them).
#[tokio::test]
async fn regenerating_a_voided_invoice_bills_its_period_once() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].clone();
    let movements = mrr_movements(&env).await;
    let mrr = env.get_subscription(id).await.mrr_cents;

    let regenerated = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided.id)
        .await
        .expect("regenerate");
    env.process_due_events().await;

    let invoices = env.get_invoices(id).await;
    assert_eq!(invoices.len(), 2);
    let rebill = invoices
        .iter()
        .find(|i| i.id == regenerated.invoice.id)
        .expect("the regenerated invoice");
    assert_eq!(rebill.status, InvoiceStatusEnum::Finalized);
    assert_eq!(rebill.billing_period_start, voided.billing_period_start);
    assert_eq!(rebill.cycle_index, voided.cycle_index);
    assert_eq!(rebill.total, voided.total);
    assert_eq!(mrr_movements(&env).await, movements, "no MRR booked twice");
    assert_eq!(env.get_subscription(id).await.mrr_cents, mrr);

    let again = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided.id)
        .await
        .expect_err("the period has its invoice again");
    assert!(
        format!("{again:?}").contains("already has invoice"),
        "{again:?}"
    );
    assert_eq!(env.get_invoices(id).await.len(), 2);
}

/// Only a voided invoice of the period the subscription is in can be regenerated (Chargebee
/// `regenerate_invoice`); a past period is billed with a one-off invoice.
#[tokio::test]
async fn regeneration_is_refused_for_a_live_invoice_or_a_past_period() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env.get_invoices(id).await[0].id;
    let live = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, january)
        .await
        .expect_err("a live invoice is not regenerated");
    assert!(
        format!("{live:?}").contains("Only a voided invoice"),
        "{live:?}"
    );

    env.process_cycles().await;
    assert_eq!(env.get_invoices(id).await.len(), 2, "February is billed");
    env.try_void_invoice(january)
        .await
        .expect("void January's invoice");
    let past = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, january)
        .await
        .expect_err("a past period is not regenerated");
    assert!(
        format!("{past:?}").contains("current billing period"),
        "{past:?}"
    );
    assert_eq!(env.get_invoices(id).await.len(), 2);
}

/// €49 a month with a first-cycle 50% coupon: the period invoice is 24.50.
async fn coupon_once_subscription(env: &TestEnv) -> common_domain::ids::SubscriptionId {
    let coupon = env
        .create_limited_percentage_coupon("FIRSTHALF", 50, Some(1))
        .await;
    let id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .coupon(coupon)
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let invoices = env.get_invoices(id).await;
    assert_eq!(
        invoices
            .iter()
            .map(|i| (i.total, i.discount))
            .collect::<Vec<_>>(),
        vec![(2_450, 2_450)]
    );
    id
}

/// A void gives back the coupon consumption its invoice recorded (Stripe): the regenerated
/// period is discounted as the voided invoice was.
#[tokio::test]
async fn a_regenerated_invoice_keeps_a_first_cycle_coupon() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = coupon_once_subscription(&env).await;
    let voided = env.get_invoices(id).await[0].id;
    env.try_void_invoice(voided).await.expect("void");

    let regenerated = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided)
        .await
        .expect("regenerate");
    env.process_due_events().await;
    let rebill = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|i| i.id == regenerated.invoice.id)
        .expect("the regenerated invoice");
    assert_eq!(rebill.status, InvoiceStatusEnum::Finalized);
    assert_eq!((rebill.total, rebill.discount), (2_450, 2_450));

    env.process_cycles().await;
    let february = env.get_invoices(id).await;
    assert_eq!(february.len(), 3);
    assert_eq!(
        february.iter().map(|i| i.total).max(),
        Some(4_900),
        "the coupon was consumed once, by the regenerated invoice"
    );
}

/// Voided and never regenerated, the first cycle consumed nothing of a first-cycle coupon: the
/// next cycle is discounted, the one after is not.
#[tokio::test]
async fn a_voided_invoice_leaves_its_coupon_to_the_next_cycle() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = coupon_once_subscription(&env).await;
    let voided = env.get_invoices(id).await[0].id;
    env.try_void_invoice(voided).await.expect("void");

    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;
    let totals = |invoices: Vec<meteroid_store::domain::Invoice>| {
        invoices
            .into_iter()
            .filter(|i| i.id != voided)
            .map(|i| i.total)
            .collect::<Vec<_>>()
    };
    assert_eq!(totals(env.get_invoices(id).await), vec![2_450]);

    env.process_cycles().await;
    assert_eq!(totals(env.get_invoices(id).await), vec![2_450, 4_900]);
}

/// The period changed after its invoice was voided (an add-on bought mid-period, charged on its
/// own adjustment invoice): regenerating would bill the new state for the whole period.
#[tokio::test]
async fn regeneration_is_refused_after_a_change_within_the_period() {
    use meteroid_store::domain::subscription_add_ons::{
        CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
    };
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].id;
    let add_on = super::amendment::create_rate_addon(&env, "Priority support", 10_000).await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            id,
            TENANT_ID,
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
            },
            day(1, 16),
        )
        .await
        .expect("upgrade mid-period");
    let before = env.get_invoices(id).await.len();

    let refused = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided)
        .await
        .expect_err("the period changed");
    assert!(
        format!("{refused:?}").contains("one-off invoice"),
        "{refused:?}"
    );
    assert_eq!(env.get_invoices(id).await.len(), before);
}

/// Cancelled on its renewal day after the renewal was invoiced: the cancellation books the churn
/// itself, once.
#[tokio::test]
async fn cancelling_on_an_invoiced_renewal_day_churns_once() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::enums::MrrMovementType;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    assert_eq!(env.get_invoices(id).await.len(), 2, "February is invoiced");
    assert!(env.get_subscription(id).await.mrr_cents > 0);

    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(2, 1)),
        )
        .await
        .expect("cancel on the renewal day");
    env.process_due_events().await;

    let churns: i64 = m::bi_mrr_movement_log
        .filter(m::tenant_id.eq(TENANT_ID))
        .filter(m::movement_type.eq(MrrMovementType::Churn))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("churn movements");
    assert_eq!(churns, 1);
    assert_eq!(env.get_subscription(id).await.mrr_cents, 0);
    assert_eq!(env.get_invoices(id).await.len(), 2);
}

/// The cancellation is scheduled for the renewal day first, then a recurring invoice for the
/// period starting that day is inserted before the termination runs: the invoice books the churn
/// and the termination, finding the period billed, never books it a second time.
#[tokio::test]
async fn a_churn_booked_by_an_earlier_invoice_of_the_end_day_is_not_booked_again() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::enums::MrrMovementType;
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    use meteroid_store::repositories::InvoiceInterface;

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let mrr = env.get_subscription(id).await.mrr_cents;
    assert!(mrr > 0);
    // Scheduled ahead of its day, so the end day's invoice below is written before it applies.
    env.set_now(day(1, 20).and_hms_opt(12, 0, 0).unwrap().and_utc());
    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(2, 1)),
        )
        .await
        .expect("cancel on the renewal day");

    let january = env.get_invoices(id).await[0].clone();
    env.store()
        .insert_invoice(meteroid_store::domain::InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: january.tenant_id,
            customer_id: january.customer_id,
            subscription_id: Some(id),
            plan_version_id: january.plan_version_id,
            invoice_type: meteroid_store::domain::InvoiceType::Recurring,
            cycle_index: Some(1),
            currency: january.currency.clone(),
            line_items: vec![],
            coupons: vec![],
            data_updated_at: None,
            invoice_date: day(2, 1),
            billing_period_start: day(2, 1),
            tax_inclusive: false,
            finalized_at: None,
            subtotal: 0,
            subtotal_recurring: 0,
            discount: 0,
            tax_amount: 0,
            total: 0,
            amount_due: 0,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: january.net_terms,
            invoice_number: "draft".to_string(),
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: january.plan_name.clone(),
            customer_details: january.customer_details.clone(),
            seller_details: january.seller_details.clone(),
            auto_advance: false,
            payment_status: meteroid_store::domain::InvoicePaymentStatus::Unpaid,
            tax_breakdown: vec![],
            manual: false,
            invoicing_entity_id: january.invoicing_entity_id,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: vec![],
            custom_properties: serde_json::Value::Null,
        })
        .await
        .expect("the end day's recurring invoice");
    assert_eq!(
        env.get_subscription(id).await.mrr_cents,
        0,
        "the invoice booked the churn"
    );

    env.set_now(day(2, 1).and_hms_opt(1, 0, 0).unwrap().and_utc());
    env.process_cycles().await;
    env.process_due_events().await;

    let churns: Vec<i64> = m::bi_mrr_movement_log
        .filter(m::tenant_id.eq(TENANT_ID))
        .filter(m::movement_type.eq(MrrMovementType::Churn))
        .select(m::net_mrr_change)
        .load(&mut env.conn().await)
        .await
        .expect("churn movements");
    assert_eq!(churns, vec![-mrr], "the churn once");
    assert_eq!(env.get_subscription(id).await.mrr_cents, 0);
}

/// A regenerated invoice bills the period's charges under the ids the voided one gave them: a
/// line is identified by its charge, not by when it was computed (R-075).
#[tokio::test]
async fn a_regenerated_invoice_bills_its_charges_under_the_same_line_ids() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].clone();
    let ids = |invoice: &meteroid_store::domain::Invoice| {
        invoice
            .line_items
            .iter()
            .map(|line| line.local_id.clone())
            .collect::<Vec<_>>()
    };
    assert!(!voided.line_items.is_empty());

    let regenerated = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided.id)
        .await
        .expect("regenerate");

    assert_eq!(ids(&regenerated.invoice), ids(&voided));
}

/// Starter (29.00 fee, 10.00 seats) from Jan 1, a 31.00 fee in arrears added on Jan 1 and removed
/// on Jan 20: Feb 1 bills its 19 days, 3100 × 19/31 = 1900, with February's fees. That invoice is
/// voided: March bills March alone, the voided arrears forgiven, never billed again. Two seats
/// bought on Feb 16 bill what they add to February's seats, 1000 − 1000 × 14/29 (483) = 517 and
/// 3000 × 14/29 = 1448 owed, 1965 less the voided 1000: 965, never the voided seat again.
/// Rules: R-096
#[tokio::test]
async fn a_void_forgives_the_arrears_it_billed() {
    use meteroid_store::domain::enums::{BillingPeriodEnum, BillingType, FeeTypeEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let amend = |component_changes| SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes,
        add_on_changes: AddOnChanges::default(),
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            id,
            TENANT_ID,
            amend(ComponentChanges {
                edited: vec![],
                added: vec![AddExtraComponent {
                    name: "Overage Fee".to_string(),
                    product: ProductRef::New {
                        name: "Overage Fee Product".to_string(),
                        fee_type: FeeTypeEnum::ExtraRecurring,
                        fee_structure: FeeStructure::ExtraRecurring {
                            billing_type: BillingType::Arrears,
                        },
                    },
                    price_entry: PriceEntry::New(PriceInput {
                        cadence: BillingPeriodEnum::Monthly,
                        currency: "EUR".to_string(),
                        pricing: Pricing::ExtraRecurring {
                            unit_price: rust_decimal::Decimal::new(3100, 2),
                            quantity: 1,
                        },
                        credit_unit: None,
                    }),
                }],
                removed: vec![],
            }),
            day(1, 1),
        )
        .await
        .expect("add the arrears fee");
    let overage = env
        .store()
        .get_subscription_details(TENANT_ID, id)
        .await
        .expect("details")
        .price_components
        .into_iter()
        .find(|component| component.name == "Overage Fee")
        .expect("the arrears fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            id,
            TENANT_ID,
            amend(ComponentChanges {
                edited: vec![],
                added: vec![],
                removed: vec![overage],
            }),
            day(1, 20),
        )
        .await
        .expect("remove the arrears fee");

    env.process_cycles().await;
    let february = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2, 1))
        .expect("the Feb 1 invoice");
    env.try_void_invoice(february.id)
        .await
        .expect("void the Feb 1 invoice");
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            id,
            COMP_STARTER_SEATS_ID,
            2,
            meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
            Some(day(2, 16).and_time(chrono::NaiveTime::MIN)),
        )
        .await
        .expect("add 2 seats");
    env.process_cycles().await;

    let documents: Vec<String> = env
        .get_invoices(id)
        .await
        .iter()
        .flat_map(|invoice| {
            invoice.line_items.iter().map(move |line| {
                format!(
                    "{:?} {:?} {}: {} {}..{} {}",
                    invoice.status,
                    invoice.invoice_type,
                    invoice.billing_period_start,
                    line.name,
                    line.start_date,
                    line.end_date,
                    line.amount_subtotal
                )
            })
        })
        .collect();
    assert_eq!(
        documents,
        [
            "Finalized Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Finalized Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Void Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Void Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "Void Recurring 2024-02-01: Overage Fee 2024-01-01..2024-01-20 1900",
            "Finalized OneOff 2024-02-16: 2 Seats upgrade 2024-02-16..2024-03-01 965",
            "Finalized Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
            "Finalized Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 3000",
        ]
    );
}

/// January's invoice voided and regenerated, the regenerated one issued, then cancelled from
/// Jan 16: the regenerated invoice replaces the void, so its unused 16 of 31 days come back once,
/// on it, 2900 × 16/31 = 1497 and 1000 × 16/31 = 516; the voided invoice keeps its own note alone.
/// Rules: R-096, R-077
#[tokio::test]
async fn cancelling_a_regenerated_period_credits_its_unused_days_once() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].id;
    let regenerated = env
        .services()
        .regenerate_invoice(Actor::System, TENANT_ID, voided)
        .await
        .expect("regenerate")
        .invoice
        .id;
    env.process_due_events().await;
    let rebill = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|i| i.id == regenerated)
        .expect("the regenerated invoice");
    assert_eq!(rebill.status, InvoiceStatusEnum::Finalized);
    let voided_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, voided)
        .await
        .expect("the void's notes")
        .len();

    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 16)),
        )
        .await
        .expect("cancel mid-period");
    env.process_due_events().await;
    env.process_due_events().await;

    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, regenerated)
        .await
        .expect("credit notes");
    let mut lines: Vec<(String, i64)> = notes
        .iter()
        .flat_map(|note| note.line_items.iter())
        .map(|line| (line.name.clone(), line.amount_subtotal))
        .collect();
    lines.sort();
    assert_eq!(notes.len(), 1, "{lines:?}");
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_497),
            ("Seats".to_owned(), -516)
        ]
    );
    assert_eq!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, voided)
            .await
            .expect("the void's notes")
            .len(),
        voided_notes,
        "the voided invoice keeps its own note alone"
    );
}

/// Starter with January voided, five seats added from Jan 11 (4065 − 677 = 3388), cancelled
/// from Jan 17: of the 2903 owed back by unused time the voided seat's 484 is forgiven and the
/// added seats give back 2419, keeping 969; with the voided seat's 516, the 1485 owed.
/// Rules: R-096, R-077
#[tokio::test]
async fn a_void_forgives_only_its_own_document_when_its_period_is_cancelled() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let id = starter_with_voided_first_period(&env).await;
    let voided = env.get_invoices(id).await[0].id;
    let voided_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, voided)
        .await
        .expect("the void's notes")
        .len();
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            id,
            COMP_STARTER_SEATS_ID,
            5,
            meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
            Some(day(1, 11).and_time(chrono::NaiveTime::MIN)),
        )
        .await
        .expect("add 5 seats");
    let step = env
        .get_invoices(id)
        .await
        .into_iter()
        .find(|invoice| invoice.id != voided)
        .expect("the seats' invoice");
    assert_eq!(step.subtotal, 3_388);

    env.services()
        .cancel_subscription(
            Actor::System,
            id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 17)),
        )
        .await
        .expect("cancel mid-period");
    env.process_due_events().await;
    env.process_due_events().await;

    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, step.id)
        .await
        .expect("credit notes");
    let given: Vec<i64> = notes
        .iter()
        .flat_map(|note| note.line_items.iter())
        .map(|line| line.amount_subtotal)
        .collect();
    assert_eq!(given, vec![-2_419]);
    assert_eq!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, voided)
            .await
            .expect("the void's notes")
            .len(),
        voided_notes
    );
}

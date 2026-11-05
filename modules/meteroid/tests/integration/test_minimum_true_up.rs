//! A minimum commitment is a true-up of a completed period, billed on the invoice that closes it
//! and dated over it (Lago). Nothing is charged on the first invoice, a period cut short is
//! prorated by days served, and a true-up is never credited back.
use std::str::FromStr;
use std::sync::Arc;

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{
    BaseId, CustomerId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum, TaxResolverEnum};
use meteroid_store::domain::plan_minimums::MinimumScope;
use meteroid_store::domain::price_components::{
    PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{
    Address, BillingPeriodEnum, Invoice, InvoicingEntityPatch, LineItemKind, UsagePeriod,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn usage_env() -> (TestEnv, MutableMockUsageClient) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    (env, usage)
}

/// Bandwidth units used over `[from, to)`, read at midnight as the billing windows are.
fn set_usage(usage: &MutableMockUsageClient, from: NaiveDate, to: NaiveDate, units: i64) {
    let (start, end) = (
        from.and_time(chrono::NaiveTime::MIN),
        to.and_time(chrono::NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

async fn plan_minimum(env: &TestEnv, plan_version_id: PlanVersionId, amount_subunit: i64) {
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id,
            amount_subunit,
            includes_all_components: true,
        })
        .execute(&mut env.conn().await)
        .await
        .expect("minimum");
}

/// Platform Fee 29.00 a month in advance, bandwidth at 0.10 a unit in arrears, a 1,000.00
/// all-components minimum.
async fn fee_and_usage_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Fee and usage", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(29),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan_minimum(env, version_id, 100_000).await;
    version_id
}

async fn start(env: &TestEnv, plan_version_id: PlanVersionId, on: NaiveDate) -> SubscriptionId {
    subscription()
        .plan_version(plan_version_id)
        .start_date(on)
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn cancel_on(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) {
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(on),
        )
        .await
        .expect("cancellation scheduled");
    env.process_due_events().await;
    env.process_due_events().await;
}

/// The invoice of the period starting on `on` (the final one starts on the termination day).
async fn invoice_of(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Invoice {
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .iter()
        .find(|invoice| invoice.billing_period_start == on)
        .cloned()
        .unwrap_or_else(|| {
            let seen: Vec<_> = invoices
                .iter()
                .map(|i| (i.billing_period_start, i.status, lines(i)))
                .collect();
            panic!("no invoice starting {on}: {seen:?}")
        })
}

fn lines(invoice: &Invoice) -> Vec<(String, i64, NaiveDate, NaiveDate)> {
    invoice
        .line_items
        .iter()
        .map(|l| (l.name.clone(), l.amount_subtotal, l.start_date, l.end_date))
        .collect()
}

fn shortfalls(invoice: &Invoice) -> Vec<(i64, NaiveDate, NaiveDate)> {
    invoice
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect()
}

async fn note_lines(env: &TestEnv, invoice: &Invoice) -> Vec<(String, i64)> {
    let mut lines: Vec<(String, i64)> = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("credit notes")
        .into_iter()
        .flat_map(|note| note.line_items)
        .map(|line| (line.name, line.amount_subtotal))
        .collect();
    lines.sort();
    lines
}

/// September's usage of 0 leaves 1000.00 − 29.00 = 971.00, trued up on
/// Oct 1 over September. Cancelled Oct 10 after 900.00 of usage: only the fee comes back
/// (2900 × 22/31 = 2058), and Oct 1–10 owes nothing (29032 − 842 − 90000 < 0).
#[tokio::test]
async fn an_immediate_cancellation_credits_the_fee_only_and_trues_up_the_days_served() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.current_period_start,
        day(2024, 10, 1)
    );
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(
        shortfalls(&october),
        vec![(97_100, day(2024, 9, 1), day(2024, 10, 1))],
        "September trued up over September: {:?}",
        lines(&october)
    );

    set_usage(&usage, day(2024, 10, 1), day(2024, 10, 10), 9_000);
    cancel_on(&env, sub_id, day(2024, 10, 10)).await;

    assert_eq!(
        note_lines(&env, &october).await,
        vec![("Platform Fee".to_owned(), -2_058)],
        "the September true-up is never credited back"
    );
    let last = invoice_of(&env, sub_id, day(2024, 10, 10)).await;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));
    let usage_billed: i64 = last
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_some())
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(usage_billed, 90_000);
}

/// The same plan with a 45-day paid trial from Sep 1, cancelled on Sep 10 while still in trial
/// (a renewal ends the trial status, so its first period): the fee comes back as without a trial
/// (2900 × 21/30 = 2030), the trial owes no minimum, and the usage up to Sep 10 is billed.
#[tokio::test]
async fn a_paid_trial_with_a_minimum_cancelled_mid_period_credits_the_fee() {
    let (env, usage) = usage_env().await;
    let plan = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Fee and usage, paid trial", plan)
        .trial(45, PLAN_ENTERPRISE_ID, false)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(29),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan_minimum(&env, plan, 100_000).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 9, 1))
        .on_start()
        .trial_days(45)
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::TrialActive
    );
    let september = invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    assert!(shortfalls(&september).is_empty(), "{:?}", lines(&september));

    set_usage(&usage, day(2024, 9, 1), day(2024, 9, 10), 9_000);
    cancel_on(&env, sub_id, day(2024, 9, 10)).await;

    assert_eq!(
        note_lines(&env, &september).await,
        vec![("Platform Fee".to_owned(), -2_030)]
    );
    let last = invoice_of(&env, sub_id, day(2024, 9, 10)).await;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));
    let usage_billed: i64 = last
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_some())
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(usage_billed, 90_000);
}

/// The reverse: September's 971.00 of usage meets the minimum with the fee, nothing is trued up
/// on Oct 1. Cancelled Oct 10 with no October usage, the final invoice trues up Oct 1–10:
/// 100000 × 9/31 = 29032, less the fee left after its note (2900 − 2058 = 842): 28190.
#[tokio::test]
async fn a_period_cut_short_is_trued_up_on_the_final_invoice_by_days_served() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 9_710);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert!(shortfalls(&october).is_empty(), "{:?}", lines(&october));

    cancel_on(&env, sub_id, day(2024, 10, 10)).await;

    assert_eq!(
        note_lines(&env, &october).await,
        vec![("Platform Fee".to_owned(), -2_058)]
    );
    let last = invoice_of(&env, sub_id, day(2024, 10, 10)).await;
    assert_eq!(
        shortfalls(&last),
        vec![(28_190, day(2024, 10, 1), day(2024, 10, 10))],
        "{:?}",
        lines(&last)
    );
}

/// No period has completed when a fee + usage subscription starts: its first invoice bills the
/// fee alone, whatever the minimum.
#[tokio::test]
async fn the_first_invoice_of_a_fee_and_usage_plan_bills_no_shortfall() {
    let (env, _usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let first = invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    assert!(shortfalls(&first).is_empty(), "{:?}", lines(&first));
    assert_eq!(first.subtotal, 2_900);
}

/// LeetCode bills 35.00 in advance under a 50.00 minimum: nothing on January's invoice, then the
/// renewal trues January up, 5000 − 3500 = 1500, dated over January.
#[tokio::test]
async fn an_advance_only_plan_is_trued_up_on_the_next_renewal() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_1_LEETCODE_ID, 5_000).await;
    let sub_id = start(&env, PLAN_VERSION_1_LEETCODE_ID, day(2024, 1, 1)).await;
    let january = invoice_of(&env, sub_id, day(2024, 1, 1)).await;
    assert!(shortfalls(&january).is_empty(), "{:?}", lines(&january));

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(
        shortfalls(&february),
        vec![(1_500, day(2024, 1, 1), day(2024, 2, 1))],
        "{:?}",
        lines(&february)
    );
    assert_eq!(
        february.subtotal, 5_000,
        "February's 35.00 + January's 15.00"
    );
}

async fn french_customer(env: &TestEnv) -> CustomerId {
    let customer_id = CustomerId::new();
    diesel_models::customers::CustomerRowNew {
        id: customer_id,
        name: "Customer FR".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(
            serde_json::to_value(Address {
                line1: Some("1 rue de Rivoli".to_string()),
                line2: None,
                city: Some("Paris".to_string()),
                country: Some(CountryCode::from_str("FR").expect("country")),
                state: None,
                zip_code: Some("75001".to_string()),
            })
            .expect("address"),
        ),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some("FR12345678901".to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: true,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("customer");
    customer_id
}

/// A 100.00 minimum on Starter's Platform Fee (10%) and Seats (20%): January billed 29.00 + 10.00,
/// so the renewal trues up 61.00 over January, split by what each counted (45.36 and 15.64) and
/// taxed with its product: 4.54 and 3.13.
#[tokio::test]
async fn a_product_scoped_true_up_is_split_and_taxed_by_its_products() {
    use meteroid_store::domain::accounting::{TaxRateNew, TaxRateRule};
    use meteroid_store::repositories::accounting::AccountingInterface;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;

    let (env, _usage) = usage_env().await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("tax resolver");
    let reduced = env
        .store()
        .list_tax_categories(TENANT_ID)
        .await
        .expect("categories")
        .into_iter()
        .find(|category| category.key == "digital_services")
        .expect("built-in category");
    env.store()
        .update_product(
            Actor::System,
            ProductUpdate {
                id: PRODUCT_PLATFORM_FEE_ID,
                tenant_id: TENANT_ID,
                name: None,
                description: None,
                fee_type: None,
                fee_structure: None,
                tax_category_id: Some(Some(reduced.id)),
            },
        )
        .await
        .expect("platform fee category");
    env.store()
        .insert_custom_tax(
            TENANT_ID,
            TaxRateNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Reduced rate".to_string(),
                tax_code: "REDUCED_FR".to_string(),
                tax_category_id: Some(reduced.id),
                rules: vec![TaxRateRule {
                    country: Some(CountryCode::from_str("FR").expect("FR")),
                    region: None,
                    rate: dec!(0.10),
                }],
            },
        )
        .await
        .expect("reduced rate");
    let french = french_customer(&env).await;
    let sub_id = subscription()
        .customer(french)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .set_subscription_minimum(
            TENANT_ID,
            sub_id,
            10_000,
            MinimumScope::Products(vec![PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID]),
        )
        .await
        .expect("minimum");

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    let mut split: Vec<_> = february
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| {
            (
                line.product_id,
                line.amount_subtotal,
                line.tax_rate,
                line.tax_amount,
                line.start_date,
                line.end_date,
            )
        })
        .collect();
    split.sort_by_key(|(_, amount, ..)| -amount);
    assert_eq!(
        split,
        vec![
            (
                Some(PRODUCT_PLATFORM_FEE_ID),
                4_536,
                dec!(0.10),
                454,
                day(2024, 1, 1),
                day(2024, 2, 1)
            ),
            (
                Some(PRODUCT_SEATS_ID),
                1_564,
                dec!(0.20),
                313,
                day(2024, 1, 1),
                day(2024, 2, 1)
            ),
        ],
        "{:?}",
        lines(&february)
    );
}

/// Appends to `invoice` a shortfall charged up front by the rule before the true-up, dated over
/// `[from, to)`, its totals kept consistent. Fabricated state: the transition is exercised
/// whichever release wrote the invoice.
async fn append_up_front_shortfall(
    env: &TestEnv,
    invoice: &Invoice,
    amount: i64,
    from: NaiveDate,
    to: NaiveDate,
) {
    assert_eq!(
        (
            invoice.tax_amount,
            invoice.applied_credits,
            invoice.discount
        ),
        (0, 0, 0)
    );
    let mut old = invoice.line_items[0].clone();
    old.local_id = uuid::Uuid::now_v7().to_string();
    old.name = "Minimum commitment".to_owned();
    old.kind = LineItemKind::MinimumShortfall;
    old.amount_subtotal = amount;
    old.taxable_amount = amount;
    old.amount_total = amount;
    old.tax_amount = 0;
    old.tax_details = vec![];
    old.quantity = None;
    old.unit_price = None;
    old.sub_lines = vec![];
    old.is_prorated = false;
    old.price_component_id = None;
    old.sub_component_id = None;
    old.sub_add_on_id = None;
    old.product_id = None;
    old.metric_id = None;
    old.description = None;
    old.start_date = from;
    old.end_date = to;
    let mut line_items = invoice.line_items.clone();
    line_items.push(old);

    use diesel_models::schema::invoice::dsl as i;
    diesel::update(i::invoice.filter(i::id.eq(invoice.id)))
        .set((
            i::line_items.eq(serde_json::to_value(&line_items).expect("lines")),
            i::subtotal.eq(invoice.subtotal + amount),
            i::subtotal_recurring.eq(invoice.subtotal_recurring + amount),
            i::total.eq(invoice.total + amount),
            i::amount_due.eq(invoice.amount_due + amount),
        ))
        .execute(&mut env.conn().await)
        .await
        .expect("old up-front shortfall");
}

/// Redates `invoice`'s shortfall lines over `[from, to)`, as the rule before the true-up dated a
/// shortfall charged up front. Fabricated state; amounts and totals are unchanged.
async fn redate_shortfall(env: &TestEnv, invoice: &Invoice, from: NaiveDate, to: NaiveDate) {
    let line_items: Vec<_> = invoice
        .line_items
        .iter()
        .cloned()
        .map(|mut line| {
            if line.kind == LineItemKind::MinimumShortfall {
                line.start_date = from;
                line.end_date = to;
            }
            line
        })
        .collect();
    use diesel_models::schema::invoice::dsl as i;
    diesel::update(i::invoice.filter(i::id.eq(invoice.id)))
        .set(i::line_items.eq(serde_json::to_value(&line_items).expect("lines")))
        .execute(&mut env.conn().await)
        .await
        .expect("redated shortfall");
}

/// LeetCode's January invoice, issued, with `amount` charged up front over January.
async fn with_up_front_shortfall(env: &TestEnv, sub_id: SubscriptionId, amount: i64) -> Invoice {
    let mut january = invoice_of(env, sub_id, day(2024, 1, 1)).await;
    if january.status == InvoiceStatusEnum::Draft {
        env.manual_finalize_invoice(january.id)
            .await
            .expect("January issued");
        january = invoice_of(env, sub_id, day(2024, 1, 1)).await;
    }
    append_up_front_shortfall(env, &january, amount, day(2024, 1, 1), day(2024, 2, 1)).await;
    invoice_of(env, sub_id, day(2024, 1, 1)).await
}

/// January's invoice already charged 15.00 up front under the earlier rule: the renewal trues
/// January up net of it, 5000 − 3500 − 1500 = 0, so it is never charged twice.
#[tokio::test]
async fn a_true_up_nets_the_shortfall_its_opening_invoice_charged_up_front() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_1_LEETCODE_ID, 5_000).await;
    let sub_id = start(&env, PLAN_VERSION_1_LEETCODE_ID, day(2024, 1, 1)).await;
    with_up_front_shortfall(&env, sub_id, 1_500).await;

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert!(shortfalls(&february).is_empty(), "{:?}", lines(&february));
    assert_eq!(february.subtotal, 3_500);
}

/// As above, with 10.00 of that up-front 15.00 given back by a note outside a cancellation: a
/// charge counts as billed, never net of a note, so January stays covered: 5000 − 3500 − 1500.
#[tokio::test]
async fn a_note_on_an_up_front_shortfall_does_not_reopen_its_period() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_1_LEETCODE_ID, 5_000).await;
    let sub_id = start(&env, PLAN_VERSION_1_LEETCODE_ID, day(2024, 1, 1)).await;
    let january = with_up_front_shortfall(&env, sub_id, 1_500).await;
    let old = january
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::MinimumShortfall)
        .expect("the up-front shortfall");
    env.store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: january.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: old.local_id.clone(),
                    amount_cents: 1_000,
                }],
                reason: Some("Goodwill".to_owned()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("a note on the up-front shortfall");

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert!(shortfalls(&february).is_empty(), "{:?}", lines(&february));
}

/// Starter (29.00 + 10.00) under a 100.00 minimum adds a 50.00 support fee on Jan 16, at once:
/// an increase, charged 5000 × 16/31 = 2581 on its adjustment invoice. January's true-up counts
/// it: 10000 − 3900 − 2581 = 3519.
#[tokio::test]
async fn an_immediate_increase_counts_toward_the_period_true_up() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_STARTER_ID, 10_000).await;
    let sub_id = start(&env, PLAN_VERSION_STARTER_ID, day(2024, 1, 1)).await;

    let support = AddExtraComponent {
        name: "Support".to_string(),
        product: ProductRef::New {
            name: "Support Product".to_string(),
            fee_type: FeeTypeEnum::Rate,
            fee_structure: FeeStructure::Rate {},
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate { rate: dec!(50) },
            credit_unit: None,
        }),
    };
    let result = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![support],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2024, 1, 16),
        )
        .await
        .expect("an increase applies at once under a minimum");
    let adjustment_id = result.adjustment_invoice_id.expect("adjustment invoice");
    let adjustment = env.get_detailed_invoice(adjustment_id).await.invoice;
    assert_eq!(adjustment.subtotal, 2_581);

    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(
        shortfalls(&february),
        vec![(3_519, day(2024, 1, 1), day(2024, 2, 1))],
        "{:?}",
        lines(&february)
    );
}

/// A fixed term ending Mar 1 on the period boundary: February's renewal trues up January, and
/// the final invoice of Mar 1 trues up February, 5000 − 3500 = 1500, with nothing else to bill.
#[tokio::test]
async fn the_last_period_is_trued_up_on_the_final_invoice() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_1_LEETCODE_ID, 5_000).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .end_date(day(2024, 3, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(
        shortfalls(&february),
        vec![(1_500, day(2024, 1, 1), day(2024, 2, 1))]
    );

    env.process_cycles().await;
    let last = invoice_of(&env, sub_id, day(2024, 3, 1)).await;
    assert_eq!(
        lines(&last)
            .into_iter()
            .map(|(_, amount, start, end)| (amount, start, end))
            .collect::<Vec<_>>(),
        vec![(1_500, day(2024, 2, 1), day(2024, 3, 1))],
        "the final invoice carries February's true-up alone"
    );
}

/// A cancellation on Oct 11 (21 of 31 days left) of a period whose issued invoice charged 971.00
/// up front before the true-up: that charge gives its unused days back like the fee (97100 × 21/31
/// = 65777, 2900 × 21/31 = 1965), and the final invoice owes nothing: 32258 − 935 − 1000 − 31323 < 0.
#[tokio::test]
async fn a_cancellation_credits_the_unused_days_of_a_shortfall_charged_up_front() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(october.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        shortfalls(&october),
        vec![(97_100, day(2024, 9, 1), day(2024, 10, 1))]
    );
    // As the rule before the true-up wrote it: October's shortfall, dated over October.
    redate_shortfall(&env, &october, day(2024, 10, 1), day(2024, 11, 1)).await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 10, 11), 100);
    cancel_on(&env, sub_id, day(2024, 10, 11)).await;

    assert_eq!(
        note_lines(&env, &october).await,
        vec![
            ("Minimum commitment".to_owned(), -65_777),
            ("Platform Fee".to_owned(), -1_965),
        ]
    );
    let last = invoice_of(&env, sub_id, day(2024, 10, 11)).await;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));
}

/// As above while October's invoice (no auto-advance) is still a draft when cancelled: issued
/// afterwards, it keeps the shortfall it charged up front, and the deferred note credits it with
/// the fee. The final invoice owes nothing whether issued before or after that note.
#[tokio::test]
async fn a_draft_keeps_its_up_front_shortfall_and_is_credited_once_issued() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 9, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let september = invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    append_up_front_shortfall(&env, &september, 97_100, day(2024, 9, 1), day(2024, 10, 1)).await;
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(october.status, InvoiceStatusEnum::Draft);
    assert!(
        shortfalls(&october).is_empty(),
        "September was charged up front: {:?}",
        lines(&october)
    );
    append_up_front_shortfall(&env, &october, 97_100, day(2024, 10, 1), day(2024, 11, 1)).await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 10, 11), 100);
    cancel_on(&env, sub_id, day(2024, 10, 11)).await;
    let last = invoice_of(&env, sub_id, day(2024, 10, 11)).await;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));

    env.manual_finalize_invoice(october.id)
        .await
        .expect("October issued");
    for _ in 0..2 {
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("sweep");
    }
    let issued = env.get_detailed_invoice(october.id).await.invoice;
    assert_eq!(
        shortfalls(&issued),
        vec![(97_100, day(2024, 10, 1), day(2024, 11, 1))],
        "a shortfall charged up front stays as billed: {:?}",
        lines(&issued)
    );
    assert_eq!(
        note_lines(&env, &issued).await,
        vec![
            ("Minimum commitment".to_owned(), -65_777),
            ("Platform Fee".to_owned(), -1_965),
        ]
    );
    env.manual_finalize_invoice(last.id)
        .await
        .expect("final invoice issued");
    let last = env.get_detailed_invoice(last.id).await.invoice;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));
}

/// Rewrites `invoice` as the rule before the true-up wrote it: its shortfall lines replaced by
/// `old` (amount, dated over `[from, to)`), totals kept consistent. Fabricated history.
async fn as_written_before_the_true_up(
    env: &TestEnv,
    invoice: &Invoice,
    old: Option<(i64, NaiveDate, NaiveDate)>,
) -> Invoice {
    assert_eq!(
        (
            invoice.tax_amount,
            invoice.applied_credits,
            invoice.discount
        ),
        (0, 0, 0)
    );
    let removed: i64 = shortfalls(invoice).iter().map(|(amount, ..)| amount).sum();
    let mut line_items: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|line| line.kind != LineItemKind::MinimumShortfall)
        .cloned()
        .collect();
    let added = old.map_or(0, |(amount, _, _)| amount);
    if let Some((amount, from, to)) = old {
        let mut line = <[_]>::first(&invoice.line_items)
            .cloned()
            .expect("a line to shape the old shortfall on");
        line.local_id = uuid::Uuid::now_v7().to_string();
        line.name = "Minimum commitment".to_owned();
        line.kind = LineItemKind::MinimumShortfall;
        line.charge_type = None;
        line.amount_subtotal = amount;
        line.taxable_amount = amount;
        line.amount_total = amount;
        line.tax_amount = 0;
        line.tax_details = vec![];
        line.quantity = None;
        line.unit_price = None;
        line.sub_lines = vec![];
        line.is_prorated = false;
        line.price_component_id = None;
        line.sub_component_id = None;
        line.sub_add_on_id = None;
        line.product_id = None;
        line.metric_id = None;
        line.description = None;
        line.group_by_dimensions = None;
        line.subscription_id = None;
        line.start_date = from;
        line.end_date = to;
        line_items.push(line);
    }
    let delta = added - removed;
    use diesel_models::schema::invoice::dsl as i;
    diesel::update(i::invoice.filter(i::id.eq(invoice.id)))
        .set((
            i::line_items.eq(serde_json::to_value(&line_items).expect("lines")),
            i::subtotal.eq(invoice.subtotal + delta),
            i::subtotal_recurring.eq(invoice.subtotal_recurring + delta),
            i::total.eq(invoice.total + delta),
            i::amount_due.eq(invoice.amount_due + delta),
        ))
        .execute(&mut env.conn().await)
        .await
        .expect("history before the true-up");
    env.get_detailed_invoice(invoice.id).await.invoice
}

/// The issued invoice of the period starting on `on`.
async fn issued_invoice_of(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Invoice {
    let invoice = invoice_of(env, sub_id, on).await;
    if invoice.status == InvoiceStatusEnum::Draft {
        env.manual_finalize_invoice(invoice.id)
            .await
            .expect("invoice issued");
        return invoice_of(env, sub_id, on).await;
    }
    invoice
}

/// Bandwidth at 0.01 a unit in arrears alone, under a 7.56 all-components minimum: the
/// pay-as-you-go plan of the transition.
async fn usage_only_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Pay as you go", version_id)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit { rate: dec!(0.01) },
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan_minimum(env, version_id, 756).await;
    version_id
}

async fn mrr_movements(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> Vec<(diesel_models::enums::MrrMovementType, i64)> {
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(sub_id))
        .order((m::created_at, m::id))
        .select((m::movement_type, m::net_mrr_change))
        .load(&mut env.conn().await)
        .await
        .unwrap()
}

/// The minimum set or cleared on a running subscription restates the MRR it floors, each move a
/// movement: the 7.56 floor raised to 100.00, then removed.
#[tokio::test]
async fn setting_and_clearing_the_minimum_restates_the_mrr() {
    use diesel_models::enums::MrrMovementType;

    let (env, _usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 756);

    env.services()
        .set_subscription_minimum(TENANT_ID, sub_id, 10_000, MinimumScope::AllComponents)
        .await
        .expect("minimum");
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 10_000);
    env.services()
        .clear_subscription_minimum(TENANT_ID, sub_id)
        .await
        .expect("minimum removed");
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 0);

    assert_eq!(
        mrr_movements(&env, sub_id).await,
        vec![
            (MrrMovementType::NewBusiness, 756),
            (MrrMovementType::Expansion, 9_244),
            (MrrMovementType::Contraction, -10_000),
        ]
    );
}

/// Before it starts, a subscription has booked nothing: the new floor is what its creation will
/// book, not a movement now.
#[tokio::test]
async fn a_minimum_set_before_the_start_is_booked_by_the_creation() {
    use diesel_models::enums::SubscriptionEventType;
    use diesel_models::subscription_events::SubscriptionEventRow;

    let (env, _usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let start_date = chrono::Utc::now().date_naive() + chrono::Duration::days(10);
    let sub_id = start(&env, plan, start_date).await;
    env.services()
        .set_subscription_minimum(TENANT_ID, sub_id, 10_000, MinimumScope::AllComponents)
        .await
        .expect("minimum");

    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 0);
    assert!(mrr_movements(&env, sub_id).await.is_empty());
    let created = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        &mut env.conn().await,
        sub_id,
        SubscriptionEventType::Created,
        start_date,
    )
    .await
    .unwrap()
    .expect("a Created event");
    assert_eq!(created.mrr_delta, Some(10_000));
}

/// Usage alone: the rule before the true-up charged nothing on the first invoice, then on the
/// issued Oct 1 invoice September's 7.56 − 3.94 = 3.62, dated over October.
async fn usage_only_with_old_shortfall(
    env: &TestEnv,
    usage: &MutableMockUsageClient,
    sub_id: SubscriptionId,
) -> Invoice {
    set_usage(usage, day(2024, 9, 1), day(2024, 10, 1), 394);
    env.process_cycles().await;
    let october = invoice_of(env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(october.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        shortfalls(&october),
        vec![(362, day(2024, 9, 1), day(2024, 10, 1))],
        "{:?}",
        lines(&october)
    );
    as_written_before_the_true_up(
        env,
        &october,
        Some((362, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await
}

/// The production case: the 3.62 dated over October was September's true-up, nothing of
/// October was prepaid. October's 5.00 of usage is trued up in full, 756 − 500 = 256.
#[tokio::test]
async fn an_old_shortfall_beside_usage_alone_is_not_netted_from_the_next_true_up() {
    let (env, usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    usage_only_with_old_shortfall(&env, &usage, sub_id).await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 500);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(256, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
    assert_eq!(november.subtotal, 756);
}

/// As above, cancelled Oct 11: September's true-up gives nothing back, and Oct 1–11 is trued up
/// by days served, 756 × 10/31 = 244, less its 1.00 of usage: 144.
#[tokio::test]
async fn a_cancellation_never_credits_an_old_shortfall_beside_usage_alone() {
    let (env, usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let october = usage_only_with_old_shortfall(&env, &usage, sub_id).await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 10, 11), 100);
    cancel_on(&env, sub_id, day(2024, 10, 11)).await;

    assert_eq!(
        note_lines(&env, &october).await,
        Vec::<(String, i64)>::new()
    );
    let last = invoice_of(&env, sub_id, day(2024, 10, 11)).await;
    assert_eq!(
        shortfalls(&last),
        vec![(144, day(2024, 10, 1), day(2024, 10, 11))],
        "{:?}",
        lines(&last)
    );
}

/// An October draft (no auto-advance) to which an earlier release gave September's 3.62, dated
/// over October: issued, it trues September up over September instead, once, and October is
/// trued up in full, 756 − 500 = 256.
#[tokio::test]
async fn a_draft_beside_usage_alone_replaces_its_old_shortfall_by_the_true_up() {
    let (env, usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 9, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 394);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(october.status, InvoiceStatusEnum::Draft);
    as_written_before_the_true_up(
        &env,
        &october,
        Some((362, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    env.manual_finalize_invoice(october.id)
        .await
        .expect("October issued");
    let issued = env.get_detailed_invoice(october.id).await.invoice;
    assert_eq!(
        shortfalls(&issued),
        vec![(362, day(2024, 9, 1), day(2024, 10, 1))],
        "{:?}",
        lines(&issued)
    );
    assert_eq!(issued.subtotal, 756);

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 500);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(256, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
}

/// As above, its minimum removed before the draft is issued: no true-up replaces the 3.62, which
/// stays as billed, and a cancellation on Oct 11 gives none of it back.
#[tokio::test]
async fn a_draft_whose_minimum_was_removed_keeps_its_old_shortfall_uncredited() {
    let (env, usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 9, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 394);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    as_written_before_the_true_up(
        &env,
        &october,
        Some((362, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;
    env.services()
        .clear_subscription_minimum(TENANT_ID, sub_id)
        .await
        .expect("minimum removed");

    env.manual_finalize_invoice(october.id)
        .await
        .expect("October issued");
    let issued = env.get_detailed_invoice(october.id).await.invoice;
    assert_eq!(
        shortfalls(&issued),
        vec![(362, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&issued)
    );
    assert_eq!(issued.subtotal, 756);

    set_usage(&usage, day(2024, 10, 1), day(2024, 10, 11), 100);
    cancel_on(&env, sub_id, day(2024, 10, 11)).await;
    assert_eq!(note_lines(&env, &issued).await, Vec::<(String, i64)>::new());
    let last = invoice_of(&env, sub_id, day(2024, 10, 11)).await;
    assert!(shortfalls(&last).is_empty(), "{:?}", lines(&last));
}

/// Fee and usage: the rule before the true-up charged 1000.00 − 29.00 = 971.00 over September on
/// the first invoice, then on Oct 1 1000.00 − 29.00 − 900.00 of September's usage = 71.00 over
/// October. Each prepaid its period: September owes nothing more, October is trued up net of
/// it, 100000 − 2900 − 50000 − 7100 = 40000.
#[tokio::test]
async fn old_shortfalls_beside_an_advance_fee_are_netted_from_their_periods() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let september = issued_invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    assert_eq!(september.subtotal, 2_900);
    as_written_before_the_true_up(
        &env,
        &september,
        Some((97_100, day(2024, 9, 1), day(2024, 10, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 9_000);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert!(shortfalls(&october).is_empty(), "{:?}", lines(&october));
    assert_eq!(october.subtotal, 92_900);
    as_written_before_the_true_up(
        &env,
        &october,
        Some((7_100, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 5_000);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(40_000, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
}

/// As above under a 1000.00 minimum on Bandwidth alone: the fee is out of scope, so the rule
/// before the true-up charged nothing on the first invoice, and its 100.00 on Oct 1 was
/// September's true-up. October's is 100000 − 50000 = 50000.
#[tokio::test]
async fn an_old_shortfall_beside_an_out_of_scope_fee_is_not_netted() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    env.services()
        .set_subscription_minimum(
            TENANT_ID,
            sub_id,
            100_000,
            MinimumScope::Products(vec![PRODUCT_BANDWIDTH_ID]),
        )
        .await
        .expect("minimum");
    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 9_000);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(
        shortfalls(&october),
        vec![(10_000, day(2024, 9, 1), day(2024, 10, 1))],
        "{:?}",
        lines(&october)
    );
    as_written_before_the_true_up(
        &env,
        &october,
        Some((10_000, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 5_000);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(50_000, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
}

/// A 20.00 setup fee, billed once, and bandwidth at 0.10 a unit under a 100.00 minimum. The rule
/// before the true-up charged 10000 − 2000 = 8000 over September on the first invoice, then
/// 10000 − 3000 of September's usage = 7000 over October on Oct 1. September is covered; October
/// is trued up net of its 70.00: 10000 − 1000 − 7000 = 2000. Each period bills its floor once.
#[tokio::test]
async fn old_shortfalls_beside_a_one_time_fee_prepaid_each_period_once() {
    let (env, usage) = usage_env().await;
    let plan = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Setup and usage", plan)
        .components(vec![
            SeedComp::one_time(
                PriceComponentId::new(),
                "Setup",
                PRODUCT_ONETIME_SETUP_ID,
                PriceId::new(),
                dec!(20),
                1,
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan_minimum(&env, plan, 10_000).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let september = issued_invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    assert_eq!(september.subtotal, 2_000, "{:?}", lines(&september));
    as_written_before_the_true_up(
        &env,
        &september,
        Some((8_000, day(2024, 9, 1), day(2024, 10, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 300);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert!(shortfalls(&october).is_empty(), "{:?}", lines(&october));
    assert_eq!(october.subtotal, 3_000);
    as_written_before_the_true_up(
        &env,
        &october,
        Some((7_000, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 100);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(2_000, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
    assert_eq!(november.subtotal, 3_000, "October's 10.00 and 20.00");
}

/// A 120.00 annual fee on a monthly subscription with bandwidth at 0.10 a unit, under a 100.00
/// monthly minimum: the first invoice's fee meets September's floor, so the rule before the
/// true-up charged nothing there, then 10000 − 3000 = 7000 over October on Oct 1, which no
/// October fee line stands beside. October is trued up net of it: 10000 − 1000 − 7000 = 2000.
#[tokio::test]
async fn an_old_shortfall_of_a_month_without_its_annual_fee_is_netted() {
    let (env, usage) = usage_env().await;
    let plan = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Annual fee and usage", plan)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(120),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan_minimum(&env, plan, 10_000).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let september = issued_invoice_of(&env, sub_id, day(2024, 9, 1)).await;
    assert_eq!(september.subtotal, 12_000, "{:?}", lines(&september));

    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 300);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert!(shortfalls(&october).is_empty(), "{:?}", lines(&october));
    assert_eq!(october.subtotal, 3_000, "{:?}", lines(&october));
    as_written_before_the_true_up(
        &env,
        &october,
        Some((7_000, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 100);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(2_000, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
}

/// Usage alone at first, then a 1.00 monthly support fee added on Sep 16 (100 × 15/30 = 50 at
/// once). The rule before the true-up never applied its floor on the first invoice, so its Oct 1
/// line, 756 − 100 − 394 = 262 over October beside October's fee, still trued up September:
/// October is trued up in full, 756 − 100 − 500 = 156.
#[tokio::test]
async fn an_old_shortfall_stays_a_true_up_after_an_advance_fee_is_added() {
    let (env, usage) = usage_env().await;
    let plan = usage_only_plan(&env).await;
    let sub_id = start(&env, plan, day(2024, 9, 1)).await;
    let support = AddExtraComponent {
        name: "Support".to_string(),
        product: ProductRef::New {
            name: "Support Product".to_string(),
            fee_type: FeeTypeEnum::Rate,
            fee_structure: FeeStructure::Rate {},
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate { rate: dec!(1) },
            credit_unit: None,
        }),
    };
    let result = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![support],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2024, 9, 16),
        )
        .await
        .expect("an increase applies at once under a minimum");
    let adjustment_id = result.adjustment_invoice_id.expect("adjustment invoice");
    assert_eq!(
        env.get_detailed_invoice(adjustment_id)
            .await
            .invoice
            .subtotal,
        50
    );

    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 1), 394);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert_eq!(
        shortfalls(&october),
        vec![(312, day(2024, 9, 1), day(2024, 10, 1))],
        "{:?}",
        lines(&october)
    );
    as_written_before_the_true_up(
        &env,
        &october,
        Some((262, day(2024, 10, 1), day(2024, 11, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 10, 1), day(2024, 11, 1), 500);
    env.process_cycles().await;
    let november = invoice_of(&env, sub_id, day(2024, 11, 1)).await;
    assert_eq!(
        shortfalls(&november),
        vec![(156, day(2024, 10, 1), day(2024, 11, 1))],
        "{:?}",
        lines(&november)
    );
}

/// Fee and usage started Sep 20 on a period anchored on the 1st: the first invoice bills the
/// fee for 11 of 30 days, 2900 × 11/30 = 1063, and the rule before the true-up the floor for
/// as many, 100000 × 11/30 = 36667, less that fee: 35604 over Sep 20–Oct 1. It prepaid those
/// days: the Oct 1 invoice trues nothing up beside September's 100.00 of usage.
#[tokio::test]
async fn an_old_prorated_shortfall_on_the_first_invoice_prepaid_its_days() {
    let (env, usage) = usage_env().await;
    let plan = fee_and_usage_plan(&env).await;
    let mut request = subscription()
        .plan_version(plan)
        .start_date(day(2024, 9, 20))
        .on_start()
        .no_trial()
        .request();
    request.subscription.billing_day_anchor = Some(1);
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("subscription")
        .id;
    let september = issued_invoice_of(&env, sub_id, day(2024, 9, 20)).await;
    assert_eq!(september.subtotal, 1_063, "{:?}", lines(&september));
    as_written_before_the_true_up(
        &env,
        &september,
        Some((35_604, day(2024, 9, 20), day(2024, 10, 1))),
    )
    .await;

    set_usage(&usage, day(2024, 9, 20), day(2024, 10, 1), 1_000);
    env.process_cycles().await;
    let october = invoice_of(&env, sub_id, day(2024, 10, 1)).await;
    assert!(shortfalls(&october).is_empty(), "{:?}", lines(&october));
    assert_eq!(october.subtotal, 2_900 + 10_000);
}

/// A 14-day paid trial at 99.00 a month under a 200.00 minimum: trial days are not committed, so
/// January is trued up over its 17 other days, 20000 × 17/31 = 10968, less 9900: 1068.
#[tokio::test]
async fn paid_trial_days_are_not_committed_days() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_PAID_TRIAL_ID, 20_000).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(
        shortfalls(&february),
        vec![(1_068, day(2024, 1, 1), day(2024, 2, 1))],
        "{:?}",
        lines(&february)
    );
}

/// A paid trial ending on the renewal date leaves no committed day in its period: nothing is
/// trued up, whether the trial's end or the renewal is processed first.
#[tokio::test]
async fn a_paid_trial_ending_at_the_renewal_is_never_trued_up_whatever_the_order() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_PAID_TRIAL_ID, 20_000).await;
    let trial_of_31_days = || {
        subscription()
            .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
            .start_date(day(2024, 1, 1))
            .on_start()
            .trial_days(31)
            .no_auto_charge()
    };

    let renewal_first = trial_of_31_days().create(env.services()).await;
    env.process_cycle_transitions().await;
    env.process_due_events().await;

    let trial_end_first = trial_of_31_days().create(env.services()).await;
    env.process_due_events().await;
    env.process_cycle_transitions().await;
    env.process_due_events().await;

    for sub_id in [renewal_first, trial_end_first] {
        let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
        assert!(shortfalls(&february).is_empty(), "{:?}", lines(&february));
        assert_eq!(february.subtotal, 9_900);
    }
}

/// Platform Fee 29.00 a month under a 500.00 minimum, and the same fee 300.00 a quarter under a
/// 2,000.00 minimum.
async fn monthly_and_quarterly_plans(env: &TestEnv) -> (PlanVersionId, PlanVersionId) {
    let (monthly, quarterly) = (PlanVersionId::new(), PlanVersionId::new());
    for (version, name, period, rate, minimum) in [
        (
            monthly,
            "Monthly",
            DieselBillingPeriodEnum::Monthly,
            dec!(29),
            50_000,
        ),
        (
            quarterly,
            "Quarterly",
            DieselBillingPeriodEnum::Quarterly,
            dec!(300),
            200_000,
        ),
    ] {
        PlanSeed::new(PlanId::new(), name, version)
            .components(vec![SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                period,
                rate,
            )])
            .seed(&mut env.conn().await)
            .await
            .unwrap();
        plan_minimum(env, version, minimum).await;
    }
    (monthly, quarterly)
}

/// A switch to the quarterly plan recorded by a release before the true-up, without the terms it
/// replaced: issuing the Feb 1 draft still trues January up under the monthly plan's 500.00 on
/// the monthly grid, rebuilt from the plan version left: 50000 − 2900 = 47100.
#[tokio::test]
async fn a_switch_written_before_the_true_up_is_trued_up_under_the_plan_it_left() {
    let (env, _usage) = usage_env().await;
    let (monthly, quarterly) = monthly_and_quarterly_plans(&env).await;
    let sub_id = subscription()
        .plan_version(monthly)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, quarterly, vec![])
        .await
        .expect("a change at the period end");
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        quarterly
    );
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);

    // As a release before the true-up wrote the switch. Fabricated state.
    diesel::sql_query(
        "UPDATE subscription_event SET details = details - 'outgoing_minimum' - 'outgoing_period' \
         WHERE subscription_id = $1 AND event_type = 'SWITCH'",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id)
    .execute(&mut env.conn().await)
    .await
    .expect("switch without its outgoing terms");

    env.manual_finalize_invoice(february.id)
        .await
        .expect("February issued");
    let issued = env.get_detailed_invoice(february.id).await.invoice;
    assert_eq!(
        shortfalls(&issued),
        vec![(47_100, day(2024, 1, 1), day(2024, 2, 1))],
        "{:?}",
        lines(&issued)
    );
}

/// The same goodwill note of 10.00 on January's 35.00, issued before one renewal and after the
/// other was drafted: fees count as billed, so both true up 5000 − 3500 = 1500.
#[tokio::test]
async fn a_goodwill_note_never_changes_the_true_up() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_1_LEETCODE_ID, 5_000).await;
    let goodwill = |invoice: Invoice| {
        let env = &env;
        async move {
            env.store()
                .create_and_finalize_credit_note(
                    Actor::System,
                    TENANT_ID,
                    CreateCreditNoteParams {
                        invoice_id: invoice.id,
                        line_items: vec![CreditLineItem::Amount {
                            local_id: invoice.line_items[0].local_id.clone(),
                            amount_cents: 1_000,
                        }],
                        reason: Some("Goodwill".to_owned()),
                        memo: None,
                        credit_type: CreditType::DebtCancellation,
                        custom_properties: None,
                    },
                )
                .await
                .expect("goodwill note");
        }
    };
    let issued_january = |sub_id: SubscriptionId| {
        let env = &env;
        async move {
            let january = invoice_of(env, sub_id, day(2024, 1, 1)).await;
            if january.status == InvoiceStatusEnum::Draft {
                env.manual_finalize_invoice(january.id)
                    .await
                    .expect("January issued");
            }
            invoice_of(env, sub_id, day(2024, 1, 1)).await
        }
    };

    let before = start(&env, PLAN_VERSION_1_LEETCODE_ID, day(2024, 1, 1)).await;
    goodwill(issued_january(before).await).await;
    let after = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let after_january = issued_january(after).await;

    env.process_cycles().await;
    let drafted = invoice_of(&env, after, day(2024, 2, 1)).await;
    assert_eq!(drafted.status, InvoiceStatusEnum::Draft);
    goodwill(after_january).await;
    env.manual_finalize_invoice(drafted.id)
        .await
        .expect("February issued");

    for sub_id in [before, after] {
        let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
        assert_eq!(
            shortfalls(&february),
            vec![(1_500, day(2024, 1, 1), day(2024, 2, 1))],
            "{:?}",
            lines(&february)
        );
    }
}

/// Under a minimum, an immediate amendment adding a 50.00 fee while removing the 29.00 Platform
/// Fee is refused, in preview and apply: anything removed waits for the period's end, even beside
/// a larger increase.
#[tokio::test]
async fn an_immediate_amendment_removing_a_fee_is_refused_even_beside_an_increase() {
    let (env, _usage) = usage_env().await;
    plan_minimum(&env, PLAN_VERSION_STARTER_ID, 10_000).await;
    let sub_id = start(&env, PLAN_VERSION_STARTER_ID, day(2024, 1, 1)).await;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Platform Fee")
        .expect("Platform Fee component");
    let swap = || SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![AddExtraComponent {
                name: "Support".to_string(),
                product: ProductRef::New {
                    name: "Support Product".to_string(),
                    fee_type: FeeTypeEnum::Rate,
                    fee_structure: FeeStructure::Rate {},
                },
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Rate { rate: dec!(50) },
                    credit_unit: None,
                }),
            }],
            removed: vec![platform.id],
        },
        add_on_changes: AddOnChanges::default(),
    };

    let refused = env
        .services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, swap(), day(2024, 1, 16))
        .await
        .expect_err("a removal waits for the period's end");
    assert!(
        format!("{refused:?}").contains("minimum commitment"),
        "{refused:?}"
    );
    let previewed = env
        .services()
        .preview_amendment(sub_id, TENANT_ID, swap())
        .await
        .expect_err("the preview is refused too");
    assert!(
        format!("{previewed:?}").contains("minimum commitment"),
        "{previewed:?}"
    );
    assert!(
        env.get_subscription_components(sub_id)
            .await
            .iter()
            .any(|c| c.id == platform.id),
        "the fee stays"
    );
}

/// A scheduled switch from the monthly plan (500.00 minimum) to the quarterly one (2,000.00) at
/// Feb 1: February's renewal trues January up under the monthly terms, 50000 − 2900 = 47100; May's
/// trues up the quarter from the switch, net of its fee, and never measures January again.
#[tokio::test]
async fn a_cadence_change_trues_up_each_period_once_under_its_own_terms() {
    let (env, _usage) = usage_env().await;
    let (monthly, quarterly) = monthly_and_quarterly_plans(&env).await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, quarterly, vec![])
        .await
        .expect("a change at the period end");
    env.process_cycles().await;
    let february = invoice_of(&env, sub_id, day(2024, 2, 1)).await;
    assert_eq!(
        shortfalls(&february),
        vec![(47_100, day(2024, 1, 1), day(2024, 2, 1))],
        "{:?}",
        lines(&february)
    );
    let quarterly_fee: i64 = february
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::Regular && l.start_date == day(2024, 2, 1))
        .map(|l| l.amount_subtotal)
        .sum();
    assert!(quarterly_fee > 0, "{:?}", lines(&february));

    env.process_cycles().await;
    let next = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|i| i.billing_period_start > day(2024, 2, 1))
        .min_by_key(|i| i.billing_period_start)
        .expect("the renewal after the switch");
    assert_eq!(
        next.billing_period_start,
        day(2024, 5, 1),
        "a quarter from the switch"
    );
    assert_eq!(
        shortfalls(&next),
        vec![(200_000 - quarterly_fee, day(2024, 2, 1), day(2024, 5, 1))],
        "the quarter from the switch, its fee counted, January never measured again: {:?}",
        lines(&next)
    );
}

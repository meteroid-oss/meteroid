//! Plan changes between credit-pack plans shaped like the Halyard sandbox. "Scale" (Pro) sells a
//! EUR 500 pack worth EUR 550 that resets each period, and a EUR 50 welcome credit once; "Studio"
//! (Starter) includes 5,000 credits a period, at most 1,000 rolled over for a month; "Large"
//! (Enterprise) sells a EUR 1,000 pack worth EUR 1,100. A pack tied to the period is prorated like
//! any advance fee: the new pack billed and granted by the share of the period left, the old
//! pack's credit removed once it paid the usage up to the change, its fee credited back on a
//! credit note no higher than what its unused paid credit is worth. Any other pack is bought whole.
//! Other test binaries include this file: reach the harness through `meteroid_testkit`, not
//! `crate::harness`.
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use uuid::Uuid;

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, PlanVersionId, SubscriptionId};
use diesel_models::customer_credit::{
    CustomerCreditAccountRow, CustomerCreditEntryRow, CustomerCreditGrantRow,
};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::credit::PLAN_CHANGE_REMOVAL;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum, TaxResolverEnum};
use meteroid_store::domain::price_components::{
    CreditExpiry, PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_changes::{
    ChangeDirection, PackChangeKind, PlanChangeMode,
};
use meteroid_store::domain::{Invoice, InvoicingEntityPatch, LineItemKind, UsagePeriod};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::CreditType;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_testkit::harness::{
    SubscriptionAssertExt, TestEnv, subscription, test_env, test_env_with_seed_and_usage,
};

use crate::data::ids::*;
use crate::meteroid_it::container::SeedLevel;

pub(super) use crate::helpers::credit_plans::{
    LARGE, SCALE, STUDIO, add_component, date, halyard, midnight, new_pack_product, usage_env,
};

const FREE: PlanVersionId = PLAN_VERSION_FREE_ID;

/// Large: Platform Fee 299 + Seats 50 + a EUR 1,000 pack of EUR 1,100 for the period.
pub(super) async fn large(env: &TestEnv) {
    add_component(
        env,
        "Platform Fee",
        LARGE,
        ProductRef::Existing(PRODUCT_PLATFORM_FEE_ID),
        Pricing::Rate { rate: dec!(299) },
    )
    .await;
    add_component(
        env,
        "Seats",
        LARGE,
        ProductRef::Existing(PRODUCT_SEATS_ID),
        Pricing::Slot {
            unit_rate: dec!(50),
            min_slots: None,
            max_slots: None,
        },
    )
    .await;
    add_component(
        env,
        "Large credit",
        LARGE,
        new_pack_product("Large credit"),
        Pricing::Credit {
            unit_price: dec!(1000),
            quantity: 1,
            credit_amount: dec!(1100),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: None,
            rollover: None,
            once: false,
        },
    )
    .await;
}

/// Scale bills its bandwidth at EUR 0.10 a unit in arrears.
pub(super) async fn scale_usage(env: &TestEnv) {
    add_component(
        env,
        "Bandwidth",
        SCALE,
        ProductRef::New {
            name: "Bandwidth".to_owned(),
            fee_type: FeeTypeEnum::Usage,
            fee_structure: FeeStructure::Usage {
                metric_id: METRIC_BANDWIDTH,
                model: UsageModel::PerUnit,
            },
        },
        Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
    )
    .await;
}

pub(super) fn set_usage(
    usage: &MutableMockUsageClient,
    from: NaiveDate,
    to: NaiveDate,
    units: i64,
) {
    let (start, end) = (from.and_time(NaiveTime::MIN), to.and_time(NaiveTime::MIN));
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

/// A French customer taxed at 20% by the EU VAT resolver of the invoicing entity.
async fn french_customer(env: &TestEnv) -> CustomerId {
    use common_domain::country::CountryCode;
    use diesel_models::customers::CustomerRowNew;
    use std::str::FromStr;

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
        .expect("EU VAT resolver");
    let customer_id = CustomerId::new();
    let address = meteroid_store::domain::Address {
        line1: Some("12 rue de Rivoli".to_owned()),
        line2: None,
        city: Some("Paris".to_owned()),
        country: Some(CountryCode::from_str("FR").expect("country")),
        state: None,
        zip_code: Some("75004".to_owned()),
    };
    let mut conn = env.conn().await;
    CustomerRowNew {
        id: customer_id,
        name: "Camille Durand".to_owned(),
        created_at: Some(Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_owned(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: false,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Individual,
        first_name: Some("Camille".to_owned()),
        last_name: Some("Durand".to_owned()),
    }
    .insert(&mut conn)
    .await
    .expect("customer");
    customer_id
}

pub(super) async fn start(
    env: &TestEnv,
    customer: CustomerId,
    plan: PlanVersionId,
    on: NaiveDate,
) -> SubscriptionId {
    subscription()
        .customer(customer)
        .plan_version(plan)
        .start_date(on)
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

/// (name, charge, credit granted) of each pack line, by name.
pub(super) fn pack_lines(invoice: &Invoice) -> Vec<(String, i64, Option<i64>)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::Credit)
        .map(|line| {
            (
                line.name.clone(),
                line.amount_subtotal,
                line.credit_grant_amount,
            )
        })
        .collect();
    lines.sort();
    lines
}

pub(super) fn pack(name: &str, charge: i64, credit: i64) -> (String, i64, Option<i64>) {
    (name.to_owned(), charge, Some(credit))
}

/// (subtotal, promotional credit, tax, total, paid credit applied, amount due).
pub(super) fn money(invoice: &Invoice) -> (i64, i64, i64, i64, i64, i64) {
    (
        invoice.subtotal,
        invoice.promotional_credits,
        invoice.tax_amount,
        invoice.total,
        invoice.applied_credits,
        invoice.amount_due,
    )
}

/// Every grant as stored, in business time (a 2024 grant is not hidden as expired today):
/// "currency SOURCE CLASSIFICATION amount remaining", sorted.
pub(super) async fn grants(env: &TestEnv, customer: CustomerId) -> Vec<String> {
    let mut conn = env.conn().await;
    let accounts = CustomerCreditAccountRow::list_for_customer(&mut conn, TENANT_ID, customer)
        .await
        .expect("accounts");
    let mut lines: Vec<String> =
        CustomerCreditGrantRow::list_for_customer(&mut conn, TENANT_ID, customer)
            .await
            .expect("grants")
            .into_iter()
            .map(|grant| {
                let currency = accounts
                    .iter()
                    .find(|account| account.id == grant.account_id)
                    .map(|account| account.currency.clone())
                    .unwrap_or_default();
                format!(
                    "{currency} {} {} {} {}",
                    grant.source_kind, grant.classification, grant.amount, grant.remaining
                )
            })
            .collect();
    lines.sort();
    lines
}

pub(super) async fn grant_rows(env: &TestEnv, customer: CustomerId) -> Vec<CustomerCreditGrantRow> {
    let mut conn = env.conn().await;
    CustomerCreditGrantRow::list_for_customer(&mut conn, TENANT_ID, customer)
        .await
        .expect("grants")
}

/// A grant's movements as "KIND amount reason?", in order.
async fn movements(env: &TestEnv, grant: &CustomerCreditGrantRow) -> Vec<String> {
    let mut conn = env.conn().await;
    CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, grant.id)
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| match entry.reason {
            Some(reason) if reason == PLAN_CHANGE_REMOVAL => {
                format!("{} {} {reason}", entry.kind, entry.amount)
            }
            _ => format!("{} {}", entry.kind, entry.amount),
        })
        .collect()
}

pub(super) fn the<'a>(
    grants: &'a [CustomerCreditGrantRow],
    source_kind: &str,
    classification: &str,
    amount: i64,
) -> &'a CustomerCreditGrantRow {
    grants
        .iter()
        .find(|g| {
            g.source_kind == source_kind && g.classification == classification && g.amount == amount
        })
        .unwrap_or_else(|| panic!("no {source_kind} {classification} {amount} grant"))
}

/// The grants an invoice drew credit from.
pub(super) async fn drawn_by(
    env: &TestEnv,
    invoice: &Invoice,
) -> Vec<common_domain::ids::CreditGrantId> {
    let mut conn = env.conn().await;
    let mut grants: Vec<_> =
        CustomerCreditEntryRow::list_by_invoice(&mut conn, TENANT_ID, invoice.id)
            .await
            .expect("movements")
            .into_iter()
            .filter(|entry| entry.kind == "DRAW")
            .map(|entry| entry.grant_id)
            .collect();
    grants.sort_by_key(|id| id.as_uuid());
    grants.dedup();
    grants
}

pub(super) async fn pay(env: &TestEnv, invoice: &Invoice) {
    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::new(invoice.amount_due, 2),
            Utc::now().fixed_offset(),
            Some(format!("payment-{}", invoice.id)),
        )
        .await
        .expect("the invoice is paid");
    run_once_invoice_orchestration(
        Arc::new(env.store().clone()),
        Arc::new(env.services().clone()),
    )
    .await;
}

/// The expiry sweep as it runs at `at`, once the settlement grace after a change has passed: the
/// credit a change ended pays its late usage, then lapses.
pub(super) async fn sweep_at(env: &TestEnv, customer: CustomerId, at: DateTime<Utc>) {
    env.store()
        .expire_customer_credit_for(TENANT_ID, customer, at)
        .await
        .expect("expiry sweep");
}

pub(super) async fn invoice(env: &TestEnv, sub_id: SubscriptionId, index: usize) -> Invoice {
    env.get_invoices(sub_id).await.remove(index)
}

pub(super) async fn change_on(
    env: &TestEnv,
    sub_id: SubscriptionId,
    to: PlanVersionId,
    on: NaiveDate,
) {
    env.services()
        .apply_plan_change_immediate_at(Actor::System, sub_id, TENANT_ID, to, vec![], on)
        .await
        .expect("immediate plan change");
}

/// The one finalized credit note of an invoice: (type, subtotal, tax, total) and its lines as
/// (name, subtotal, tax).
pub(super) async fn the_note(
    env: &TestEnv,
    invoice: &Invoice,
) -> ((CreditType, i64, i64, i64), Vec<(String, i64, i64)>) {
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("credit notes");
    assert_eq!(notes.len(), 1, "one credit note on {}", invoice.id);
    let note = &notes[0];
    let mut lines: Vec<_> = note
        .line_items
        .iter()
        .map(|l| (l.name.clone(), l.amount_subtotal, l.tax_amount))
        .collect();
    lines.sort();
    (
        (note.credit_type, note.subtotal, note.tax_amount, note.total),
        lines,
    )
}

/// Revenue stays sane: nothing recognized below zero, never more than was billed. Returns the
/// breakage recognized; None when this test binary derives no revenue.
pub(super) async fn assert_revenue_sane(env: &TestEnv, customer: CustomerId) -> Option<i64> {
    meteroid_testkit::revenue::assert_sane(env.store(), customer).await
}

/// Studio -> Scale on Feb 15 (15 of 29 days left), after a January whose credits rolled over.
/// Scale's pack is billed and granted by that share; Studio's February credits are removed with no
/// money; January's rollover is kept; Studio's fees come back on a credit note that pays Scale's.
#[rstest]
#[tokio::test]
async fn studio_to_scale_mid_period_prorates_scale_s_pack_and_removes_studio_s_period_credit(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    assert_eq!(
        pack_lines(&february),
        vec![pack("Monthly credits", 0, 500_000)]
    );
    pay(&env, &february).await;
    env.store()
        .expire_customer_credit_for(
            TENANT_ID,
            CUST_UBER_ID,
            midnight(date(2024, 2, 1)) + Duration::hours(1),
        )
        .await
        .expect("January's credits roll over");

    change_on(&env, sub_id, SCALE, date(2024, 2, 15)).await;

    // Studio's fees for the 15 days left come back: 2900 × 15/29 = 1500, 1000 × 15/29 = 517.24.
    let ((kind, subtotal, tax, total), lines) = the_note(&env, &february).await;
    assert_eq!(kind, CreditType::CreditToBalance);
    assert_eq!((subtotal.abs(), tax, total.abs()), (2_017, 0, 2_017));
    assert_eq!(
        lines.len(),
        2,
        "fees only: included credit carries no money"
    );

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 3);
    let adjustment = &invoices[2];
    assert_eq!(
        pack_lines(adjustment),
        vec![
            pack("Monthly credit (prorated)", 25_862, 28_448),
            pack("Welcome credit", 0, 5_000),
        ],
        "50000 × 15/29 = 25862.07 for 55000 × 15/29 = 28448.28; the welcome credit whole"
    );
    assert_eq!(
        money(adjustment),
        (32_276, 0, 0, 32_276, 2_017, 30_259),
        "5121 + 1293 + 25862; Studio's fee credit pays the fees, never the pack"
    );
    env.get_subscription(sub_id).await.assert().has_mrr(62_400);

    pay(&env, adjustment).await;
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 15)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        grants(&env, CUST_UBER_ID).await,
        vec![
            "CREDITS INCLUDED_CREDIT PROMOTIONAL 500000 0",
            "CREDITS INCLUDED_CREDIT PROMOTIONAL 500000 0",
            "CREDITS ROLLOVER PROMOTIONAL 100000 100000",
            "EUR CREDIT_NOTE PAID 2017 0",
            "EUR CREDIT_PURCHASE PAID 25862 25862",
            "EUR CREDIT_PURCHASE PROMOTIONAL 2586 2586",
            "EUR INCLUDED_CREDIT PROMOTIONAL 5000 5000",
        ],
        "Paid and Bonus at the share left; Studio's February credits removed, January's rollover kept"
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let feb_credits = rows
        .iter()
        .filter(|g| g.source_kind == "INCLUDED_CREDIT" && g.amount == 500_000)
        .max_by_key(|g| g.effective_at)
        .unwrap();
    assert_eq!(
        movements(&env, feb_credits).await,
        vec![
            "FUND 500000".to_owned(),
            format!("EXPIRY -500000 {PLAN_CHANGE_REMOVAL}")
        ],
        "ended with Studio on Feb 15, lapsed by the sweep after the grace"
    );
    assert_eq!(
        drawn_by(&env, adjustment).await,
        vec![the(&rows, "CREDIT_NOTE", "PAID", 2_017).id],
        "the adjustment draws only the fee credit, never Studio's period credit"
    );
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 25_862);
    assert_eq!(paid.expires_at, Some(midnight(date(2024, 3, 1))));
    assert_eq!(
        the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 2_586).expires_at,
        paid.expires_at
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

/// Scale -> Studio forced on Jan 16 (16 of 31 days left) for a customer taxed at 20%, who used
/// EUR 400 of usage since Jan 1. The usage is settled first (bonus before tax, then paid credit
/// with its tax), so EUR 80 of paid credit is left: the pack's fee credit is capped there, below
/// the 258.06 the unused time is worth. The fees come back with exactly the tax they bore.
#[tokio::test]
async fn scale_to_studio_forced_with_partial_use_caps_the_fee_credit_and_reverses_tax_exactly() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let customer = french_customer(&env).await;
    let sub_id = start(&env, customer, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    assert_eq!(
        pack_lines(&january),
        vec![
            pack("Monthly credit", 50_000, 55_000),
            pack("Welcome credit", 0, 5_000)
        ]
    );
    assert_eq!(
        money(&january),
        (62_400, 0, 2_480, 64_880, 0, 64_880),
        "20% on the fees only: currency credit is taxed when it is used"
    );
    pay(&env, &january).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 4_000);

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;

    let rows = grant_rows(&env, customer).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        movements(&env, bonus).await,
        vec!["FUND 5000", "DRAW -5000"],
        "the bonus pays 50.00 of the usage before tax"
    );
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -42000".to_owned(),
            format!("REVERSAL -8000 {PLAN_CHANGE_REMOVAL}"),
        ],
        "paid credit pays the other 350.00 with its 70.00 tax; the 80.00 left is removed"
    );

    let ((kind, subtotal, tax, total), lines) = the_note(&env, &january).await;
    assert_eq!(kind, CreditType::CreditToBalance);
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -8_000, 0),
            ("Platform Fee".to_owned(), -5_110, -1_022),
            ("Seats".to_owned(), -1_290, -258),
        ],
        "capped at the 80.00 left (not 258.06); 1980 × 5110/9900 and 500 × 1290/2500 of tax"
    );
    assert_eq!(
        (subtotal.abs(), tax.abs(), total.abs()),
        (14_400, 1_280, 15_680)
    );

    let studio = invoice(&env, sub_id, 1).await;
    assert_eq!(
        pack_lines(&studio),
        vec![pack("Monthly credits (prorated)", 0, 258_065)],
        "500000 × 16/31 = 258064.52 credits"
    );
    assert_eq!(
        money(&studio),
        (2_013, 0, 403, 2_416, 2_416, 0),
        "plan credit pays usage only: the note's credit pays Studio's 1497 + 516 and 20% of tax"
    );
    let note_credit = the(&rows, "CREDIT_NOTE", "PAID", 15_680).id;
    assert_eq!(
        drawn_by(&env, &studio).await,
        vec![note_credit],
        "Scale's period credit was removed before the adjustment drew any credit"
    );
    assert_eq!(
        grants(&env, customer).await,
        vec![
            "CREDITS INCLUDED_CREDIT PROMOTIONAL 258065 258065",
            "EUR CREDIT_NOTE PAID 15680 13264",
            "EUR CREDIT_PURCHASE PAID 50000 0",
            "EUR CREDIT_PURCHASE PROMOTIONAL 5000 0",
            "EUR INCLUDED_CREDIT PROMOTIONAL 5000 5000",
        ],
        "the only Paid credit created is the money the note credited: none of it from the bonus"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, customer).await;
    if let Some(breakage) = assert_revenue_sane(&env, customer).await {
        assert_eq!(
            breakage, 0,
            "every unused paid cent was refunded: nothing is breakage"
        );
    }
}

/// Scale -> Studio forced on Jan 16 with no usage: the unused time is worth 258.06 of the fee,
/// less than the 500.00 of paid credit left. The 258.06 refunded is un-billed with the note; the
/// other 241.94 of paid credit is forfeited (breakage); the bonus goes with no money.
#[rstest]
#[tokio::test]
async fn a_forced_downgrade_refunds_the_unused_time_and_books_the_rest_of_the_paid_credit_as_breakage(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;

    let (_, lines) = the_note(&env, &january).await;
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -25_806, 0),
            ("Platform Fee".to_owned(), -5_110, 0),
            ("Seats".to_owned(), -1_290, 0),
        ]
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000)).await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
        ],
        "the rest ends with the change and waits out the settlement grace"
    );
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000)).await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
            format!("EXPIRY -24194 {PLAN_CHANGE_REMOVAL}"),
        ]
    );
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000)).await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {PLAN_CHANGE_REMOVAL}")
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(
            breakage, 24_194,
            "only the forfeited paid credit is breakage; the refunded part is un-billed"
        );
    }
}

/// S1-low with a late event (OQ15): Scale -> Studio forced on Jan 16 after 100.00 of usage; the
/// pack's rest ends with the change instead of lapsing at once. 20.00 more usage dated before the
/// change arrives within the settlement grace: the sweep settles it from the ended pack at the
/// same bound (2000), then lapses the other 171.94 as breakage. February bills the 120.00 in
/// arrears, all taken over from the settlements.
#[tokio::test]
async fn a_late_event_after_an_immediate_plan_change_is_paid_by_the_removed_pack() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_000);

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -5000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
        ],
        "the 191.94 not refunded ends with the change, waiting out the settlement grace"
    );
    assert_eq!(
        paid.lapses_at,
        Some(midnight(date(2024, 1, 16))),
        "ended at the change"
    );

    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_200);
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -5000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
            "DRAW -2000".to_owned(),
            format!("EXPIRY -17194 {PLAN_CHANGE_REMOVAL}"),
        ]
    );
    let welcome = the(&rows, "INCLUDED_CREDIT", "PROMOTIONAL", 5_000);
    assert_eq!(movements(&env, welcome).await, vec!["FUND 5000"]);

    env.process_cycles().await;
    let february = invoice(&env, sub_id, 2).await;
    assert_eq!(
        money(&february),
        (15_900, 5_000, 0, 10_900, 10_900, 0),
        "120.00 of Scale usage (bonus 50.00, paid 70.00 settled) and Studio's 39.00 of fees"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 17_194);
    }
}

/// A period invoice credit paid whole is paid, marked so yet or not: Studio's February invoice is
/// drawn entirely from a top-up (39.00, due 0) and the change to Large on Feb 15 comes before it
/// is marked paid. The fees' unused time comes back to the balance: the 20.17 return to the top-up
/// that paid them, which then pays the adjustment's fees.
#[tokio::test]
async fn a_change_on_an_invoice_credit_paid_but_not_marked_paid_credits_the_balance() {
    use meteroid_store::domain::enums::InvoicePaymentStatus;

    let (env, _usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let top_up = crate::helpers::credit::fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        crate::helpers::credit::Grant::invoice(10_000),
    )
    .await;
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    assert_eq!(money(&february), (3_900, 0, 0, 3_900, 3_900, 0));
    assert_eq!(
        february.payment_status,
        InvoicePaymentStatus::Unpaid,
        "marked paid only once its document is generated"
    );

    change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;

    let ((kind, subtotal, _, _), lines) = the_note(&env, &february).await;
    assert_eq!((kind, subtotal), (CreditType::CreditToBalance, -2_017));
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ],
        "2900 and 1000 × 15/29"
    );
    let mut conn = env.conn().await;
    let top_up_movements: Vec<String> =
        CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, top_up)
            .await
            .expect("movements")
            .into_iter()
            .map(|entry| format!("{} {}", entry.kind, entry.amount))
            .collect();
    drop(conn);
    assert_eq!(
        top_up_movements,
        vec![
            "FUND 10000",
            "DRAW -3900",
            "DRAW_REVERSAL 2017",
            "DRAW -8117"
        ],
        "the credit that paid the fees goes back to it, then pays Large's fees"
    );
    assert!(
        grant_rows(&env, CUST_UBER_ID)
            .await
            .iter()
            .all(|g| g.source_kind != "CREDIT_NOTE"),
        "no cash was paid, so no new balance credit"
    );
    let adjustment = invoice(&env, sub_id, 2).await;
    assert_eq!(
        money(&adjustment),
        (69_776, 0, 0, 69_776, 8_117, 61_659),
        "15466 + 2586 of fees and 51724 of pack for 15 of 29 days"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Disagreement 10: the balance credit a backdated change's note returns is dated at the change,
/// not when the change was entered.
#[rstest]
#[tokio::test]
async fn a_backdated_change_dates_the_note_credit_at_the_change(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let note_credit = the(&rows, "CREDIT_NOTE", "PAID", 32_206);
    assert_eq!(note_credit.effective_at, midnight(date(2024, 1, 16)));
    let mut conn = env.conn().await;
    let funded = CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, note_credit.id)
        .await
        .expect("movements")
        .into_iter()
        .find(|entry| entry.kind == "FUND")
        .expect("funded");
    assert_eq!(funded.occurred_at, midnight(date(2024, 1, 16)));
}

/// `once` and voids: a void that takes the welcome credit back lets the next invoice bill it again;
/// one after the credit lapsed (nothing taken back) does not, so it is never granted twice.
#[rstest]
#[tokio::test]
async fn a_voided_welcome_credit_is_billed_again_only_when_the_void_took_it_back(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    for (customer, lapsed_first) in [(CUST_UBER_ID, false), (CUST_SPOTIFY_ID, true)] {
        let sub_id = start(&env, customer, SCALE, date(2024, 1, 1)).await;
        let january = invoice(&env, sub_id, 0).await;
        let welcome = the(
            &grant_rows(&env, customer).await,
            "INCLUDED_CREDIT",
            "PROMOTIONAL",
            5_000,
        )
        .clone();
        if lapsed_first {
            crate::helpers::credit::expire_grant(env.store(), welcome.id).await;
            sweep_at(&env, customer, Utc::now()).await;
        }
        env.try_void_invoice(january.id).await.expect("void");
        let expected = if lapsed_first {
            vec!["FUND 5000", "EXPIRY -5000"]
        } else {
            vec!["FUND 5000", "REVERSAL -5000"]
        };
        assert_eq!(movements(&env, &welcome).await, expected);

        env.process_cycles().await;
        let february = invoice(&env, sub_id, 1).await;
        let mut lines = vec![pack("Monthly credit", 50_000, 55_000)];
        if !lapsed_first {
            lines.push(pack("Welcome credit", 0, 5_000));
        }
        assert_eq!(pack_lines(&february), lines, "lapsed first: {lapsed_first}");
    }
}

/// The welcome credit billed on an immediate change's adjustment invoice counts: the renewal
/// never bills it a second time.
#[rstest]
#[tokio::test]
async fn the_welcome_credit_billed_on_an_adjustment_is_not_billed_at_renewal(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;
    let adjustment = invoice(&env, sub_id, 1).await;
    assert!(pack_lines(&adjustment).contains(&pack("Welcome credit", 0, 5_000)));
    pay(&env, &adjustment).await;

    env.process_cycles().await;
    let february = invoice(&env, sub_id, 2).await;
    assert_eq!(
        pack_lines(&february),
        vec![pack("Monthly credit", 50_000, 55_000)]
    );
    assert_eq!(
        grant_rows(&env, CUST_UBER_ID)
            .await
            .iter()
            .filter(|g| g.source_kind == "INCLUDED_CREDIT" && g.amount == 5_000)
            .count(),
        1,
        "one welcome credit"
    );
}

/// Spend the whole pack, then change plan: the unused time is worth 258.06 but no paid credit is
/// left, so the fee credit is 0. Only the fees come back.
#[tokio::test]
async fn spending_the_pack_then_changing_plan_refunds_nothing_of_it() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 6_000);

    change_on(&env, sub_id, LARGE, date(2024, 1, 16)).await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000)).await,
        vec!["FUND 50000", "DRAW -50000"],
        "600.00 of usage: 50.00 of bonus, then all 500.00 of paid credit"
    );
    let (_, lines) = the_note(&env, &january).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -5_110, 0),
            ("Seats".to_owned(), -1_290, 0),
        ],
        "no fee credit for a pack already spent"
    );
    let adjustment = invoice(&env, sub_id, 1).await;
    assert_eq!(
        pack_lines(&adjustment),
        vec![pack("Large credit (prorated)", 51_613, 56_774)],
        "100000 × 16/31 = 51612.90 for 110000 × 16/31 = 56774.19"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

/// Scale -> Large on Jan 16 while January's Scale invoice is unpaid: no credit was granted yet, so
/// the pack's fee is credited by the unused time on a note that cancels that debt. Paid later, the
/// invoice grants only what was paid (55000 less its credited share), ending with the change: it
/// pays January's usage up to Jan 16, never Large's period, then lapses.
#[tokio::test]
async fn a_change_while_the_pack_is_unpaid_lets_its_payment_grant_only_the_share_kept() {
    let (env, _usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;

    change_on(&env, sub_id, LARGE, date(2024, 1, 16)).await;

    let january = invoice(&env, sub_id, 0).await;
    let ((kind, _, _, total), lines) = the_note(&env, &january).await;
    assert_eq!(kind, CreditType::DebtCancellation);
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -25_806, 0),
            ("Platform Fee".to_owned(), -5_110, 0),
            ("Seats".to_owned(), -1_290, 0),
        ]
    );
    assert_eq!(total.abs(), 32_206);
    assert_eq!(january.amount_due, 62_400 - 32_206);
    assert!(
        grants(&env, CUST_UBER_ID)
            .await
            .iter()
            .all(|g| !g.contains("CREDIT_PURCHASE")),
        "nothing granted before payment"
    );

    pay(&env, &january).await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    // Cancelled 55000 × 25806/50000 = 28386.6 → 28387; cost 50000 - 25806.
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 24_194);
    let bonus = the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 2_419);
    for grant in [paid, bonus] {
        assert_eq!(grant.effective_at, midnight(date(2024, 1, 1)));
        assert_eq!(
            grant.expires_at,
            Some(midnight(date(2024, 1, 16))),
            "the change ended January's credit early"
        );
    }
    env.store()
        .expire_customer_credit_for(
            TENANT_ID,
            CUST_UBER_ID,
            midnight(date(2024, 1, 16)) + Duration::hours(1),
        )
        .await
        .expect("sweep");
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 24_194)).await,
        vec!["FUND 24194", "EXPIRY -24194"],
        "no usage up to Jan 16: it lapses, paying nothing of Large's period"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

/// A pack whose credit lasts 12 months is a purchase: changing away leaves it untouched (no fee
/// credit, nothing removed), changing to it bills it whole.
#[rstest]
#[tokio::test]
async fn a_pack_with_its_own_duration_is_never_prorated_nor_removed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::Month(12)).await;
    let scale_sub = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, scale_sub, 0).await;
    pay(&env, &january).await;

    change_on(&env, scale_sub, STUDIO, date(2024, 1, 16)).await;

    let (_, lines) = the_note(&env, &january).await;
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        vec!["Platform Fee", "Seats"],
        "the 12-month pack keeps its fee"
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    for grant in rows.iter().filter(|g| g.source_kind == "CREDIT_PURCHASE") {
        assert!(
            movements(&env, grant)
                .await
                .iter()
                .all(|m| !m.contains(PLAN_CHANGE_REMOVAL)),
            "the 12-month credit is kept"
        );
    }

    let studio_sub = start(&env, CUST_SPOTIFY_ID, STUDIO, date(2024, 1, 1)).await;
    change_on(&env, studio_sub, SCALE, date(2024, 1, 16)).await;
    assert_eq!(
        pack_lines(&invoice(&env, studio_sub, 1).await),
        vec![
            pack("Monthly credit", 50_000, 55_000),
            pack("Welcome credit", 0, 5_000)
        ],
        "billed and granted whole"
    );
}

/// Scale -> Studio on Jan 10, back to Scale on Jan 20: the welcome credit (`once`) is never billed
/// again, and the second change credits what the first adjustment billed for Studio.
#[rstest]
#[tokio::test]
async fn a_round_trip_never_grants_the_once_credit_again(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;

    change_on(&env, sub_id, STUDIO, date(2024, 1, 10)).await;
    let to_studio = invoice(&env, sub_id, 1).await;
    // 22 of 31 days left.
    assert_eq!(
        pack_lines(&to_studio),
        vec![pack("Monthly credits (prorated)", 0, 354_839)]
    );
    assert_eq!(
        to_studio.amount_due, 0,
        "Scale's fee credit and welcome credit pay it"
    );

    change_on(&env, sub_id, SCALE, date(2024, 1, 20)).await;
    let back = invoice(&env, sub_id, 2).await;
    // 12 of 31 days left: 50000 × 12/31 = 19354.84, 55000 × 12/31 = 21290.32.
    assert_eq!(
        pack_lines(&back),
        vec![pack("Monthly credit (prorated)", 19_355, 21_290)],
        "no welcome credit the second time"
    );
    let (_, lines) = the_note(&env, &to_studio).await;
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.0.as_str(), l.1))
            .collect::<Vec<_>>(),
        vec![
            ("Platform Fee (prorated)", -1_123),
            ("Seats (prorated)", -387)
        ],
        "Studio's fees for 12 of 31 days, credited on the adjustment that billed them"
    );
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 20)) + Duration::hours(1),
    )
    .await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let studio_credits = the(&rows, "INCLUDED_CREDIT", "PROMOTIONAL", 354_839);
    assert_eq!(
        movements(&env, studio_credits).await,
        vec![
            "FUND 354839".to_owned(),
            format!("EXPIRY -354839 {PLAN_CHANGE_REMOVAL}")
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

/// A subscription starting on Jan 16 on a period anchored on the 1st bills its pack tied to the
/// period for 16 of 31 days and grants the same share; the welcome credit is whole.
#[rstest]
#[tokio::test]
async fn a_mid_period_start_prorates_the_pack_fee_and_its_credit(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    for (plan, customer, expected) in [
        (
            SCALE,
            CUST_UBER_ID,
            vec![
                pack("Monthly credit", 25_806, 28_387),
                pack("Welcome credit", 0, 5_000),
            ],
        ),
        (
            STUDIO,
            CUST_SPOTIFY_ID,
            vec![pack("Monthly credits", 0, 258_065)],
        ),
    ] {
        let mut request = subscription()
            .customer(customer)
            .plan_version(plan)
            .start_date(date(2024, 1, 16))
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
        let first = invoice(&env, sub_id, 0).await;
        assert_eq!(pack_lines(&first), expected);
        assert!(
            first
                .line_items
                .iter()
                .filter(|l| l.kind == LineItemKind::Credit && l.amount_subtotal > 0)
                .all(|l| l.is_prorated)
        );
    }
}

/// The direction compares fees, pack fees, period credit and rollover per month: anything less, or
/// a credit unit gone, is a downgrade. The preview of an immediate change shows the new pack's
/// charge and the old pack's credit removed, from a dry run that changes nothing.
#[rstest]
#[tokio::test]
async fn the_direction_counts_packs_and_the_preview_shows_them(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let scale_sub = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let studio_sub = start(&env, CUST_SPOTIFY_ID, STUDIO, date(2024, 1, 1)).await;
    let free_sub = start(&env, CUST_COMODO_ID, FREE, date(2024, 1, 1)).await;
    let direction = |sub_id, to| {
        let env = &env;
        async move {
            env.services()
                .preview_plan_change(sub_id, TENANT_ID, to, vec![], None)
                .await
                .expect("preview")
                .change_direction
        }
    };
    assert_eq!(
        direction(scale_sub, STUDIO).await,
        ChangeDirection::Downgrade
    );
    assert_eq!(
        direction(studio_sub, SCALE).await,
        ChangeDirection::Downgrade,
        "Studio's credit unit would be removed"
    );
    assert_eq!(direction(free_sub, SCALE).await, ChangeDirection::Upgrade);

    let today = Utc::now().date_naive();
    let previewed_sub = start(&env, CUST_UBER_ID, STUDIO, today).await;
    let preview = env
        .services()
        .preview_plan_change(
            previewed_sub,
            TENANT_ID,
            SCALE,
            vec![],
            Some(PlanChangeMode::Immediate),
        )
        .await
        .expect("immediate preview");
    let proration = preview.proration.expect("proration");
    let kinds: Vec<_> = proration
        .pack_changes
        .iter()
        .map(|c| (c.kind, c.name.clone(), c.promotional))
        .collect();
    assert!(kinds.contains(&(PackChangeKind::Charge, "Monthly credit".to_owned(), false)));
    assert!(kinds.contains(&(PackChangeKind::Charge, "Welcome credit".to_owned(), true)));
    assert!(
        proration
            .pack_changes
            .iter()
            .any(|c| c.kind == PackChangeKind::CreditRemoved && c.credit_minor == 500_000),
        "Studio's credits for the period, all unused"
    );
    assert!(
        grants(&env, CUST_UBER_ID)
            .await
            .contains(&"CREDITS INCLUDED_CREDIT PROMOTIONAL 500000 500000".to_owned()),
        "the preview changed nothing"
    );
}

/// A scheduled downgrade still applies at the renewal: Scale's credit lapses on its own at the
/// period's end, nothing is refunded or removed, Studio's credits come with its first invoice.
#[rstest]
#[tokio::test]
async fn a_scheduled_downgrade_at_renewal_refunds_nothing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, STUDIO, vec![])
        .await
        .expect("scheduled");
    env.process_cycles().await;
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, STUDIO);
    assert_eq!(
        pack_lines(&invoice(&env, sub_id, 1).await),
        vec![pack("Monthly credits", 0, 500_000)]
    );
    assert!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, january.id)
            .await
            .expect("notes")
            .is_empty()
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(paid.expires_at, Some(midnight(date(2024, 2, 1))));
    assert_eq!(movements(&env, paid).await, vec!["FUND 50000"]);
}

/// Rule: a pack tied to the period ends with the period it was bought for, however late it is
/// paid. Paid on time or days late, it expires at the period's end; paid after that, it still pays
/// that period's usage at its expiry, then lapses, and never a later period's.
#[tokio::test]
async fn a_period_pack_ends_with_its_period_however_late_it_is_paid() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let today = Utc::now().date_naive();

    for (customer, start_on) in [
        (CUST_UBER_ID, today),
        (CUST_SPOTIFY_ID, today - Duration::days(3)),
    ] {
        let sub_id = start(&env, customer, SCALE, start_on).await;
        let period_end = env
            .get_subscription(sub_id)
            .await
            .current_period_end
            .expect("period end");
        pay(&env, &invoice(&env, sub_id, 0).await).await;
        let rows = grant_rows(&env, customer).await;
        for grant in rows.iter().filter(|g| g.source_kind == "CREDIT_PURCHASE") {
            assert_eq!(
                grant.expires_at,
                Some(midnight(period_end)),
                "paid {} days into its period",
                (today - start_on).num_days()
            );
        }
    }

    // Paid 40 days after the period opened: January-like period ended about 10 days ago.
    let start_on = today - Duration::days(40);
    let sub_id = start(&env, CUST_COMODO_ID, SCALE, start_on).await;
    let period_end = env
        .get_subscription(sub_id)
        .await
        .current_period_end
        .expect("period end");
    set_usage(&usage, start_on, period_end, 1_000);
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let rows = grant_rows(&env, CUST_COMODO_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(paid.effective_at, midnight(start_on));
    assert_eq!(paid.expires_at, Some(midnight(period_end)));
    env.store()
        .expire_customer_credit(200)
        .await
        .expect("expiry sweep");
    let rows = grant_rows(&env, CUST_COMODO_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000)).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000)).await,
        vec!["FUND 50000", "DRAW -5000", "EXPIRY -45000"],
        "100.00 of usage: 50.00 of bonus, 50.00 of paid credit, then the rest lapses"
    );
    env.process_cycles().await;
    let rows = grant_rows(&env, CUST_COMODO_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000)).await,
        vec!["FUND 50000", "DRAW -5000", "EXPIRY -45000"],
        "the next period's invoice takes the settlement over; nothing more is drawn"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_COMODO_ID).await;
}

/// Scale and Studio renew on one consolidated invoice, which carries no subscription. Changing
/// the Scale subscription on Feb 15 (15 of 29 days left) finds its pack on that invoice: the fee
/// comes back on a note against it, the pack's credit is removed, Studio's credit is untouched.
#[rstest]
#[tokio::test]
async fn a_change_reaches_the_pack_billed_on_a_consolidated_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let scale_sub = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let studio_sub = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, scale_sub, 0).await).await;
    pay(&env, &invoice(&env, studio_sub, 0).await).await;
    env.process_cycles().await;
    let parent = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .find(|i| i.subscription_id.is_none())
        .expect("the consolidated February invoice");
    assert_eq!(
        pack_lines(&parent),
        vec![
            pack("Monthly credit", 50_000, 55_000),
            pack("Monthly credits", 0, 500_000)
        ]
    );
    pay(&env, &parent).await;

    change_on(&env, scale_sub, STUDIO, date(2024, 2, 15)).await;
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 15)) + Duration::hours(1),
    )
    .await;

    let (_, lines) = the_note(&env, &parent).await;
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -25_862, 0),
            ("Platform Fee".to_owned(), -5_121, 0),
            ("Seats".to_owned(), -1_293, 0),
        ],
        "Scale's lines only, by the share of the period left"
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let february_paid = rows
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.invoice_id == Some(parent.id)
        })
        .expect("February's Scale pack");
    assert_eq!(
        movements(&env, february_paid).await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -25862 {PLAN_CHANGE_REMOVAL}"),
            format!("EXPIRY -24138 {PLAN_CHANGE_REMOVAL}"),
        ]
    );
    let studio_credits = rows
        .iter()
        .find(|g| g.source_kind == "INCLUDED_CREDIT" && g.invoice_id == Some(parent.id))
        .expect("February's Studio credits of the other subscription");
    assert_eq!(movements(&env, studio_credits).await, vec!["FUND 500000"]);
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

fn succeeded_payload(tx: common_domain::ids::PaymentTransactionId, amount: i64) -> String {
    format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_{tx}","amount":{amount},"currency":"EUR"}}"#,
        tx = tx.as_base62(),
    )
}

fn failed_payload(tx: common_domain::ids::PaymentTransactionId) -> String {
    format!(
        r#"{{"id":"evt_fail_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"pi_{tx}"}}"#,
        tx = tx.as_base62(),
    )
}

async fn notes_on(env: &TestEnv, invoice: &Invoice) -> usize {
    env.store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("credit notes")
        .len()
}

async fn checkout(
    env: &TestEnv,
    session_id: common_domain::ids::CheckoutSessionId,
) -> meteroid_store::domain::checkout_sessions::CheckoutCompletionResult {
    env.services()
        .complete_checkout(
            TENANT_ID,
            session_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            30_193,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout")
}

async fn assert_untouched(env: &TestEnv, sub_id: SubscriptionId, january: &Invoice) {
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, STUDIO);
    assert_eq!(notes_on(env, january).await, 0, "no credit note");
    assert_eq!(
        grants(env, CUST_UBER_ID).await,
        vec!["CREDITS INCLUDED_CREDIT PROMOTIONAL 500000 500000"],
        "nothing granted or removed"
    );
    assert_eq!(env.get_invoices(sub_id).await.len(), 1, "no adjustment yet");
}

/// A portal upgrade whose payment needs 3DS changes nothing until the payment settles: no plan
/// change, no credit note, no credit granted or removed, and no other change accepted meanwhile.
/// The pending payment is found again on a retry; once it fails, the retried checkout applies.
#[rstest]
#[tokio::test]
async fn a_portal_upgrade_waits_for_its_payment(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::MockChargeBehavior;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    let change_date = date(2024, 1, 16);
    let quote = env
        .services()
        .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, SCALE, change_date)
        .await
        .expect("quote");
    // 5110 + 1290 + 25806 charged; Studio's 1497 + 516 back on the balance pay the fees.
    assert_eq!(
        (quote.total, quote.applied_credits, quote.amount_due),
        (32_206, 2_013, 30_193)
    );
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            change_date,
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: pending,
        ..
    } = checkout(&env, session.id).await
    else {
        panic!("the payment is pending")
    };
    assert_untouched(&env, sub_id, &january).await;
    let refused = env
        .services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SCALE,
            vec![],
            change_date,
        )
        .await
        .expect_err("another change waits for the payment");
    assert!(
        format!("{refused:?}").contains("awaiting its payment"),
        "{refused:?}"
    );
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: again, ..
    } = checkout(&env, session.id).await
    else {
        panic!("a retry finds the pending payment")
    };
    assert_eq!(again.id, pending.id);

    env.deliver_mock_webhook(failed_payload(pending.id).as_bytes())
        .await;
    assert_untouched(&env, sub_id, &january).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let CheckoutCompletionResult::Completed { transaction, .. } = checkout(&env, session.id).await
    else {
        panic!("the retried checkout completes")
    };
    assert!(transaction.is_some());
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, SCALE);
    assert_eq!(notes_on(&env, &january).await, 1);
    let adjustment = invoice(&env, sub_id, 1).await;
    assert_eq!(
        (
            adjustment.total,
            adjustment.applied_credits,
            adjustment.amount_due
        ),
        (32_206, 2_013, 0),
        "paid by the 30193 charged"
    );
}

/// What an earlier release issued for Starter -> Pro on the first day: one netted adjustment
/// invoice, its 85.00 payment pending with the plan version it applies.
async fn legacy_pending_adjustment(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> (Invoice, common_domain::ids::PaymentTransactionId) {
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::domain::{InvoiceNew, InvoicePaymentStatus, LineItem};
    use meteroid_store::repositories::InvoiceInterface;
    use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;

    let subscription = env.get_subscription(sub_id).await;
    let today = subscription.current_period_start;
    let period_end = subscription.current_period_end.expect("period end");
    let line = |name: &str, amount: i64| LineItem {
        local_id: Uuid::now_v7().to_string(),
        name: name.to_owned(),
        charge_type: None,
        amount_subtotal: amount,
        tax_rate: Decimal::ZERO,
        taxable_amount: amount.max(0),
        tax_amount: 0,
        amount_total: amount,
        tax_details: vec![],
        quantity: None,
        unit_price: None,
        start_date: today,
        end_date: period_end,
        sub_lines: vec![],
        is_prorated: true,
        price_component_id: None,
        sub_component_id: None,
        sub_add_on_id: None,
        product_id: None,
        metric_id: None,
        description: None,
        group_by_dimensions: None,
        kind: LineItemKind::Regular,
        credit_grant_amount: None,
        credit_grant_expiry: None,
        credit_grant_included: false,
        taxed_at_purchase: false,
        tax_inclusive_quote: None,
        credit_unit: None,
        credit_grant_unit: None,
        credit_grant_rollover: None,
        promotional_credit: None,
        subscription_id: None,
        usage_ends_at: None,
    };
    let customer = env.get_customer(CUST_UBER_ID).await;
    let entity = env
        .store()
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .expect("entity");
    let legacy = env
        .store()
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            subscription_id: Some(sub_id),
            cycle_index: None,
            currency: "EUR".to_owned(),
            invoice_number: "draft".to_owned(),
            line_items: vec![
                line("Platform Fee (prorated)", 9_900),
                line("Seats (prorated)", 2_500),
                line("Platform Fee (credit)", -2_900),
                line("Seats (credit)", -1_000),
            ],
            coupons: vec![],
            data_updated_at: None,
            invoice_date: today,
            billing_period_start: today,
            tax_inclusive: false,
            plan_version_id: Some(STUDIO),
            invoice_type: InvoiceType::Adjustment,
            finalized_at: None,
            subtotal: 8_500,
            subtotal_recurring: 0,
            discount: 0,
            tax_amount: 0,
            total: 8_500,
            amount_due: 8_500,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: customer.into(),
            seller_details: entity.into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown: vec![],
            manual: false,
            invoicing_entity_id: INVOICING_ENTITY_ID,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: vec![],
            custom_properties: serde_json::Value::Null,
        })
        .await
        .expect("legacy adjustment");
    env.exempt_invoice_from(
        "I23",
        legacy.id,
        "an earlier release netted credits as negative lines with no taxable amount",
    );
    env.manual_finalize_invoice(legacy.id)
        .await
        .expect("finalized");
    let pending = {
        let mut conn = env.conn().await;
        PaymentTransactionRowNew {
            id: common_domain::ids::PaymentTransactionId::new(),
            tenant_id: TENANT_ID,
            invoice_id: Some(legacy.id),
            provider_transaction_id: None,
            amount: 8_500,
            currency: "EUR".to_owned(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: Some(PLAN_VERSION_PRO_ID),
            next_action: None,
            initiated_by_customer_id: Some(CUST_UBER_ID),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("pending payment")
    };

    (legacy, pending.id)
}

/// An adjustment invoice an earlier release issued nets the old plan's credits as negative lines;
/// its pending payment settling after the deploy applies the change without crediting them again,
/// and a redelivery of that settlement changes nothing.
#[rstest]
#[tokio::test]
async fn a_legacy_netted_adjustment_settling_later_credits_nothing_twice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, Utc::now().date_naive()).await;
    let starter = invoice(&env, sub_id, 0).await;
    pay(&env, &starter).await;
    let (legacy, pending) = legacy_pending_adjustment(&env, sub_id).await;
    let legacy = legacy.id;

    env.deliver_mock_webhook(succeeded_payload(pending, 8_500).as_bytes())
        .await;

    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_PRO_ID
    );
    assert_eq!(
        notes_on(&env, &starter).await,
        0,
        "the 3900 netted on the legacy invoice is not credited again"
    );
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 0);

    // The settlement delivered again finds its change applied: nothing is credited back.
    let settled = env
        .store()
        .last_settled_payment_tx_by_invoice_id(TENANT_ID, legacy)
        .await
        .expect("payments")
        .expect("the settled payment");
    env.services()
        .on_payment_transaction_settled(settled.into())
        .await
        .expect("redelivered");
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_PRO_ID
    );
    assert_eq!(notes_on(&env, &starter).await, 0);
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 0);
}

/// An unpaid pack carried over unchanged on Jan 10 and removed on Jan 16, paid later: its credit
/// ends on Jan 16, the change that removed it, not at the period's end.
#[rstest]
#[tokio::test]
async fn a_late_payment_ends_with_the_change_that_removed_the_pack(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let scale_pack = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Monthly credit")
        .and_then(|c| c.product_id)
        .expect("Scale's pack product");
    add_component(
        &env,
        "Monthly credit",
        LARGE,
        ProductRef::Existing(scale_pack),
        Pricing::Credit {
            unit_price: dec!(500),
            quantity: 1,
            credit_amount: dec!(550),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: None,
            rollover: None,
            once: false,
        },
    )
    .await;

    change_on(&env, sub_id, LARGE, date(2024, 1, 10)).await;
    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;
    let january = invoice(&env, sub_id, 0).await;
    // 62400 less Scale's fees for 22 of 31 days (7026 + 1774), less 25806 of the pack for 16 of 31.
    assert_eq!(january.amount_due, 62_400 - 8_800 - 25_806);
    pay(&env, &january).await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    for (classification, amount) in [("PAID", 24_194), ("PROMOTIONAL", 2_419)] {
        let grant = the(&rows, "CREDIT_PURCHASE", classification, amount);
        assert_eq!(
            grant.expires_at,
            Some(midnight(date(2024, 1, 16))),
            "carried on Jan 10, removed on Jan 16"
        );
    }
}

/// Removed by a plan change, a grant never pays again: usage it settled up to the change comes
/// back to it when the invoice that took the settlement over is voided, and lapses at once.
#[tokio::test]
async fn value_given_back_to_a_removed_grant_lapses_at_once() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_000);

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -5000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
            format!("EXPIRY -19194 {PLAN_CHANGE_REMOVAL}"),
        ],
        "100.00 of usage: 50.00 of bonus, 50.00 paid; 450.00 left, 258.06 of it refunded"
    );

    env.process_cycles().await;
    let february = invoice(&env, sub_id, 2).await;
    env.try_void_invoice(february.id).await.expect("void");
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(paid.remaining, 0, "never spendable again");
    assert_eq!(
        movements(&env, paid).await[4..].to_vec(),
        vec![
            "DRAW_REVERSAL 5000".to_owned(),
            format!("EXPIRY -5000 {PLAN_CHANGE_REMOVAL}"),
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Within the settlement grace of the change, value a void gives back to the removed grant stays
/// for the sweep, which settles the late usage dated before the change from it, then lapses it.
#[tokio::test]
async fn value_given_back_within_the_grace_of_a_change_waits_for_the_sweep() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_000);
    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 2).await;

    // The change is backdated: a grace reaching past today puts the void inside it.
    env.set_consolidate_and_grace(false, 100_000).await;
    env.try_void_invoice(february.id).await.expect("void");
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -5000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
            "DRAW_REVERSAL 5000".to_owned(),
        ],
        "nothing lapses before the sweep"
    );
    assert_eq!(paid.remaining, 24_194);

    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(paid.remaining, 0, "the sweep lapses it");
    assert!(
        movements(&env, paid)
            .await
            .last()
            .is_some_and(|last| last.starts_with("EXPIRY -")
                && last.ends_with(PLAN_CHANGE_REMOVAL))
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Half of a pack's fee cancelled before it was paid, then part of what was paid spent: the
/// cap reads what the line still bills (250.00) and the paid credit it funded (250.00).
#[tokio::test]
async fn the_cap_reads_what_was_paid_after_an_earlier_credit_note() {
    use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditLineItem};

    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    let pack_line = january
        .line_items
        .iter()
        .find(|l| l.name == "Monthly credit")
        .expect("pack line")
        .local_id
        .clone();
    env.store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: january.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: pack_line,
                    amount_cents: 25_000,
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("half of the pack cancelled");
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_500);

    change_on(&env, sub_id, STUDIO, date(2024, 1, 16)).await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 25_000);
    assert_eq!(
        movements(&env, paid).await,
        vec![
            "FUND 25000".to_owned(),
            "DRAW -12500".to_owned(),
            format!("REVERSAL -12500 {PLAN_CHANGE_REMOVAL}"),
        ],
        "150.00 of usage: 25.00 of bonus, 125.00 paid; the 125.00 left is all refunded"
    );
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, january.id)
        .await
        .expect("notes");
    let pack_credit: i64 = notes
        .iter()
        .flat_map(|n| n.line_items.iter())
        .filter(|l| l.name == "Monthly credit")
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(
        pack_credit,
        -25_000 - 12_500,
        "the plan change credits min(250 × 16/31, 250 × 125/250) = 125.00"
    );
}

/// A quarterly pack on a monthly subscription was billed in January; changing plan on Feb 16
/// finds it: 45 of its 91 days left, nothing spent.
#[rstest]
#[tokio::test]
async fn a_pack_billed_on_a_longer_cadence_is_found(#[future] test_env: TestEnv) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    crate::helpers::plans::as_draft(
        env.store(),
        STUDIO,
        env.store().create_price_component_from_ref(
            "Quarterly credit".to_owned(),
            new_pack_product("Quarterly credit"),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Quarterly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(900),
                    quantity: 1,
                    credit_amount: dec!(900),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            })],
            STUDIO,
            TENANT_ID,
        ),
    )
    .await
    .expect("quarterly pack");
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycles().await;
    pay(&env, &invoice(&env, sub_id, 1).await).await;

    change_on(&env, sub_id, SCALE, date(2024, 2, 16)).await;
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 16)) + Duration::hours(1),
    )
    .await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(
        movements(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 90_000)).await,
        vec![
            "FUND 90000".to_owned(),
            format!("REVERSAL -44505 {PLAN_CHANGE_REMOVAL}"),
            format!("EXPIRY -45495 {PLAN_CHANGE_REMOVAL}"),
        ],
        "90000 × 45/91 = 44505.49 refunded"
    );
}

/// (name, subtotal, end) of every line, sorted.
fn lines_of(invoice: &Invoice) -> Vec<(String, i64, NaiveDate)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .map(|line| (line.name.clone(), line.amount_subtotal, line.end_date))
        .collect();
    lines.sort();
    lines
}

async fn reread(env: &TestEnv, invoice: &Invoice) -> Invoice {
    env.get_detailed_invoice(invoice.id).await.invoice
}

/// A plan version's minimum commitment over all its components.
async fn plan_minimum(env: &TestEnv, plan_version_id: PlanVersionId, amount: i64) {
    use meteroid_store::domain::plan_minimums::{MinimumScope, PlanMinimumNew};
    let mut conn = env.conn().await;
    PlanMinimumNew {
        plan_version_id,
        amount_subunit: amount,
        scope: MinimumScope::AllComponents,
    }
    .to_row()
    .upsert(&mut conn)
    .await
    .expect("plan minimum");
}

/// The one plan-change note of an invoice, its lines as (name, subtotal, taxable, tax, total).
async fn note_lines(env: &TestEnv, invoice: &Invoice) -> Vec<(String, i64, i64, i64, i64)> {
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("credit notes");
    assert_eq!(notes.len(), 1, "one credit note on {}", invoice.id);
    let mut lines: Vec<_> = notes[0]
        .line_items
        .iter()
        .map(|l| {
            (
                l.name.clone(),
                l.amount_subtotal,
                l.taxable_amount,
                l.tax_amount,
                l.amount_total,
            )
        })
        .collect();
    lines.sort();
    lines
}

async fn feb_included_credits(env: &TestEnv, invoice: &Invoice) -> CustomerCreditGrantRow {
    grant_rows(env, invoice.customer_id)
        .await
        .into_iter()
        .find(|g| g.source_kind == "INCLUDED_CREDIT" && g.invoice_id == Some(invoice.id))
        .expect("the period's included credits")
}

/// Studio -> Large on Feb 15 while February's invoice waits out its grace period: nothing
/// happens to the draft. Once it is issued, its fees for the 15 days left come back on a note
/// (2900 and 1000 × 15/29) and its included credits, granted then, are removed, as for an issued
/// invoice. The sweep finds nothing more to do.
#[rstest]
#[tokio::test]
async fn a_change_during_grace_is_settled_once_the_draft_is_issued(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::InvoiceStatusEnum;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    let february = invoice(&env, sub_id, 1).await;
    let whole = lines_of(&february);

    change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;
    let draft = reread(&env, &february).await;
    assert_eq!(draft.status, InvoiceStatusEnum::Draft);
    assert_eq!(lines_of(&draft), whole, "a draft is never changed");
    assert_eq!(notes_on(&env, &february).await, 0);

    env.process_due_events().await;
    let february = reread(&env, &february).await;
    assert_eq!(february.status, InvoiceStatusEnum::Finalized);
    let ((kind, subtotal, tax, total), lines) = the_note(&env, &february).await;
    assert_eq!(
        (kind, subtotal, tax, total),
        (CreditType::DebtCancellation, -2_017, 0, -2_017)
    );
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ]
    );
    assert_eq!(february.amount_due, 3_900 - 2_017);
    let credits = feb_included_credits(&env, &february).await;
    assert_eq!(credits.expires_at, Some(midnight(date(2024, 2, 15))));
    assert_eq!(movements(&env, &credits).await, vec!["FUND 500000"]);
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 15)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        movements(&env, &credits).await,
        vec![
            "FUND 500000".to_owned(),
            format!("EXPIRY -500000 {PLAN_CHANGE_REMOVAL}")
        ]
    );

    assert_eq!(
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("sweep"),
        0
    );
    assert_eq!(notes_on(&env, &february).await, 1);
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_revenue_sane(&env, CUST_UBER_ID).await;
}

/// Without auto-advance, February's draft bills January's 100.00 of bandwidth, Scale's fees and
/// its pack. Scale -> Studio on Feb 15 leaves the draft as is through a refresh; issuing it
/// credits 15 of 29 days of the fees and of the unpaid pack. Paying the draft then grants only the
/// pack's share kept, ending on the change.
#[tokio::test]
async fn a_change_without_auto_advance_settles_the_draft_once_issued() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(SCALE)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), 1_000);
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    let whole = vec![
        ("Bandwidth".to_owned(), 10_000, date(2024, 2, 1)),
        ("Monthly credit".to_owned(), 50_000, date(2024, 3, 1)),
        ("Platform Fee".to_owned(), 9_900, date(2024, 3, 1)),
        ("Seats".to_owned(), 2_500, date(2024, 3, 1)),
    ];
    assert_eq!(lines_of(&february), whole);

    change_on(&env, sub_id, STUDIO, date(2024, 2, 15)).await;
    env.services()
        .refresh_invoice_data(february.id, TENANT_ID)
        .await
        .expect("refresh");
    assert_eq!(lines_of(&reread(&env, &february).await), whole);

    env.manual_finalize_invoice(february.id)
        .await
        .expect("finalized");
    let ((kind, subtotal, _, _), lines) = the_note(&env, &february).await;
    assert_eq!(
        (kind, subtotal),
        (CreditType::DebtCancellation, -(25_862 + 5_121 + 1_293))
    );
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -25_862, 0),
            ("Platform Fee".to_owned(), -5_121, 0),
            ("Seats".to_owned(), -1_293, 0),
        ],
        "50000, 9900 and 2500 × 15/29"
    );
    let february = reread(&env, &february).await;
    assert_eq!(february.amount_due, 72_400 - 32_276);
    pay(&env, &february).await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    for (classification, amount) in [("PAID", 24_138), ("PROMOTIONAL", 2_414)] {
        let grant = rows
            .iter()
            .find(|g| {
                g.source_kind == "CREDIT_PURCHASE"
                    && g.classification == classification
                    && g.invoice_id == Some(february.id)
            })
            .unwrap_or_else(|| panic!("February's {classification} pack credit"));
        assert_eq!(
            (grant.amount, grant.expires_at),
            (amount, Some(midnight(date(2024, 2, 15))))
        );
    }
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Studio raised from 1 seat to 10 on Jan 20, so February's draft bills 100.00 of seats. Studio
/// -> Large on Feb 15 during grace: once issued, the draft gets 10 seats × 15/29 back, as the
/// ledger held them, not the seat it started with.
#[rstest]
#[tokio::test]
async fn a_deferred_seat_credit_reads_the_seats_billed(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "INSERT INTO slot_transaction \
             (id, subscription_id, delta, prev_active_slots, effective_at, transaction_at, unit, status) \
             SELECT gen_random_uuid(), subscription_id, 9, 1, '2024-01-20', '2024-01-20', unit, status \
             FROM slot_transaction WHERE subscription_id = $1 LIMIT 1",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("nine more seats");
    }
    env.process_cycle_transitions().await;
    let february = invoice(&env, sub_id, 1).await;
    assert!(lines_of(&february).contains(&("Seats".to_owned(), 10_000, date(2024, 3, 1))));

    change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;
    env.process_due_events().await;

    let (_, lines) = the_note(&env, &february).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -5_172, 0)
        ],
        "10000 × 15/29 = 5172.41"
    );
}

/// Studio -> Large on Feb 15 while both subscriptions' February drafts wait to be consolidated:
/// the consolidated invoice, issued after the change, gets the note for Studio's lines only, and
/// Studio's credits granted with it are removed.
#[rstest]
#[tokio::test]
async fn a_consolidated_invoice_issued_after_the_change_is_settled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_and_grace(true, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let scale_sub = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let studio_sub = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, scale_sub, 0).await).await;
    pay(&env, &invoice(&env, studio_sub, 0).await).await;
    env.process_cycle_transitions().await;

    change_on(&env, studio_sub, LARGE, date(2024, 2, 15)).await;
    env.process_due_events().await;

    let parent = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .find(|i| i.subscription_id.is_none())
        .expect("the consolidated February invoice");
    let (_, lines) = the_note(&env, &parent).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ],
        "Studio's fees for 15 of 29 days; Scale's lines untouched"
    );
    sweep_at(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 15)) + Duration::hours(1),
    )
    .await;
    let credits = feb_included_credits(&env, &parent).await;
    assert_eq!(
        movements(&env, &credits).await,
        vec![
            "FUND 500000".to_owned(),
            format!("EXPIRY -500000 {PLAN_CHANGE_REMOVAL}")
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// An invoice issued without the settlement (by an earlier release, during a deploy) is caught
/// by the sweep, once.
#[rstest]
#[tokio::test]
async fn the_sweep_settles_an_invoice_issued_without_it(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    let february = invoice(&env, sub_id, 1).await;
    change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;

    let flag = |sql: &'static str| {
        let env = &env;
        async move {
            let mut conn = env.conn().await;
            diesel::sql_query(sql)
                .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
                .execute(&mut conn)
                .await
                .expect("switch event");
        }
    };
    // What an earlier release finalizing the draft knows of the change: nothing.
    flag("UPDATE subscription_event SET details = details - 'outgoing' WHERE subscription_id = $1")
        .await;
    env.process_due_events().await;
    assert_eq!(notes_on(&env, &february).await, 0);
    flag(
        "UPDATE subscription_event SET details = details || '{\"outgoing\": \"settled\"}' \
         WHERE subscription_id = $1 AND details ? 'new_plan_version_id'",
    )
    .await;

    let sweep = || async {
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("sweep")
    };
    assert_eq!(sweep().await, 1);
    let (_, lines) = the_note(&env, &february).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ]
    );
    assert_eq!(sweep().await, 0);
    assert_eq!(notes_on(&env, &february).await, 1);
}

/// The change and the issue of February's draft run at once: whichever takes the customer first,
/// the draft ends up credited exactly once.
#[rstest]
#[tokio::test]
async fn a_change_racing_the_issue_of_the_draft_credits_it_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    let february = invoice(&env, sub_id, 1).await;

    let (changed, issued) = tokio::join!(
        env.services().apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            LARGE,
            vec![],
            date(2024, 2, 15),
        ),
        env.manual_finalize_invoice(february.id),
    );
    changed.expect("change");
    issued.expect("issue");
    let (_, lines) = the_note(&env, &february).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ]
    );
}

/// A tax-included Studio for a French customer (20% VAT): one subscription changes to Large while
/// February's draft waits out its grace, the other after issuing it. Both drafts get the same
/// note, giving back the quoted 29.00 and 10.00 × 15/29 with their tax.
#[rstest]
#[tokio::test]
async fn a_tax_included_period_is_credited_alike_issued_or_draft(#[future] test_env: TestEnv) {
    use diesel::ExpressionMethods;
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    let customer = french_customer(&env).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    {
        use diesel_models::schema::plan_version::dsl;
        let mut conn = env.conn().await;
        diesel::update(dsl::plan_version)
            .filter(dsl::id.eq_any([STUDIO, LARGE]))
            .set(dsl::tax_inclusive.eq(true))
            .execute(&mut conn)
            .await
            .expect("tax-included Studio");
    }
    let deferred = start(&env, customer, STUDIO, date(2024, 1, 1)).await;
    let issued = start(&env, customer, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, deferred, 0).await).await;
    pay(&env, &invoice(&env, issued, 0).await).await;
    env.process_cycle_transitions().await;
    let deferred_february = invoice(&env, deferred, 1).await;
    let issued_february = invoice(&env, issued, 1).await;
    assert!(deferred_february.tax_inclusive);
    env.manual_finalize_invoice(issued_february.id)
        .await
        .expect("issued");

    change_on(&env, issued, LARGE, date(2024, 2, 15)).await;
    change_on(&env, deferred, LARGE, date(2024, 2, 15)).await;
    assert_eq!(notes_on(&env, &deferred_february).await, 0);
    env.process_due_events().await;

    let lines = note_lines(&env, &deferred_february).await;
    assert_eq!(lines, note_lines(&env, &issued_february).await);
    let totals: Vec<i64> = lines.iter().map(|line| line.4).collect();
    assert_eq!(
        totals,
        vec![-1_500, -517],
        "29.00 and 10.00 × 15/29, tax included"
    );
    let ((_, _, tax, total), _) = the_note(&env, &deferred_february).await;
    assert_eq!((tax, total), (-(250 + 86), -2_017));
}

async fn refused_now(env: &TestEnv, sub_id: SubscriptionId, to: PlanVersionId) -> String {
    let refused = env
        .services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            to,
            vec![],
            date(2024, 1, 16),
        )
        .await
        .expect_err("a minimum commitment holds the change to the period's end");
    format!("{refused:?}")
}

/// A minimum commitment, on the subscription or on the target plan, holds a plan change to the
/// period's end: an immediate change is refused, the preview says so, a scheduled change applies.
#[rstest]
#[tokio::test]
async fn a_minimum_commitment_holds_a_plan_change_to_the_period_end(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    plan_minimum(&env, LARGE, 60_000).await;
    let to_minimum = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    assert!(
        refused_now(&env, to_minimum, LARGE)
            .await
            .contains("minimum commitment")
    );
    let preview = env
        .services()
        .preview_plan_change(to_minimum, TENANT_ID, LARGE, vec![], None)
        .await
        .expect("preview");
    assert!(!preview.immediate_allowed);

    let from_minimum = start(&env, CUST_UBER_ID, LARGE, date(2024, 1, 1)).await;
    assert!(
        refused_now(&env, from_minimum, SCALE)
            .await
            .contains("minimum commitment")
    );
    assert!(
        env.services()
            .preview_plan_change(
                from_minimum,
                TENANT_ID,
                SCALE,
                vec![],
                Some(PlanChangeMode::Immediate)
            )
            .await
            .is_err()
    );

    env.services()
        .schedule_plan_change(Actor::System, from_minimum, TENANT_ID, SCALE, vec![])
        .await
        .expect("scheduled at the period's end");
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(from_minimum).await.plan_version_id,
        SCALE
    );
    assert_eq!(
        notes_on(&env, &invoice(&env, from_minimum, 0).await).await,
        0
    );
}

/// A note an operator wrote with the reason a plan change uses is not one: the change still
/// credits the unused time of the line it touched; that note only caps what can be taken back
/// (R-082).
#[rstest]
#[tokio::test]
async fn a_hand_written_note_is_not_a_plan_change_note(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditLineItem};

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    let seats = january
        .line_items
        .iter()
        .find(|line| line.name == "Seats")
        .expect("seats")
        .local_id
        .clone();
    env.store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: january.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: seats,
                    amount_cents: 100,
                }],
                reason: Some("Plan change".to_owned()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .expect("hand-written note");

    change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;

    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, january.id)
        .await
        .expect("notes");
    let seat_credits: Vec<i64> = notes
        .iter()
        .flat_map(|note| note.line_items.iter())
        .filter(|line| line.name == "Seats")
        .map(|line| line.amount_subtotal)
        .collect();
    assert_eq!(seat_credits.len(), 2);
    assert!(
        seat_credits.contains(&-516),
        "1,000 × 16/31 = 516.1, {seat_credits:?}"
    );
}

/// A French customer (20% VAT) with a 20% coupon: January bills 29.00 and 10.00 less 5.80 and
/// 2.00 off, taxed 4.64 and 1.60. Studio -> Scale on Jan 16 gives back 16 of 31 days of each line
/// at its own discount and tax.
#[rstest]
#[tokio::test]
async fn a_change_credits_a_discounted_taxed_invoice_as_billed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let customer = french_customer(&env).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let coupon = env.create_percentage_coupon("PLANCHANGE20", 20).await;
    let sub_id = subscription()
        .customer(customer)
        .plan_version(STUDIO)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .coupon(coupon)
        .create(env.services())
        .await;
    let january = invoice(&env, sub_id, 0).await;
    assert_eq!(
        (
            january.subtotal,
            january.discount,
            january.tax_amount,
            january.total
        ),
        (3_900, 780, 624, 3_744)
    );
    pay(&env, &january).await;

    change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;

    assert_eq!(
        note_lines(&env, &january).await,
        vec![
            ("Platform Fee".to_owned(), -1_497, -1_198, -240, -1_438),
            ("Seats".to_owned(), -516, -413, -83, -496),
        ],
        "subtotal × 16/31; discounted 2320 and 800 and taxed 464 and 160 in proportion"
    );
}

/// A quarterly pack billed on January's draft (no auto-advance) is found when the draft is issued
/// after a change on Feb 16: 45 of its 91 days come back, by time since it is not paid.
#[tokio::test]
async fn a_longer_cadence_pack_on_an_older_draft_is_settled_when_issued() {
    let env = test_env_with_seed_and_usage(
        SeedLevel::PLANS,
        Arc::new(meteroid_store::clients::usage::MockUsageClient::noop()),
    )
    .await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    crate::helpers::plans::as_draft(
        env.store(),
        STUDIO,
        env.store().create_price_component_from_ref(
            "Quarterly credit".to_owned(),
            new_pack_product("Quarterly credit"),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Quarterly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(900),
                    quantity: 1,
                    credit_amount: dec!(900),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            })],
            STUDIO,
            TENANT_ID,
        ),
    )
    .await
    .expect("quarterly pack");
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(STUDIO)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let january = invoice(&env, sub_id, 0).await;
    env.process_cycles().await;

    change_on(&env, sub_id, SCALE, date(2024, 2, 16)).await;
    env.manual_finalize_invoice(january.id)
        .await
        .expect("January issued");

    let ((kind, _, _, _), lines) = the_note(&env, &january).await;
    assert_eq!(kind, CreditType::DebtCancellation);
    assert_eq!(
        lines,
        vec![("Quarterly credit".to_owned(), -44_505, 0)],
        "90000 × 45/91"
    );
}

/// A portal upgrade quoted on day 10 whose payment settles once the next period started: the
/// change applies today, priced today, and the payment is recorded for what it is: the
/// adjustment is left partially paid, its remainder collected like any invoice.
#[rstest]
#[tokio::test]
async fn a_checkout_settling_after_a_rollover_pays_only_what_it_paid(#[future] test_env: TestEnv) {
    use meteroid_store::domain::InvoicePaymentStatus;
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let today = Utc::now().date_naive();
    let begin = today - Duration::days(35);
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, begin).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let quoted_on = begin + Duration::days(10);
    let quote = env
        .services()
        .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, SCALE, quoted_on)
        .await
        .expect("quote");
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            quoted_on,
        )
        .await
        .expect("session");
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
        .expect("checkout")
    else {
        panic!("the payment is pending")
    };
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.process_cycles().await;

    env.deliver_mock_webhook(succeeded_payload(pending.id, quote.amount_due).as_bytes())
        .await;

    let subscription = env.get_subscription(sub_id).await;
    assert_eq!(subscription.plan_version_id, SCALE);
    let (period_start, period_end) = (
        subscription.current_period_start,
        subscription.current_period_end.expect("period end"),
    );
    assert!(period_start > quoted_on && period_start <= today);
    let adjustment = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.invoice_type == InvoiceType::Adjustment)
        .expect("the adjustment");
    let share = |amount: i64| {
        let cents = (Decimal::from(amount) * Decimal::from((period_end - today).num_days())
            / Decimal::from((period_end - period_start).num_days()))
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        i64::try_from(cents).expect("cents")
    };
    assert_eq!(
        adjustment.total,
        share(9_900) + share(2_500) + share(50_000)
    );
    assert!(adjustment.total - adjustment.applied_credits > quote.amount_due);
    assert_eq!(
        adjustment.amount_due,
        adjustment.total - adjustment.applied_credits - quote.amount_due,
        "the payment counts for what it paid"
    );
    assert_eq!(adjustment.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        adjustment.payment_status,
        InvoicePaymentStatus::PartiallyPaid
    );
}

fn charged_back_payload(external_id: &str) -> String {
    format!(
        r#"{{"id":"evt_cb_{external_id}","kind":"payment_charged_back","external_id":"{external_id}"}}"#
    )
}

/// The debit failed after it settled (GoCardless `late_failure_settled`): the invoice reopens.
fn late_failure_payload(external_id: &str) -> String {
    format!(
        r#"{{"id":"evt_lf_{external_id}","kind":"payment_late_failure","external_id":"{external_id}"}}"#
    )
}

fn reinstated_payload(external_id: &str) -> String {
    format!(
        r#"{{"id":"evt_rs_{external_id}","kind":"payment_reinstated","external_id":"{external_id}"}}"#
    )
}

async fn collected_credit(
    env: &TestEnv,
    payment: common_domain::ids::PaymentTransactionId,
) -> CustomerCreditGrantRow {
    grant_rows(env, CUST_UBER_ID)
        .await
        .into_iter()
        .find(|g| g.source_kind == "OVERPAYMENT" && g.source_id == payment.as_uuid())
        .expect("the credit kept from the payment")
}

/// The credit kept from what an invoice's payments collected above it: one grant per invoice.
async fn overpayment_credit(env: &TestEnv, invoice: &Invoice) -> CustomerCreditGrantRow {
    let mut kept: Vec<_> = grant_rows(env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "OVERPAYMENT" && g.invoice_id == Some(invoice.id))
        .collect();
    assert_eq!(kept.len(), 1, "one grant keeps the invoice's overpayment");
    kept.remove(0)
}

async fn refused_change(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> String {
    let refused = env
        .services()
        .apply_plan_change_immediate_at(Actor::System, sub_id, TENANT_ID, SCALE, vec![], on)
        .await
        .expect_err("another change waits for the payment");
    format!("{refused:?}")
}

/// A plan-change payment frozen at 341.93 settles on its adjustment repriced to 301.93 (an
/// accepted payment does, once its change applied): the 40.00 above is kept as credit. A
/// chargeback takes that credit back; the invoice stays paid, the rest is the merchant's loss.
#[rstest]
#[tokio::test]
async fn a_payment_above_its_repriced_adjustment_keeps_the_excess_until_reversed(
    #[future] test_env: TestEnv,
) {
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;
    use meteroid_store::domain::InvoicePaymentStatus;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let change_date = date(2024, 1, 16);
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            change_date,
        )
        .await
        .expect("session");
    change_on(&env, sub_id, SCALE, change_date).await;
    let adjustment = invoice(&env, sub_id, 1).await;
    assert_eq!(adjustment.amount_due, 30_193);

    let payment = common_domain::ids::PaymentTransactionId::new();
    let external_id = format!("pi_{}", payment.as_base62());
    {
        let mut conn = env.conn().await;
        PaymentTransactionRowNew {
            id: payment,
            tenant_id: TENANT_ID,
            invoice_id: Some(adjustment.id),
            provider_transaction_id: Some(external_id.clone()),
            amount: 34_193,
            currency: "EUR".to_owned(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: Some(session.id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: Some(CUST_UBER_ID),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("accepted payment");
    }

    env.deliver_mock_webhook(succeeded_payload(payment, 34_193).as_bytes())
        .await;
    let paid = reread(&env, &adjustment).await;
    assert_eq!(
        (paid.amount_due, paid.payment_status),
        (0, InvoicePaymentStatus::Paid)
    );
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(
        (kept.amount, kept.remaining, kept.reference),
        (4_000, 4_000, Some(payment.to_string()))
    );

    env.deliver_mock_webhook(charged_back_payload(&external_id).as_bytes())
        .await;
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(kept.remaining, 0);
    assert_eq!(
        movements(&env, &kept).await,
        vec!["FUND 4000".to_owned(), "REVERSAL -4000".to_owned()]
    );
    let disputed = reread(&env, &adjustment).await;
    assert_eq!(
        (disputed.amount_due, disputed.payment_status),
        (0, meteroid_store::domain::enums::InvoicePaymentStatus::Paid)
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A portal upgrade cancelled while its payment is pending lets another change apply; the old
/// payment settling later never applies its stale change, it is kept as credit. Spent, then
/// charged back: what a void gives back to it is taken back at once; the dispute won restores it.
#[rstest]
#[tokio::test]
async fn a_cancelled_plan_change_paid_late_keeps_the_money_as_credit(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: pending,
        ..
    } = checkout(&env, session.id).await
    else {
        panic!("the payment is pending")
    };
    let refused = refused_change(&env, sub_id, date(2024, 1, 20)).await;
    assert!(
        refused.contains("cancel that checkout session"),
        "{refused}"
    );

    env.store()
        .cancel_checkout_session(TENANT_ID, session.id)
        .await
        .expect("cancelled");
    change_on(&env, sub_id, LARGE, date(2024, 1, 20)).await;
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.deliver_mock_webhook(succeeded_payload(pending.id, 30_193).as_bytes())
        .await;

    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, LARGE);
    let kept = collected_credit(&env, pending.id).await;
    assert_eq!(
        (kept.amount, kept.remaining, kept.invoice_id),
        (30_193, 30_193, None)
    );

    let external_id = {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRow::get_by_id(&mut conn, pending.id, TENANT_ID)
            .await
            .expect("payment")
            .provider_transaction_id
            .expect("provider id")
    };
    // February's Large fees draw it all.
    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;
    env.process_cycles().await;
    let february = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| {
            i.billing_period_start == date(2024, 2, 1)
                && i.invoice_type == meteroid_store::domain::enums::InvoiceType::Recurring
        })
        .expect("February");
    assert_eq!(collected_credit(&env, pending.id).await.remaining, 0);

    env.deliver_mock_webhook(charged_back_payload(&external_id).as_bytes())
        .await;
    assert_eq!(
        collected_credit(&env, pending.id).await.remaining,
        0,
        "spent: nothing left to take back"
    );
    assert_eq!(
        payment_keyed_notices(&env, pending.id).await,
        vec![("PAYMENT_DISPUTED".to_owned(), 30_193, 30_193)],
        "the spent credit no invoice took is reported, keyed by its payment"
    );
    env.try_void_invoice(february.id).await.expect("void");
    assert_eq!(
        collected_credit(&env, pending.id).await.remaining,
        0,
        "given back to a reversed payment's credit, taken back at once"
    );

    env.deliver_mock_webhook(reinstated_payload(&external_id).as_bytes())
        .await;
    assert_eq!(
        collected_credit(&env, pending.id).await.remaining,
        30_193,
        "the dispute won restores it"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A portal upgrade cancelled while its 301.93 payment is pending, another change applied, then
/// the payment settles: kept whole as credit. Returns the subscription, the payment and its
/// provider id.
async fn kept_plan_change_payment(
    env: &TestEnv,
) -> (
    SubscriptionId,
    common_domain::ids::PaymentTransactionId,
    String,
) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    env.seed_payments().await;
    halyard(env, CreditExpiry::BillingPeriod).await;
    large(env).await;
    let sub_id = start(env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(env, &invoice(env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: pending,
        ..
    } = checkout(env, session.id).await
    else {
        panic!("the payment is pending")
    };
    env.store()
        .cancel_checkout_session(TENANT_ID, session.id)
        .await
        .expect("cancelled");
    change_on(env, sub_id, LARGE, date(2024, 1, 20)).await;
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.deliver_mock_webhook(succeeded_payload(pending.id, 30_193).as_bytes())
        .await;
    let kept = collected_credit(env, pending.id).await;
    assert_eq!(
        (kept.amount, kept.remaining, kept.invoice_id),
        (30_193, 30_193, None)
    );
    let external_id = {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRow::get_by_id(&mut conn, pending.id, TENANT_ID)
            .await
            .expect("payment")
            .provider_transaction_id
            .expect("provider id")
    };
    (sub_id, pending.id, external_id)
}

/// The provider's `payment_refunded` envelope: refunds reaching `cumulative` in all, the last one
/// `refund` of `amount` in `state` ("succeeded", "pending" or "failed").
fn refunded_payload(
    external_id: &str,
    cumulative: i64,
    refund: &str,
    amount: i64,
    state: &str,
) -> String {
    let flags = match state {
        "succeeded" => r#""succeeded":true"#,
        "pending" => r#""pending":true"#,
        _ => r#""succeeded":false"#,
    };
    format!(
        r#"{{"id":"evt_re_{refund}_{state}","kind":"payment_refunded","external_id":"{external_id}","amount":{cumulative},"currency":"EUR","refunds":[{{"id":"{refund}","amount":{amount},{flags}}}]}}"#
    )
}

/// The kept payment refunded in full at the provider: the credit it funded goes back with the
/// money, once, however often the webhook is delivered; nothing was spent, nothing is reported.
#[rstest]
#[tokio::test]
async fn a_kept_plan_change_payment_refunded_at_the_provider_takes_its_credit_back(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, payment, external_id) = kept_plan_change_payment(&env).await;

    let refunded = refunded_payload(&external_id, 30_193, "re_full", 30_193, "succeeded");
    env.deliver_mock_webhook(refunded.as_bytes()).await;
    env.deliver_mock_webhook(refunded.as_bytes()).await;
    let kept = collected_credit(&env, payment).await;
    assert_eq!(kept.remaining, 0);
    assert_eq!(
        movements(&env, &kept).await,
        vec!["FUND 30193".to_owned(), "REVERSAL -30193".to_owned()]
    );
    assert!(payment_keyed_notices(&env, payment).await.is_empty());
    assert_eq!(
        env.store()
            .reconcile_reversed_credit_purchases(100)
            .await
            .expect("sweep"),
        0,
        "nothing left out of line"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// 100.00 then the other 201.93 of the kept payment refunded at the provider: each refund takes
/// back its own share of the credit, never the same money twice.
#[rstest]
#[tokio::test]
async fn partial_provider_refunds_of_a_kept_payment_take_back_their_share(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, payment, external_id) = kept_plan_change_payment(&env).await;

    let first = refunded_payload(&external_id, 10_000, "re_first", 10_000, "succeeded");
    env.deliver_mock_webhook(first.as_bytes()).await;
    assert_eq!(collected_credit(&env, payment).await.remaining, 20_193);

    let second = refunded_payload(&external_id, 30_193, "re_second", 20_193, "succeeded");
    env.deliver_mock_webhook(second.as_bytes()).await;
    env.deliver_mock_webhook(second.as_bytes()).await;
    let kept = collected_credit(&env, payment).await;
    assert_eq!(kept.remaining, 0);
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 30193".to_owned(),
            "REVERSAL -10000".to_owned(),
            "REVERSAL -20193".to_owned()
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The kept credit spent on February's fees, then the payment refunded at the provider: nothing
/// is left to take back, and the 301.93 spent is reported unrecovered, as refunded, once.
#[rstest]
#[tokio::test]
async fn a_kept_payment_spent_then_refunded_reports_the_spent_credit_as_refunded(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::connectors::MockChargeBehavior;

    let env = test_env.await;
    let (_, payment, external_id) = kept_plan_change_payment(&env).await;
    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;
    env.process_cycles().await;
    assert_eq!(collected_credit(&env, payment).await.remaining, 0);

    let refunded = refunded_payload(&external_id, 30_193, "re_spent", 30_193, "succeeded");
    env.deliver_mock_webhook(refunded.as_bytes()).await;
    env.deliver_mock_webhook(refunded.as_bytes()).await;
    assert_eq!(collected_credit(&env, payment).await.remaining, 0);
    assert_eq!(
        payment_keyed_notices(&env, payment).await,
        vec![("PAYMENT_REFUNDED".to_owned(), 30_193, 30_193)]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// 30.00 of the 301.93 kept payment spent on a one-off invoice, then all of it refunded at the
/// provider: the 271.93 left is taken back, nothing remains, and the 30.00 spent is reported
/// unrecovered, as refunded.
#[rstest]
#[tokio::test]
async fn a_kept_payment_partly_spent_then_refunded_takes_back_the_rest(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, payment, external_id) = kept_plan_change_payment(&env).await;
    spend_on_one_off(&env, 3_000).await;
    assert_eq!(collected_credit(&env, payment).await.remaining, 27_193);

    let refunded = refunded_payload(&external_id, 30_193, "re_rest", 30_193, "succeeded");
    env.deliver_mock_webhook(refunded.as_bytes()).await;
    let kept = collected_credit(&env, payment).await;
    assert_eq!(kept.remaining, 0);
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 30193".to_owned(),
            "DRAW -3000".to_owned(),
            "REVERSAL -27193".to_owned()
        ]
    );
    assert_eq!(
        payment_keyed_notices(&env, payment).await,
        vec![("PAYMENT_REFUNDED".to_owned(), 3_000, 3_000)]
    );
    assert_eq!(
        env.store()
            .reconcile_reversed_credit_purchases(100)
            .await
            .expect("sweep"),
        0,
        "the loss reported is the loss the money leaves"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A refund of the kept payment the provider reports in flight takes the credit back at once;
/// when it fails, the money stayed and the credit comes back.
#[rstest]
#[tokio::test]
async fn a_failed_provider_refund_of_a_kept_payment_gives_the_credit_back(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (_, payment, external_id) = kept_plan_change_payment(&env).await;

    let pending = refunded_payload(&external_id, 30_193, "re_failing", 30_193, "pending");
    env.deliver_mock_webhook(pending.as_bytes()).await;
    assert_eq!(collected_credit(&env, payment).await.remaining, 0);

    let failed = refunded_payload(&external_id, 0, "re_failing", 30_193, "failed");
    env.deliver_mock_webhook(failed.as_bytes()).await;
    let kept = collected_credit(&env, payment).await;
    assert_eq!(kept.remaining, 30_193);
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 30193".to_owned(),
            "REVERSAL -30193".to_owned(),
            "REINSTATEMENT 30193".to_owned()
        ]
    );
    assert!(payment_keyed_notices(&env, payment).await.is_empty());
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A refund an instance of the previous release recorded on the kept payment, without following
/// the credit: the reversal sweep takes the credit back.
#[rstest]
#[tokio::test]
async fn the_sweep_follows_a_refund_of_a_kept_payment_the_previous_release_recorded(
    #[future] test_env: TestEnv,
) {
    use diesel::sql_types::{BigInt, Uuid as SqlUuid};
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    let (_, payment, _) = kept_plan_change_payment(&env).await;
    diesel::sql_query(
        "INSERT INTO payment_transaction
         SELECT (jsonb_populate_record(p, jsonb_build_object(
             'id', $1::uuid,
             'payment_type', 'REFUND',
             'parent_transaction_id', p.id,
             'provider_transaction_id', 're_old_pod',
             'amount', $2::bigint,
             'status', 'SETTLED',
             'checkout_session_id', NULL,
             'pending_plan_version_id', NULL,
             'amount_refunded', 0,
             'amount_reversed', 0,
             'amount_disputed', 0,
             'amount_reversed_excess', 0,
             'refund_excess', 0,
             'refund_mode', 'ONLINE',
             'credit_note_id', NULL))).*
         FROM payment_transaction p
         WHERE p.id = $3",
    )
    .bind::<SqlUuid, _>(Uuid::now_v7())
    .bind::<BigInt, _>(12_000_i64)
    .bind::<SqlUuid, _>(payment.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("a refund recorded by the previous release");

    assert_eq!(
        env.store()
            .reconcile_reversed_credit_purchases(100)
            .await
            .expect("sweep"),
        1
    );
    assert_eq!(collected_credit(&env, payment).await.remaining, 18_193);
    assert_eq!(
        env.store()
            .reconcile_reversed_credit_purchases(100)
            .await
            .expect("sweep"),
        0,
        "in line once followed"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A plan-change checkout whose payment waits on the customer's authentication past a day
/// expires, and no longer holds other changes back.
#[rstest]
#[tokio::test]
async fn a_plan_change_checkout_left_unauthenticated_expires(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;
    checkout(&env, session.id).await;
    assert!(
        refused_change(&env, sub_id, date(2024, 1, 16))
            .await
            .contains("checkout session")
    );

    env.store().expire_sessions().await.expect("expiry");
    assert!(
        refused_change(&env, sub_id, date(2024, 1, 16))
            .await
            .contains("checkout session"),
        "still within the authentication window"
    );
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "UPDATE payment_transaction SET created_at = now() - interval '2 days' \
             WHERE checkout_session_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(session.id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("aged payment");
    }
    env.store().expire_sessions().await.expect("expiry");
    change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, SCALE);
}

async fn aged(env: &TestEnv, session: common_domain::ids::CheckoutSessionId) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE payment_transaction SET created_at = now() - interval '2 days' \
         WHERE checkout_session_id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(session.as_uuid())
    .execute(&mut conn)
    .await
    .expect("aged payment");
}

/// A debit processing for days (SEPA) is never abandoned: its checkout keeps waiting for the
/// provider and holds other changes back. One that needed 3DS and then went processing waits the
/// same way, and its change applies when it settles.
#[rstest]
#[tokio::test]
async fn a_plan_change_payment_processing_is_never_abandoned(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sepa_sub = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    let card_sub = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sepa_sub, 0).await).await;
    pay(&env, &invoice(&env, card_sub, 0).await).await;
    let session_of = |sub_id| {
        let env = &env;
        async move {
            env.services()
                .create_plan_change_checkout_session(
                    TENANT_ID,
                    sub_id,
                    SCALE,
                    CUST_UBER_ID,
                    None,
                    date(2024, 1, 16),
                )
                .await
                .expect("session")
        }
    };
    let sepa = session_of(sepa_sub).await;
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    checkout(&env, sepa.id).await;
    let card = session_of(card_sub).await;
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: card_payment,
        ..
    } = checkout(&env, card.id).await
    else {
        panic!("the card payment awaits authentication")
    };
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_pp_{tx}","kind":"payment_pending","transaction_id":"{tx}","external_id":"pi_{tx}"}}"#,
            tx = card_payment.id.as_base62()
        )
        .as_bytes(),
    )
    .await;
    aged(&env, sepa.id).await;
    aged(&env, card.id).await;
    env.store().expire_sessions().await.expect("expiry");

    assert!(
        refused_change(&env, sepa_sub, date(2024, 1, 16))
            .await
            .contains("checkout session"),
        "the debit still holds the change back"
    );
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.deliver_mock_webhook(succeeded_payload(card_payment.id, 30_193).as_bytes())
        .await;
    assert_eq!(env.get_subscription(card_sub).await.plan_version_id, SCALE);
}

async fn pending_payment(
    env: &TestEnv,
    invoice_id: common_domain::ids::InvoiceId,
    session: Option<common_domain::ids::CheckoutSessionId>,
    amount: i64,
) -> common_domain::ids::PaymentTransactionId {
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;

    let payment = common_domain::ids::PaymentTransactionId::new();
    let mut conn = env.conn().await;
    PaymentTransactionRowNew {
        id: payment,
        tenant_id: TENANT_ID,
        invoice_id: Some(invoice_id),
        provider_transaction_id: Some(format!("pi_{}", payment.as_base62())),
        amount,
        currency: "EUR".to_owned(),
        payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
        status: PaymentStatusEnum::Pending,
        payment_type: PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: session,
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: Some(CUST_UBER_ID),
        pending_provider_intent_id: None,
        pending_connection_id: None,
        parent_transaction_id: None,
        credit_note_id: None,
        refund_mode: None,
        reversal_reason: None,
    }
    .insert(&mut conn)
    .await
    .expect("payment");
    payment
}

/// A plan-change payment of 250.00 against its adjustment repriced to 301.93 leaves 51.93 due;
/// an ordinary 60.00 payment then overpays it by 8.07, kept as credit once, citing it.
#[rstest]
#[tokio::test]
async fn a_later_payment_overpaying_a_plan_change_invoice_is_kept_once(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;
    let adjustment = invoice(&env, sub_id, 1).await;
    assert_eq!(adjustment.amount_due, 30_193);

    let checkout = pending_payment(&env, adjustment.id, Some(session.id), 25_000).await;
    env.deliver_mock_webhook(succeeded_payload(checkout, 25_000).as_bytes())
        .await;
    assert_eq!(reread(&env, &adjustment).await.amount_due, 5_193);
    let ordinary = pending_payment(&env, adjustment.id, None, 6_000).await;
    env.deliver_mock_webhook(succeeded_payload(ordinary, 6_000).as_bytes())
        .await;

    assert_eq!(reread(&env, &adjustment).await.amount_due, 0);
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(
        (kept.amount, kept.reference),
        (807, Some(ordinary.to_string()))
    );
    let manual: Vec<_> = grant_rows(&env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "OVERPAYMENT")
        .collect();
    assert_eq!(
        manual.len(),
        1,
        "nothing kept for the checkout's own payment"
    );
}

/// Two plan changes whose drafts were issued without their settlement: the sweep, a switch per
/// page, visits both and completes them; a second sweep finds nothing.
#[rstest]
#[tokio::test]
async fn the_sweep_visits_every_switch_page_by_page(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let mut subs = Vec::new();
    for _ in 0..2 {
        let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
        pay(&env, &invoice(&env, sub_id, 0).await).await;
        subs.push(sub_id);
    }
    env.process_cycle_transitions().await;
    let mut februaries = Vec::new();
    for sub_id in subs {
        februaries.push((sub_id, invoice(&env, sub_id, 1).await));
        change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;
    }
    let sql = |sql: &'static str| {
        let env = &env;
        async move {
            let mut conn = env.conn().await;
            diesel::sql_query(sql)
                .execute(&mut conn)
                .await
                .expect("switch events");
        }
    };
    sql("UPDATE subscription_event SET details = details - 'outgoing' WHERE event_type = 'SWITCH'")
        .await;
    env.process_due_events().await;
    sql(
        "UPDATE subscription_event SET details = details || '{\"outgoing\": \"settled\"}' \
         WHERE event_type = 'SWITCH' AND details ? 'new_plan_version_id'",
    )
    .await;

    let env = &env;
    let sweep = |page: i64| async move {
        env.services()
            .settle_missed_plan_change_endings(page)
            .await
            .expect("sweep")
    };
    assert_eq!(sweep(1).await, 2);
    for (_, february) in &februaries {
        assert_eq!(notes_on(env, february).await, 1);
    }
    assert_eq!(sweep(1).await, 0);
}

/// An expired plan-change checkout whose payment has no invoice keeps its row: the cleanup
/// skips it, never orphaning the payment.
#[rstest]
#[tokio::test]
async fn the_cleanup_keeps_a_checkout_anchoring_a_payment(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;
    checkout(&env, session.id).await;
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "UPDATE payment_transaction SET created_at = now() - interval '9 days' \
             WHERE checkout_session_id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(session.id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("aged payment");
        diesel::sql_query(
            "UPDATE checkout_session SET created_at = now() - interval '9 days' WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(session.id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("aged session");
    }
    env.store().expire_sessions().await.expect("expiry");
    env.store().cleanup_old_sessions(3).await.expect("cleanup");
    env.store()
        .get_checkout_session(TENANT_ID, session.id)
        .await
        .expect("kept");
}

/// A legacy SEPA payment settling while the period's invoice is still a draft: the change
/// applies as the earlier release netted it; the draft is credited neither now nor once issued.
#[rstest]
#[tokio::test]
async fn a_legacy_payment_settling_over_a_draft_period_leaves_the_draft_whole(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::InvoicePaymentStatus;
    use meteroid_store::domain::enums::InvoiceStatusEnum;

    let env = test_env.await;
    env.seed_payments().await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(STUDIO)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let starter = invoice(&env, sub_id, 0).await;
    assert_eq!(starter.status, InvoiceStatusEnum::Draft);
    let (legacy, pending) = legacy_pending_adjustment(&env, sub_id).await;

    env.deliver_mock_webhook(succeeded_payload(pending, 8_500).as_bytes())
        .await;

    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_PRO_ID
    );
    let after = reread(&env, &starter).await;
    assert_eq!(after.status, InvoiceStatusEnum::Draft);
    assert_eq!(
        lines_of(&after),
        lines_of(&starter),
        "netted already, not cut"
    );
    assert_eq!(after.total, 3_900);
    assert_eq!(notes_on(&env, &starter).await, 0);
    assert_eq!(
        reread(&env, &legacy).await.payment_status,
        InvoicePaymentStatus::Paid
    );
    env.manual_finalize_invoice(starter.id)
        .await
        .expect("issued");
    assert_eq!(
        notes_on(&env, &starter).await,
        0,
        "nor credited once issued: the earlier release netted it"
    );
}

/// A pack of 70.07 starting Feb 27, 2023 on a period anchored on the 1st covers 2 of 28 days:
/// 7007 × 2 / 28 = 500.5, billed and granted 501, the exact half rounded away from zero.
#[rstest]
#[tokio::test]
async fn a_mid_period_start_prorates_by_exact_days(#[future] test_env: TestEnv) {
    let env = test_env.await;
    add_component(
        &env,
        "Starter credit",
        LARGE,
        new_pack_product("Starter credit"),
        Pricing::Credit {
            unit_price: dec!(70.07),
            quantity: 1,
            credit_amount: dec!(70.07),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: None,
            rollover: None,
            once: false,
        },
    )
    .await;
    let mut request = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(LARGE)
        .start_date(date(2023, 2, 27))
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
    assert_eq!(
        pack_lines(&invoice(&env, sub_id, 0).await),
        vec![pack("Starter credit", 501, 501)]
    );
}

/// Round 5, item 1. Scale (a 300.00 minimum) without auto-advance: February's draft bills
/// January's 100.00 of bandwidth and 124.00 of fees, 76.00 short. A change to Studio (a 500.00
/// minimum) scheduled at February's end applies; the draft, issued after, keeps Scale's minimum.
#[tokio::test]
async fn a_draft_keeps_the_minimum_of_its_period_after_a_scheduled_change() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    plan_minimum(&env, SCALE, 30_000).await;
    plan_minimum(&env, STUDIO, 50_000).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(SCALE)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), 1_000);
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    let shortfall = |invoice: &Invoice| {
        invoice
            .line_items
            .iter()
            .find(|line| line.kind == LineItemKind::MinimumShortfall)
            .map(|line| line.amount_subtotal)
    };
    assert_eq!(shortfall(&february), Some(7_600));

    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, STUDIO, vec![])
        .await
        .expect("scheduled");
    env.process_cycles().await;
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, STUDIO);

    env.manual_finalize_invoice(february.id)
        .await
        .expect("issued");
    assert_eq!(
        shortfall(&reread(&env, &february).await),
        Some(7_600),
        "Scale's 300.00 less the 224.00 billed, not Studio's 500.00"
    );
}

/// The credit kept from overpayments of an invoice a plan-change checkout paid toward, as left.
async fn kept_for(env: &TestEnv, invoice: &Invoice) -> i64 {
    grant_rows(env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "OVERPAYMENT" && g.invoice_id == Some(invoice.id))
        .map(|g| g.remaining)
        .sum()
}

async fn plan_change_adjustment(env: &TestEnv) -> (Invoice, common_domain::ids::CheckoutSessionId) {
    halyard(env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(env, &invoice(env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    change_on(env, sub_id, SCALE, date(2024, 1, 16)).await;
    let adjustment = invoice(env, sub_id, 1).await;
    assert_eq!(adjustment.amount_due, 30_193);
    (adjustment, session.id)
}

/// Round 5, item 4. A 250.00 plan-change payment and a 60.00 one overpay the 301.93 adjustment by
/// 8.07, kept as credit; the 60.00 is charged back (8.07 taken back), then replaced by another
/// 60.00: the 8.07 is kept again.
#[rstest]
#[tokio::test]
async fn an_overpayment_replaced_after_a_reversal_is_kept_again(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let (adjustment, session) = plan_change_adjustment(&env).await;
    let checkout = pending_payment(&env, adjustment.id, Some(session), 25_000).await;
    env.deliver_mock_webhook(succeeded_payload(checkout, 25_000).as_bytes())
        .await;
    let first = pending_payment(&env, adjustment.id, None, 6_000).await;
    env.deliver_mock_webhook(succeeded_payload(first, 6_000).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 807);

    env.deliver_mock_webhook(late_failure_payload(&format!("pi_{}", first.as_base62())).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 0);
    let replacement = pending_payment(&env, adjustment.id, None, 6_000).await;
    env.deliver_mock_webhook(succeeded_payload(replacement, 6_000).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 807);
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Round 5, item 4. The 301.93 plan-change payment fails late, a 301.93 payment replaces it,
/// then the first is reinstated: 301.93 was collected twice, the excess kept as credit.
#[rstest]
#[tokio::test]
async fn a_payment_reinstated_after_its_replacement_is_kept_as_credit(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let (adjustment, session) = plan_change_adjustment(&env).await;
    let original = pending_payment(&env, adjustment.id, Some(session), 30_193).await;
    let external_id = format!("pi_{}", original.as_base62());
    env.deliver_mock_webhook(succeeded_payload(original, 30_193).as_bytes())
        .await;
    env.deliver_mock_webhook(late_failure_payload(&external_id).as_bytes())
        .await;
    assert_eq!(reread(&env, &adjustment).await.amount_due, 30_193);
    let replacement = pending_payment(&env, adjustment.id, None, 30_193).await;
    env.deliver_mock_webhook(succeeded_payload(replacement, 30_193).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 0);

    env.deliver_mock_webhook(reinstated_payload(&external_id).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 30_193);
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Round 5, item 5. Studio raised from 1 seat to 10 this morning (the 9 billed for the rest of the
/// period), then changed to Large today while this period's invoice waits out its grace: once
/// issued, its one seat comes back for the days left, as the change read 10 seats. On the period's
/// first day the included credit is never funded: the change removed it before it took effect.
#[rstest]
#[case::first_day(date(2024, 2, 1))]
#[case::mid_period(date(2024, 2, 15))]
#[tokio::test]
async fn seats_bought_the_day_of_a_change_count_once_the_draft_is_issued(
    #[future] test_env: TestEnv,
    #[case] today: NaiveDate,
) {
    use meteroid_store::domain::SlotUpgradeBillingMode;

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let (period_start, period_end) = (date(2024, 2, 1), date(2024, 3, 1));
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    let draft = invoice(&env, sub_id, 1).await;
    assert_eq!(draft.billing_period_start, period_start);
    let seats = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Seats")
        .and_then(|c| c.price_component_id)
        .expect("Studio's seats");
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            seats,
            9,
            SlotUpgradeBillingMode::Optimistic,
            Some(today.and_hms_opt(9, 0, 0).expect("this morning")),
        )
        .await
        .expect("nine more seats");

    change_on(&env, sub_id, LARGE, today).await;
    env.manual_finalize_invoice(draft.id).await.expect("issued");

    let left = |amount: i64| {
        let share = Decimal::from(amount) * Decimal::from((period_end - today).num_days())
            / Decimal::from((period_end - period_start).num_days());
        i64::try_from(
            share.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero),
        )
        .expect("cents")
    };
    let (_, lines) = the_note(&env, &draft).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -left(2_900), 0),
            ("Seats".to_owned(), -left(1_000), 0)
        ]
    );
    let included: Vec<_> = grant_rows(&env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.invoice_id == Some(draft.id) && g.source_kind == "INCLUDED_CREDIT")
        .map(|g| g.expires_at)
        .collect();
    let expected = if today == period_start {
        vec![]
    } else {
        vec![Some(midnight(today))]
    };
    assert_eq!(included, expected);
}

/// Round 5, item 8. A card payment that asked for 3DS goes processing; the webhook saying so is
/// lost and polling finds it processing: its checkout no longer waits on the customer, so the
/// day-long authentication window does not expire it.
#[rstest]
#[tokio::test]
async fn a_payment_found_processing_by_polling_is_not_abandoned(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::connectors::{MockChargeBehavior, MockReconcileStatus};
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            SCALE,
            CUST_UBER_ID,
            None,
            date(2024, 1, 16),
        )
        .await
        .expect("session");
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;
    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } =
        checkout(&env, session.id).await
    else {
        panic!("the payment awaits authentication")
    };
    env.set_mock_reconcile_status(MockReconcileStatus::Processing)
        .await;
    env.services()
        .reconcile_pending_transaction(transaction.id, TENANT_ID)
        .await
        .expect("polled");
    aged(&env, session.id).await;
    env.store().expire_sessions().await.expect("expiry");
    assert!(
        refused_change(&env, sub_id, date(2024, 1, 16))
            .await
            .contains("checkout session"),
        "the processing payment still holds the change"
    );
}

/// Studio from January with February's draft held in its grace window, changed to Large on
/// February 15: the change has nothing issued to settle yet.
async fn held_draft_switch(env: &TestEnv) -> (SubscriptionId, Invoice) {
    env.set_consolidate_and_grace(false, 720).await;
    halyard(env, CreditExpiry::BillingPeriod).await;
    large(env).await;
    let sub_id = start(env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(env, &invoice(env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    let february = invoice(env, sub_id, 1).await;
    change_on(env, sub_id, LARGE, date(2024, 2, 15)).await;
    (sub_id, february)
}

async fn run_sql(env: &TestEnv, sql: &str) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(sql)
        .execute(&mut conn)
        .await
        .expect("sql");
}

#[derive(diesel::QueryableByName, Debug, PartialEq)]
struct SwitchCheck {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamp>)]
    checked_at: Option<NaiveDateTime>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamp>)]
    next_check_at: Option<NaiveDateTime>,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    complete: bool,
}

async fn switch_check(env: &TestEnv, sub_id: SubscriptionId) -> SwitchCheck {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(
        "SELECT outgoing_checked_at AS checked_at, outgoing_next_check_at AS next_check_at, \
                details ? 'outgoing_complete' AS complete \
         FROM subscription_event WHERE event_type = 'SWITCH' AND subscription_id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .get_result(&mut conn)
    .await
    .expect("the switch")
}

/// Round 5, item 9. The sweep checks a switch whose draft is held, then waits 10 minutes, then
/// twice as long; once an older release issues the draft (no settlement hook), the switch is
/// due at once: its note is issued and it completes.
#[rstest]
#[tokio::test]
async fn the_sweep_backs_off_a_held_draft_and_returns_once_it_is_issued(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, february) = held_draft_switch(&env).await;
    let env = &env;
    let sweep = || async move {
        env.services()
            .settle_missed_plan_change_endings(10)
            .await
            .expect("sweep")
    };

    assert_eq!(sweep().await, 0);
    let first = switch_check(env, sub_id).await;
    let (checked_at, next_check_at) = (
        first.checked_at.expect("checked"),
        first.next_check_at.expect("next check"),
    );
    assert_eq!(next_check_at - checked_at, Duration::minutes(10));
    assert!(!first.complete);
    assert_eq!(sweep().await, 0);
    assert_eq!(
        switch_check(env, sub_id).await,
        first,
        "not checked again before its backoff"
    );

    // Ten minutes pass.
    run_sql(
        env,
        "UPDATE subscription_event SET outgoing_checked_at = outgoing_checked_at - interval '10 minutes', \
         outgoing_next_check_at = outgoing_next_check_at - interval '10 minutes' WHERE event_type = 'SWITCH'",
    )
    .await;
    run_sql(
        env,
        "UPDATE invoice SET updated_at = updated_at - interval '10 minutes'",
    )
    .await;
    assert_eq!(sweep().await, 0);
    let second = switch_check(env, sub_id).await;
    assert_eq!(
        second.next_check_at.expect("next check") - second.checked_at.expect("checked"),
        Duration::minutes(20)
    );

    run_sql(
        env,
        "UPDATE subscription_event SET details = details - 'outgoing' WHERE event_type = 'SWITCH'",
    )
    .await;
    env.process_due_events().await;
    run_sql(
        env,
        "UPDATE subscription_event SET details = details || '{\"outgoing\": \"settled\"}' \
         WHERE event_type = 'SWITCH' AND details ? 'new_plan_version_id'",
    )
    .await;
    assert_eq!(notes_on(env, &february).await, 0);
    assert_eq!(sweep().await, 1);
    assert_eq!(notes_on(env, &february).await, 1);
    assert!(switch_check(env, sub_id).await.complete);
}

/// Round 5, item 6. A finalization in flight holds the customer: the sweep waits for it before
/// reading what is creditable or completing the switch.
#[rstest]
#[tokio::test]
async fn the_sweep_waits_for_a_finalization_in_flight(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use std::sync::atomic::{AtomicBool, Ordering};

    let env = test_env.await;
    let (sub_id, february) = held_draft_switch(&env).await;
    let swept = AtomicBool::new(false);
    let mut holder = env.conn().await;
    let finalizing = async {
        diesel::sql_query("BEGIN")
            .execute(&mut holder)
            .await
            .expect("begin");
        diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
            .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
            .execute(&mut holder)
            .await
            .expect("customer locked");
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        let swept_meanwhile = AtomicBool::load(&swept, Ordering::SeqCst);
        diesel::sql_query("COMMIT")
            .execute(&mut holder)
            .await
            .expect("commit");
        swept_meanwhile
    };
    let sweeping = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let issued = env
            .services()
            .settle_missed_plan_change_endings(10)
            .await
            .expect("sweep");
        swept.store(true, Ordering::SeqCst);
        issued
    };
    let (swept_meanwhile, issued) = tokio::join!(finalizing, sweeping);
    assert!(
        !swept_meanwhile,
        "the sweep read the switch while the customer was held"
    );
    assert_eq!(issued, 0);
    assert_eq!(notes_on(&env, &february).await, 0);
    assert!(!switch_check(&env, sub_id).await.complete);
}

/// Round 5, item 7. An immediate change waits for the customer before it locks the subscription
/// or its seats: a finalization holding the customer and then reading seats never deadlocks it.
#[rstest]
#[tokio::test]
async fn a_plan_change_locks_the_customer_before_the_subscription(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let mut holder = env.conn().await;
    let mut probe = env.conn().await;
    let holding = async {
        diesel::sql_query("BEGIN")
            .execute(&mut holder)
            .await
            .expect("begin");
        diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
            .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
            .execute(&mut holder)
            .await
            .expect("customer locked");
        tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
        let free = |sql: &'static str| {
            diesel::sql_query(sql).bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        };
        let subscription_free = free("SELECT id FROM subscription WHERE id = $1 FOR UPDATE NOWAIT")
            .execute(&mut probe)
            .await
            .is_ok();
        let seats_free =
            free("SELECT id FROM slot_transaction WHERE subscription_id = $1 FOR UPDATE NOWAIT")
                .execute(&mut probe)
                .await
                .is_ok();
        diesel::sql_query("COMMIT")
            .execute(&mut holder)
            .await
            .expect("commit");
        (subscription_free, seats_free)
    };
    let changing = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        change_on(&env, sub_id, SCALE, date(2024, 1, 16)).await;
    };
    let ((subscription_free, seats_free), ()) = tokio::join!(holding, changing);
    assert!(
        subscription_free,
        "the change locked the subscription first"
    );
    assert!(seats_free, "the change locked the seats first");
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, SCALE);
}

/// Round 6, item 1. Studio's 10 seats change to Large, whose own seat product counts the same
/// unit from 3: the change records Large's 3 seats under that unit. Once February's held draft is
/// issued, Studio's seats are credited as the change read them, 10.
#[rstest]
#[tokio::test]
async fn a_deferred_seat_credit_reads_the_seats_before_the_new_plan_s(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::price_components::{DowngradePolicy, UpgradePolicy};

    let env = test_env.await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    add_component(
        &env,
        "Platform Fee",
        LARGE,
        ProductRef::Existing(PRODUCT_PLATFORM_FEE_ID),
        Pricing::Rate { rate: dec!(299) },
    )
    .await;
    add_component(
        &env,
        "Large seats",
        LARGE,
        ProductRef::New {
            name: "Large seats".to_owned(),
            fee_type: FeeTypeEnum::Slot,
            fee_structure: FeeStructure::Slot {
                unit_name: "Seats".to_owned(),
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            },
        },
        Pricing::Slot {
            unit_rate: dec!(50),
            min_slots: Some(3),
            max_slots: None,
        },
    )
    .await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "INSERT INTO slot_transaction \
             (id, subscription_id, delta, prev_active_slots, effective_at, transaction_at, unit, status) \
             SELECT gen_random_uuid(), subscription_id, 9, 1, '2024-01-20', '2024-01-20', unit, status \
             FROM slot_transaction WHERE subscription_id = $1 LIMIT 1",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("nine more seats");
    }
    env.process_cycle_transitions().await;
    let february = invoice(&env, sub_id, 1).await;
    assert!(lines_of(&february).contains(&("Seats".to_owned(), 10_000, date(2024, 3, 1))));

    change_on(&env, sub_id, LARGE, date(2024, 2, 15)).await;
    env.process_due_events().await;

    let (_, lines) = the_note(&env, &february).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -5_172, 0)
        ],
        "10000 × 15/29 = 5172.41, not Large's 3 seats"
    );
}

#[derive(diesel::QueryableByName)]
struct EventOutcome {
    #[diesel(sql_type = diesel::sql_types::Text)]
    status: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    error: Option<String>,
}

/// Round 6, item 2. A scheduled cancellation runs while a seat update (or an immediate change) of
/// the same subscription waits on its customer, held by a third transaction: the cancellation
/// queues behind both on the customer instead of holding the subscription. No deadlock.
#[rstest]
#[tokio::test]
async fn a_scheduled_cancellation_racing_a_seat_update_or_a_change_never_deadlocks(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::SlotUpgradeBillingMode;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    for round in 0..4 {
        let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
        pay(&env, &invoice(&env, sub_id, 0).await).await;
        env.services()
            .cancel_subscription(
                Actor::System,
                sub_id,
                TENANT_ID,
                None,
                CancellationEffectiveAt::Date(date(2024, 1, 20)),
            )
            .await
            .expect("cancellation scheduled");
        let seats = env
            .get_subscription_components(sub_id)
            .await
            .into_iter()
            .find(|c| c.name == "Seats")
            .and_then(|c| c.price_component_id)
            .expect("Studio's seats");

        let mut holder = env.conn().await;
        diesel::sql_query("BEGIN")
            .execute(&mut holder)
            .await
            .expect("begin");
        diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
            .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
            .execute(&mut holder)
            .await
            .expect("customer locked");
        let updating = async {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if round % 2 == 0 {
                env.services()
                    .update_subscription_slots(
                        TENANT_ID,
                        sub_id,
                        seats,
                        1,
                        SlotUpgradeBillingMode::Optimistic,
                        Actor::System,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{e:?}"))
            } else {
                env.services()
                    .apply_plan_change_immediate_at(
                        Actor::System,
                        sub_id,
                        TENANT_ID,
                        SCALE,
                        vec![],
                        date(2024, 1, 16),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{e:?}"))
            }
        };
        let cancelling = async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            env.services()
                .get_and_process_due_events()
                .await
                .expect("due events");
        };
        let releasing = async {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            diesel::sql_query("COMMIT")
                .execute(&mut holder)
                .await
                .expect("commit");
        };
        let (updated, (), ()) = tokio::join!(updating, cancelling, releasing);
        assert!(
            updated.is_ok(),
            "round {round}: the update failed: {updated:?}"
        );
        let mut conn = env.conn().await;
        let outcome: EventOutcome = diesel::sql_query(
            "SELECT status::text AS status, error FROM scheduled_event \
             WHERE subscription_id = $1 AND event_type = 'CANCEL_SUBSCRIPTION'",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .get_result(&mut conn)
        .await
        .expect("the cancellation");
        assert_eq!(
            (outcome.status.as_str(), outcome.error),
            ("COMPLETED", None),
            "round {round}"
        );
    }
}

#[derive(diesel::QueryableByName)]
struct ClaimState {
    #[diesel(sql_type = diesel::sql_types::Date)]
    current_period_start: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::Int4)]
    error_count: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamp>)]
    processing_started_at: Option<NaiveDateTime>,
}

async fn claim_state(env: &TestEnv, sub_id: SubscriptionId) -> ClaimState {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(
        "SELECT current_period_start, error_count, processing_started_at \
         FROM subscription WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .get_result(&mut conn)
    .await
    .expect("subscription")
}

/// Round 6, item 4. A renewal whose customer another transaction holds is deferred without
/// waiting and without counting a failure, then renewed on the first tick after the deferral.
#[rstest]
#[tokio::test]
async fn the_lifecycle_worker_skips_a_busy_customer_until_the_next_tick(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let mut holder = env.conn().await;
    diesel::sql_query("BEGIN")
        .execute(&mut holder)
        .await
        .expect("begin");
    diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
        .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
        .execute(&mut holder)
        .await
        .expect("customer locked");
    let tick = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        env.services().get_and_process_cycle_transitions(),
    )
    .await
    .expect("the worker waited on the customer")
    .expect("tick");
    assert_eq!((tick.processed, tick.has_more), (0, false));
    let skipped = claim_state(&env, sub_id).await;
    assert_eq!(
        (
            skipped.current_period_start,
            skipped.error_count,
            skipped.processing_started_at
        ),
        (date(2024, 1, 1), 0, None)
    );
    diesel::sql_query("COMMIT")
        .execute(&mut holder)
        .await
        .expect("commit");
    let tick = env
        .services()
        .get_and_process_cycle_transitions()
        .await
        .expect("tick");
    assert_eq!(tick.processed, 0, "deferred, not claimed again at once");
    diesel::sql_query(
        "UPDATE subscription SET next_retry = now() - interval '1 second' WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("the deferral is over");

    let tick = env
        .services()
        .get_and_process_cycle_transitions()
        .await
        .expect("tick");
    assert_eq!(tick.processed, 1);
    let renewed = claim_state(&env, sub_id).await;
    assert_eq!(
        (renewed.current_period_start, renewed.error_count),
        (date(2024, 2, 1), 0)
    );
}

/// Round 6, item 5. Both February drafts wait to be consolidated; the sweep checks the switch and
/// backs off. An older release then issues the consolidated invoice (no settlement hook): the
/// switch is due at once, its note is issued on the consolidated invoice and it completes.
#[rstest]
#[tokio::test]
async fn the_sweep_returns_once_a_consolidated_invoice_is_issued(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_consolidate_and_grace(true, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    large(&env).await;
    let scale_sub = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let studio_sub = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, scale_sub, 0).await).await;
    pay(&env, &invoice(&env, studio_sub, 0).await).await;
    env.process_cycle_transitions().await;
    change_on(&env, studio_sub, LARGE, date(2024, 2, 15)).await;
    let sweep = || async {
        env.services()
            .settle_missed_plan_change_endings(10)
            .await
            .expect("sweep")
    };

    assert_eq!(sweep().await, 0);
    let backed_off = switch_check(&env, studio_sub).await;
    assert!(backed_off.next_check_at.expect("next check") > Utc::now().naive_utc());

    run_sql(
        &env,
        "UPDATE subscription_event SET details = details - 'outgoing' WHERE event_type = 'SWITCH'",
    )
    .await;
    env.process_due_events().await;
    run_sql(
        &env,
        "UPDATE subscription_event SET details = details || '{\"outgoing\": \"settled\"}' \
         WHERE event_type = 'SWITCH' AND details ? 'new_plan_version_id'",
    )
    .await;
    let parent = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .find(|i| i.subscription_id.is_none())
        .expect("the consolidated February invoice");
    assert_eq!(notes_on(&env, &parent).await, 0);

    assert_eq!(sweep().await, 1);
    let (_, lines) = the_note(&env, &parent).await;
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ]
    );
    assert!(switch_check(&env, studio_sub).await.complete);
}

#[derive(diesel::QueryableByName)]
struct Plan {
    #[diesel(sql_type = diesel::sql_types::Json, column_name = "QUERY PLAN")]
    plan: serde_json::Value,
}

fn rows_read(node: &serde_json::Value, relation: &str) -> f64 {
    let number = |key: &str| node[key].as_f64().unwrap_or(0.0);
    let own = if node["Relation Name"] == relation {
        (number("Actual Rows") + number("Rows Removed by Filter"))
            * node["Actual Loops"].as_f64().unwrap_or(1.0)
    } else {
        0.0
    };
    own + node["Plans"]
        .as_array()
        .map(|plans| plans.iter().map(|plan| rows_read(plan, relation)).sum())
        .unwrap_or(0.0)
}

/// Round 6, item 5. Thirty switches still to settle, all but the last backing off: a page reads
/// at most a page of them (a sweep is linear), and the sweep walks past pages with nothing due to
/// complete the last one.
#[rstest]
#[tokio::test]
async fn a_sweep_page_reads_one_page_of_switches(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use diesel_models::subscription_events::INCOMPLETE_SETTLING_SWITCHES_PAGE;

    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "INSERT INTO subscription_event \
             (id, subscription_id, event_type, details, created_at, applies_to, outgoing_checked_at, outgoing_next_check_at) \
             SELECT gen_random_uuid(), $1, 'SWITCH', '{\"outgoing\": \"settled\"}', now(), \
                    DATE '2024-03-01' + n, now(), CASE WHEN n < 29 THEN now() + interval '1 day' END \
             FROM generate_series(0, 29) AS n",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("switches");
    }

    let mut conn = env.conn().await;
    diesel::sql_query("SET enable_seqscan = off")
        .execute(&mut conn)
        .await
        .expect("no seq scan");
    diesel::sql_query("SET enable_bitmapscan = off")
        .execute(&mut conn)
        .await
        .expect("no bitmap scan");
    let plan: Plan = diesel::sql_query(format!(
        "EXPLAIN (ANALYZE, FORMAT JSON) {INCOMPLETE_SETTLING_SWITCHES_PAGE}"
    ))
    .bind::<diesel::sql_types::Date, _>(date(1, 1, 1))
    .bind::<diesel::sql_types::Uuid, _>(Uuid::nil())
    .bind::<diesel::sql_types::BigInt, _>(3_i64)
    .bind::<diesel::sql_types::Timestamp, _>(Utc::now().naive_utc())
    .get_result(&mut conn)
    .await
    .expect("plan");
    let read = rows_read(&plan.plan[0]["Plan"], "subscription_event");
    assert!(
        (1.0..=4.0).contains(&read),
        "a page of 3 read {read} switches: {}",
        plan.plan
    );

    assert_eq!(
        env.services()
            .settle_missed_plan_change_endings(1)
            .await
            .expect("sweep"),
        0
    );
    #[derive(diesel::QueryableByName)]
    struct Day {
        #[diesel(sql_type = diesel::sql_types::Date)]
        applies_to: NaiveDate,
    }
    let completed: Vec<Day> = diesel::sql_query(
        "SELECT applies_to FROM subscription_event \
         WHERE subscription_id = $1 AND details ? 'outgoing_complete'",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .load(&mut conn)
    .await
    .expect("completed");
    assert_eq!(
        completed.iter().map(|d| d.applies_to).collect::<Vec<_>>(),
        vec![date(2024, 3, 30)]
    );
}

/// The wallet.credit_unrecovered notices keyed by `payment` and no invoice:
/// (cause, unrecovered, total so far).
async fn payment_keyed_notices(
    env: &TestEnv,
    payment: common_domain::ids::PaymentTransactionId,
) -> Vec<(String, i64, i64)> {
    use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
    use meteroid_store::domain::outbox_event::OutboxEvent;
    use meteroid_store::domain::pgmq::PgmqQueue;
    use meteroid_store::repositories::pgmq::PgmqInterface;

    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditUnrecovered(event) => {
                let fields = serde_json::to_value(&event).ok()?;
                (fields["payment_id"] == serde_json::to_value(payment).ok()?
                    && fields["invoice_id"].is_null())
                .then(|| {
                    (
                        event.cause.as_str().to_owned(),
                        event.unrecovered_minor,
                        event.unrecovered_total_minor,
                    )
                })
            }
            _ => None,
        })
        .collect()
}

/// The wallet.credit_unrecovered notices of `invoice` the webhook queue holds:
/// (cause, taken back, unrecovered, total so far).
async fn unrecovered_notices(env: &TestEnv, invoice: &Invoice) -> Vec<(String, i64, i64, i64)> {
    use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
    use meteroid_store::domain::outbox_event::OutboxEvent;
    use meteroid_store::domain::pgmq::PgmqQueue;
    use meteroid_store::repositories::pgmq::PgmqInterface;

    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditUnrecovered(event) if event.invoice_id == Some(invoice.id) => {
                Some((
                    event.cause.as_str().to_owned(),
                    event.taken_back_minor,
                    event.unrecovered_minor,
                    event.unrecovered_total_minor,
                ))
            }
            _ => None,
        })
        .collect()
}

/// Spends `cents` of the customer-wide credit on a one-off invoice of the customer.
async fn spend_on_one_off(env: &TestEnv, cents: i64) {
    let spend = crate::helpers::invoices::draft(
        &env.setup,
        vec![crate::helpers::invoice_lines::line("Support", cents, false)],
    )
    .await
    .expect("one-off draft");
    let spent = crate::helpers::invoices::finalize(&env.setup, spend.id)
        .await
        .expect("finalized")
        .invoice;
    assert_eq!(
        spent.applied_credits, cents,
        "paid with the overpayment credit"
    );
}

async fn processed_at(
    env: &TestEnv,
    payment: common_domain::ids::PaymentTransactionId,
) -> DateTime<Utc> {
    let mut conn = env.conn().await;
    diesel_models::payments::PaymentTransactionRow::get_by_id(&mut conn, payment, TENANT_ID)
        .await
        .expect("payment")
        .processed_at
        .expect("settled")
        .and_utc()
}

/// S14 (OQ12). The 301.93 plan-change adjustment is paid 341.93 by the provider: the pack is
/// granted (258.06 paid + 25.81 bonus) and the 40.00 above it is kept as OVERPAYMENT credit
/// dated at the collection. The payment fails late: the overpayment and the pack are taken
/// back. A 301.93 replacement restores the same pack grants and keeps nothing above.
#[rstest]
#[tokio::test]
async fn s14_an_overpaid_plan_change_invoice_keeps_overpayment_credit_and_reinstates_the_same_grants(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let (adjustment, session) = plan_change_adjustment(&env).await;
    let original = pending_payment(&env, adjustment.id, Some(session), 34_193).await;
    env.deliver_mock_webhook(succeeded_payload(original, 34_193).as_bytes())
        .await;

    assert_eq!(reread(&env, &adjustment).await.amount_due, 0);
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(
        (
            kept.amount,
            kept.remaining,
            kept.classification.as_str(),
            kept.expires_at,
            kept.subscription_id,
            kept.source_id,
        ),
        (4_000, 4_000, "PAID", None, None, original.as_uuid())
    );
    assert_eq!(kept.effective_at, processed_at(&env, original).await);
    let pack_grants = |grants: Vec<String>| {
        grants
            .into_iter()
            .filter(|grant| grant.contains("CREDIT_PURCHASE"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pack_grants(grants(&env, CUST_UBER_ID).await),
        vec![
            "EUR CREDIT_PURCHASE PAID 25806 25806".to_owned(),
            "EUR CREDIT_PURCHASE PROMOTIONAL 2581 2581".to_owned(),
        ]
    );

    env.deliver_mock_webhook(
        late_failure_payload(&format!("pi_{}", original.as_base62())).as_bytes(),
    )
    .await;
    assert_eq!(reread(&env, &adjustment).await.amount_due, 30_193);
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(
        movements(&env, &kept).await,
        vec!["FUND 4000".to_owned(), "REVERSAL -4000".to_owned()]
    );
    assert_eq!(
        pack_grants(grants(&env, CUST_UBER_ID).await),
        vec![
            "EUR CREDIT_PURCHASE PAID 25806 0".to_owned(),
            "EUR CREDIT_PURCHASE PROMOTIONAL 2581 0".to_owned(),
        ]
    );

    let replacement = pending_payment(&env, adjustment.id, None, 30_193).await;
    env.deliver_mock_webhook(succeeded_payload(replacement, 30_193).as_bytes())
        .await;
    assert_eq!(reread(&env, &adjustment).await.amount_due, 0);
    assert_eq!(kept_for(&env, &adjustment).await, 0);
    assert_eq!(
        grant_rows(&env, CUST_UBER_ID)
            .await
            .iter()
            .filter(|g| g.source_kind == "OVERPAYMENT")
            .count(),
        1,
        "nothing new is kept"
    );
    assert_eq!(
        pack_grants(grants(&env, CUST_UBER_ID).await),
        vec![
            "EUR CREDIT_PURCHASE PAID 25806 25806".to_owned(),
            "EUR CREDIT_PURCHASE PROMOTIONAL 2581 2581".to_owned(),
        ],
        "the same pack grants are reinstated"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// S14 variant. 20.00 of the 40.00 overpayment credit is spent on another invoice before the
/// payment is charged back: 20.00 is taken back and the 20.00 spent is reported lost to the
/// dispute, like spent purchased credit. Winning the dispute restores it and withdraws the loss.
#[rstest]
#[tokio::test]
async fn spent_overpayment_credit_is_reported_unrecovered_when_its_payment_is_reversed(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    let (adjustment, session) = plan_change_adjustment(&env).await;
    let original = pending_payment(&env, adjustment.id, Some(session), 34_193).await;
    env.deliver_mock_webhook(succeeded_payload(original, 34_193).as_bytes())
        .await;
    spend_on_one_off(&env, 2_000).await;

    let external_id = format!("pi_{}", original.as_base62());
    env.deliver_mock_webhook(charged_back_payload(&external_id).as_bytes())
        .await;
    let kept = overpayment_credit(&env, &adjustment).await;
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 4000".to_owned(),
            "DRAW -2000".to_owned(),
            "REVERSAL -2000".to_owned()
        ]
    );
    assert_eq!(
        unrecovered_notices(&env, &adjustment).await,
        vec![("PAYMENT_DISPUTED".to_owned(), 2_000, 2_000, 2_000)]
    );

    env.deliver_mock_webhook(reinstated_payload(&external_id).as_bytes())
        .await;
    assert_eq!(kept_for(&env, &adjustment).await, 2_000);
    assert_eq!(
        unrecovered_notices(&env, &adjustment).await.len(),
        1,
        "the loss is withdrawn by a recovered notice"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// OQ12. Studio's first period invoice (39.00) overpaid by a 50.00 provider settlement keeps
/// 11.00 as OVERPAYMENT credit like any other invoice; a chargeback takes it back and leaves the
/// invoice paid, a reinstatement restores the same grant.
#[rstest]
#[tokio::test]
async fn a_recurring_invoice_overpaid_by_a_provider_settlement_keeps_the_excess(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    let period = invoice(&env, sub_id, 0).await;
    assert_eq!((period.total, period.amount_due), (3_900, 3_900));

    let payment = pending_payment(&env, period.id, None, 5_000).await;
    env.deliver_mock_webhook(succeeded_payload(payment, 5_000).as_bytes())
        .await;
    assert_eq!(reread(&env, &period).await.amount_due, 0);
    let kept = overpayment_credit(&env, &period).await;
    assert_eq!(
        (kept.amount, kept.remaining, kept.source_id, kept.reference),
        (1_100, 1_100, payment.as_uuid(), Some(payment.to_string()))
    );
    assert_eq!(kept.effective_at, processed_at(&env, payment).await);

    let external_id = format!("pi_{}", payment.as_base62());
    env.deliver_mock_webhook(charged_back_payload(&external_id).as_bytes())
        .await;
    assert_eq!(reread(&env, &period).await.amount_due, 0);
    assert_eq!(kept_for(&env, &period).await, 0);

    env.deliver_mock_webhook(reinstated_payload(&external_id).as_bytes())
        .await;
    assert_eq!(reread(&env, &period).await.amount_due, 0);
    let kept = overpayment_credit(&env, &period).await;
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 1100".to_owned(),
            "REVERSAL -1100".to_owned(),
            "REINSTATEMENT 1100".to_owned()
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Studio's first period invoice (39.00) paid 50.00 by the provider: 11.00 kept as credit.
async fn overpaid_period(env: &TestEnv) -> (Invoice, common_domain::ids::PaymentTransactionId) {
    env.seed_payments().await;
    halyard(env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(env, CUST_UBER_ID, STUDIO, date(2024, 1, 1)).await;
    let period = invoice(env, sub_id, 0).await;
    let payment = pending_payment(env, period.id, None, 5_000).await;
    env.deliver_mock_webhook(succeeded_payload(payment, 5_000).as_bytes())
        .await;
    (period, payment)
}

async fn refund_without_note(
    env: &TestEnv,
    payment: common_domain::ids::PaymentTransactionId,
    amount: i64,
) -> meteroid_store::StoreResult<meteroid_store::domain::PaymentTransaction> {
    env.services()
        .refund_transaction(
            Actor::System,
            TENANT_ID,
            meteroid_store::services::RefundCommand {
                parent_transaction_id: payment,
                amount,
                credit_note_id: None,
                reason: Some("duplicate charge".to_owned()),
            },
        )
        .await
}

/// 8.00 of the 11.00 overpayment credit paid another invoice: a refund without a credit note may
/// give back only the 3.00 left, and takes it back from the credit; nothing is lost.
#[rstest]
#[tokio::test]
async fn a_refund_without_a_note_gives_back_only_the_unspent_overpayment(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (period, payment) = overpaid_period(&env).await;
    spend_on_one_off(&env, 800).await;

    let refused = refund_without_note(&env, payment, 1_100)
        .await
        .expect_err("the spent credit cannot be refunded too");
    assert!(
        format!("{refused:?}").contains("exceeds the unspent overpayment (300)"),
        "{refused:?}"
    );
    refund_without_note(&env, payment, 300)
        .await
        .expect("the unspent part is refunded");

    let kept = overpayment_credit(&env, &period).await;
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 1100".to_owned(),
            "DRAW -800".to_owned(),
            "REVERSAL -300".to_owned()
        ]
    );
    assert!(unrecovered_notices(&env, &period).await.is_empty());
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A refund of the 11.00 overpayment still on its way to the customer already holds its credit
/// back: the credit cannot pay another invoice meanwhile.
#[rstest]
#[tokio::test]
async fn a_refund_in_flight_holds_its_overpayment_credit_back(#[future] test_env: TestEnv) {
    use meteroid_store::domain::connectors::MockRefundBehavior;

    let env = test_env.await;
    let (period, payment) = overpaid_period(&env).await;
    env.set_mock_refund_behavior(MockRefundBehavior::Pending)
        .await;
    refund_without_note(&env, payment, 1_100)
        .await
        .expect("the refund is in flight");

    let kept = overpayment_credit(&env, &period).await;
    assert_eq!(
        movements(&env, &kept).await,
        vec!["FUND 1100".to_owned(), "REVERSAL -1100".to_owned()]
    );
    let spend = crate::helpers::invoices::draft(
        &env.setup,
        vec![crate::helpers::invoice_lines::line("Support", 500, false)],
    )
    .await
    .expect("one-off draft");
    let spent = crate::helpers::invoices::finalize(&env.setup, spend.id)
        .await
        .expect("finalized")
        .invoice;
    assert_eq!(
        spent.applied_credits, 0,
        "the refunded credit is not spendable"
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A refund of the 11.00 overpayment the provider refuses gives nothing back: the credit it held
/// back is restored.
#[rstest]
#[tokio::test]
async fn a_failed_refund_restores_the_overpayment_credit(#[future] test_env: TestEnv) {
    use meteroid_store::domain::connectors::MockRefundBehavior;

    let env = test_env.await;
    let (period, payment) = overpaid_period(&env).await;
    env.set_mock_refund_behavior(MockRefundBehavior::Failed)
        .await;
    let refused = refund_without_note(&env, payment, 1_100)
        .await
        .expect_err("the provider refuses the refund");
    assert!(matches!(
        refused.current_context(),
        meteroid_store::errors::StoreError::PaymentError(_)
    ));

    let kept = overpayment_credit(&env, &period).await;
    assert_eq!(kept.remaining, 1_100);
    assert_eq!(
        movements(&env, &kept).await,
        vec![
            "FUND 1100".to_owned(),
            "REVERSAL -1100".to_owned(),
            "REINSTATEMENT 1100".to_owned()
        ]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// m3. A refund of the whole 11.00 excess made at the provider (observed after the fact, so it
/// cannot be refused) after 8.00 of the credit was spent: 3.00 is taken back and the 8.00 lost
/// is reported as refunded, not as a bank reversal.
#[rstest]
#[tokio::test]
async fn a_provider_refund_of_spent_overpayment_credit_is_reported_as_refunded(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (period, payment) = overpaid_period(&env).await;
    spend_on_one_off(&env, 800).await;

    let external_id = format!("pi_{}", payment.as_base62());
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_re_1100","kind":"payment_refunded","external_id":"{external_id}","amount":1100,"currency":"EUR","refunds":[{{"id":"re_dashboard_1","amount":1100,"succeeded":true}}]}}"#
        )
        .as_bytes(),
    )
    .await;

    assert_eq!(
        unrecovered_notices(&env, &period).await,
        vec![("PAYMENT_REFUNDED".to_owned(), 300, 800, 800)]
    );
    crate::helpers::credit::assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

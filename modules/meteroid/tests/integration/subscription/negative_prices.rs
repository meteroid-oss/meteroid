//! Negative per-unit usage rates (R-142): a reversal priced below zero, billed by the real engine.
//!
//! A tenant prices a "routing commission" at +0.50 a unit and a "routing commission reversal" at
//! −0.50 a unit. A negative per-unit usage rate is the one negative price the catalog takes (every
//! other is refused, the pricer billing it at 0). These tests seed such prices and drive the
//! engine (controller, finalize, credit, coupons, tax, minimum, payments), printing every
//! document they produce, and assert that the reversal nets like any other line.
//!
//! The commission is billed on the Bandwidth metric (a sum); the reversal on a metric of its own
//! ("reversed volume", a sum) and a product of its own, unless a test says otherwise. Every
//! subscription starts on 2024-09-01, monthly, its usage billed in arrears on the invoice that
//! opens October (dated over September).
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{
    BaseId, BillableMetricId, CouponId, CustomerId, PlanId, PlanVersionId, PriceComponentId,
    PriceId, ProductId, SubscriptionId,
};
use diesel_async::RunQueryDsl;
use diesel_models::billable_metrics::BillableMetricRowNew;
use diesel_models::enums::{
    BillingMetricAggregateEnum, BillingPeriodEnum as DieselBillingPeriodEnum,
};
use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
use diesel_models::products::ProductRowNew;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::credit::CreditGrantSource;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, TaxResolverEnum};
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::{
    Address, CustomerTaxRate, Invoice, InvoicingEntityPatch, LineItemKind, UsagePeriod,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A day of 2024.
fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

/// 09:00 UTC on `on`.
fn at(on: NaiveDate) -> DateTime<Utc> {
    on.and_hms_opt(9, 0, 0).unwrap().and_utc()
}

async fn usage_env() -> (TestEnv, MutableMockUsageClient) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    (env, usage)
}

/// `units` of `metric` used over `[from, to)`, read at midnight as the billing windows are.
fn set_usage(
    usage: &MutableMockUsageClient,
    metric_id: BillableMetricId,
    from: NaiveDate,
    to: NaiveDate,
    units: Decimal,
) {
    let (start, end) = (
        from.and_time(chrono::NaiveTime::MIN),
        to.and_time(chrono::NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: units,
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

async fn new_product(env: &TestEnv, name: &str) -> ProductId {
    let id = ProductId::new();
    ProductRowNew {
        id,
        name: name.to_string(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        catalog: true,
        tax_category_id: None,
        icon: None,
        color: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("a product");
    id
}

/// A summed metric of the volume reversed, as the user meters reversals.
async fn reversal_metric(env: &TestEnv) -> BillableMetricId {
    let id = BillableMetricId::new();
    BillableMetricRowNew {
        id,
        name: "Reversed volume".to_string(),
        description: None,
        code: "reversed_volume".to_string(),
        catalog_code: None,
        aggregation_type: BillingMetricAggregateEnum::Sum,
        aggregation_key: Some("value".to_string()),
        unit_conversion_factor: Some(1),
        unit_conversion_rounding: None,
        segmentation_matrix: None,
        usage_group_key: None,
        usage_breakdown_key: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        product_id: None,
        filters: None,
        unit_singular: None,
        unit_plural: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("a metric");
    id
}

/// A charge of a plan, as its lines are told apart: by its product, or its component.
#[derive(Clone, Copy)]
struct Charge {
    component: PriceComponentId,
    product: ProductId,
}

impl Charge {
    fn new(product: ProductId) -> Self {
        Self {
            component: PriceComponentId::new(),
            product,
        }
    }
}

struct Routing {
    version: PlanVersionId,
    commission: Charge,
    reversal: Charge,
    reversal_metric: BillableMetricId,
}

/// "Routing commission" at +0.50 per unit of Bandwidth, and "Routing commission reversal"
/// priced `reversal` per unit of reversed volume (its own metric and product), both in arrears.
async fn routing_plan(env: &TestEnv, name: &'static str, reversal: UsagePricingModel) -> Routing {
    let reversal_metric = reversal_metric(env).await;
    let commission = Charge::new(PRODUCT_BANDWIDTH_ID);
    let reversal_charge = Charge::new(new_product(env, "Routing commission reversal").await);
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(plan, name, version)
        .components(vec![
            SeedComp::usage(
                commission.component,
                "Routing commission",
                commission.product,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.50) },
            ),
            SeedComp::usage(
                reversal_charge.component,
                "Routing commission reversal",
                reversal_charge.product,
                reversal_metric,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                reversal,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("the routing plan");
    Routing {
        version,
        commission,
        reversal: reversal_charge,
        reversal_metric,
    }
}

/// The user's plan: the reversal at −0.50 per unit.
async fn user_routing_plan(env: &TestEnv) -> Routing {
    routing_plan(
        env,
        "Routing",
        UsagePricingModel::PerUnit { rate: dec!(-0.50) },
    )
    .await
}

/// September's usage: `commission` units of Bandwidth, `reversed` of reversed volume.
fn september_usage(
    usage: &MutableMockUsageClient,
    routing: &Routing,
    commission: i64,
    reversed: i64,
) {
    set_usage(
        usage,
        METRIC_BANDWIDTH,
        day(9, 1),
        day(10, 1),
        Decimal::from(commission),
    );
    set_usage(
        usage,
        routing.reversal_metric,
        day(9, 1),
        day(10, 1),
        Decimal::from(reversed),
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
        .expect("a minimum");
}

/// Subscribed on 2024-09-01, that day.
async fn start(
    env: &TestEnv,
    version: PlanVersionId,
    customer: CustomerId,
    coupons: Vec<CouponId>,
) -> SubscriptionId {
    env.set_now(at(day(9, 1)));
    subscription()
        .customer(customer)
        .plan_version(version)
        .start_date(day(9, 1))
        .on_start()
        .no_trial()
        .coupons(coupons)
        .create(env.services())
        .await
}

/// Lives through October 1st: the renewal closes September and bills its usage; a day later,
/// past the grace period, the invoice is issued and its outbox work runs.
async fn close_september(env: &TestEnv) {
    env.set_now(at(day(10, 1)));
    env.process_cycles().await;
    env.set_now(at(day(10, 2)));
    env.process_due_events().await;
    env.run_outbox_and_orchestration().await;
}

/// The invoice billing September's usage, issued: one still in draft after its grace is
/// finalized by hand (and says so), so every test reads an issued document.
async fn september_invoice(env: &TestEnv, sub: SubscriptionId) -> Invoice {
    let invoices = env.get_invoices(sub).await;
    let found = invoices
        .iter()
        .find(|invoice| {
            invoice.status != InvoiceStatusEnum::Void
                && invoice
                    .line_items
                    .iter()
                    .any(|line| line.metric_id.is_some() && line.start_date == day(9, 1))
        })
        .or_else(|| {
            invoices.iter().find(|invoice| {
                invoice.status != InvoiceStatusEnum::Void
                    && invoice.billing_period_start == day(10, 1)
            })
        })
        .cloned();
    let Some(invoice) = found else {
        let row = env.get_subscription(sub).await;
        for invoice in &invoices {
            show("an invoice of the subscription", invoice);
        }
        panic!(
            "no invoice bills September's usage; subscription billing failures {}, last error {:?}",
            row.billing_failures, row.billing_last_error
        );
    };
    issued(env, invoice).await
}

/// `invoice` as issued: finalized by hand if still a draft, then re-read.
async fn issued(env: &TestEnv, invoice: Invoice) -> Invoice {
    if invoice.status == InvoiceStatusEnum::Draft {
        println!(
            "NOTE: invoice {} is still Draft after its grace period; finalizing it by hand",
            invoice.id
        );
        if let Err(err) = env.manual_finalize_invoice(invoice.id).await {
            show("the draft that could not be finalized", &invoice);
            panic!("finalizing the invoice failed: {err:?}");
        }
        env.run_outbox_and_orchestration().await;
    }
    reread(env, &invoice).await
}

async fn reread(env: &TestEnv, invoice: &Invoice) -> Invoice {
    env.store()
        .get_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .expect("the invoice")
}

/// Prints a document as an operator reads it.
fn show(label: &str, invoice: &Invoice) {
    println!(
        "=== {label}: invoice {} {:?}, payment {:?}, type {:?}, dated {}, period from {}",
        invoice.id,
        invoice.status,
        invoice.payment_status,
        invoice.invoice_type,
        invoice.invoice_date,
        invoice.billing_period_start
    );
    for line in &invoice.line_items {
        println!(
            "    line {:<30} kind {:?} [{} .. {}) qty {:?} unit {:?} subtotal {} taxable {} rate {} tax {} total {} prorated {} sublines {:?}",
            line.name,
            line.kind,
            line.start_date,
            line.end_date,
            line.quantity,
            line.unit_price,
            line.amount_subtotal,
            line.taxable_amount,
            line.tax_rate,
            line.tax_amount,
            line.amount_total,
            line.is_prorated,
            line.sub_lines
                .iter()
                .map(|sub| (sub.name.clone(), sub.quantity, sub.unit_price, sub.total))
                .collect::<Vec<_>>()
        );
    }
    println!(
        "    subtotal {} discount {} promotional {} tax {} total {} applied credits {} amount due {}",
        invoice.subtotal,
        invoice.discount,
        invoice.promotional_credits,
        invoice.tax_amount,
        invoice.total,
        invoice.applied_credits,
        invoice.amount_due
    );
}

/// The subtotals of `charge`'s lines on `invoice`.
fn amounts_of(invoice: &Invoice, charge: Charge) -> Vec<i64> {
    invoice
        .line_items
        .iter()
        .filter(|line| {
            line.kind == LineItemKind::Regular
                && (line.product_id == Some(charge.product)
                    || line.price_component_id == Some(charge.component))
        })
        .map(|line| line.amount_subtotal)
        .collect()
}

fn line_summary(invoice: &Invoice) -> Vec<(String, i64, i64)> {
    invoice
        .line_items
        .iter()
        .map(|line| (line.name.clone(), line.amount_subtotal, line.tax_amount))
        .collect()
}

/// `(subtotal, discount, tax, total, amount due)`.
fn totals(invoice: &Invoice) -> (i64, i64, i64, i64, i64) {
    (
        invoice.subtotal,
        invoice.discount,
        invoice.tax_amount,
        invoice.total,
        invoice.amount_due,
    )
}

/// The customer's grants funded by a negative invoice: `(amount, invoice)`.
async fn negative_invoice_grants(
    env: &TestEnv,
    customer: CustomerId,
) -> Vec<(i64, Option<common_domain::ids::InvoiceId>)> {
    crate::helpers::credit::wallets(env.store(), TENANT_ID, customer)
        .await
        .iter()
        .flat_map(|wallet| wallet.grants.iter())
        .filter(|grant| grant.source == CreditGrantSource::NegativeInvoice)
        .map(|grant| (grant.amount_minor, grant.invoice_id))
        .collect()
}

async fn manual_tax(env: &TestEnv) {
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("the manual tax resolver");
}

/// A taxable company customer in EUR whose own tax rate is 20%.
async fn customer_taxed_at_20_percent(env: &TestEnv) -> CustomerId {
    let customer_id = CustomerId::new();
    diesel_models::customers::CustomerRowNew {
        id: customer_id,
        name: "Taxed at 20%".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(
            serde_json::to_value(Address {
                line1: Some("1 Main Street".to_string()),
                line2: None,
                city: Some("Columbus".to_string()),
                country: Some(CountryCode::from_str("US").expect("country")),
                state: Some("OH".to_string()),
                zip_code: Some("43004".to_string()),
            })
            .expect("address"),
        ),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::to_value(vec![CustomerTaxRate {
            tax_code: "SALES_TAX".to_string(),
            name: "Sales tax".to_string(),
            rate: dec!(0.20),
        }])
        .expect("taxes"),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: false,
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

/// The user's routing plan subscribed by Uber, `commission` and `reversed` units used in
/// September: the issued invoice of September's usage.
async fn bill_routing(
    commission: i64,
    reversed: i64,
) -> (TestEnv, Routing, SubscriptionId, Invoice) {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![]).await;
    september_usage(&usage, &routing, commission, reversed);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    (env, routing, sub, invoice)
}

// ---------------------------------------------------------------------------------------------
// 1. Usage commission + reversal
// ---------------------------------------------------------------------------------------------

/// The user's case. 100 units of commission at +0.50 = +5000; 20 units reversed at −0.50 =
/// −1000. Two lines, subtotal 4000, no tax, total 4000, all of it due; the invoice is issued.
#[tokio::test]
async fn s01_a_reversal_priced_negative_per_unit_credits_its_line() {
    let (_env, routing, _sub, invoice) = bill_routing(100, 20).await;
    show("S1 commission 100 / reversal 20", &invoice);

    assert_eq!(
        amounts_of(&invoice, routing.commission),
        vec![5_000],
        "commission: 100 × 0.50: {:?}",
        line_summary(&invoice)
    );
    assert_eq!(
        amounts_of(&invoice, routing.reversal),
        vec![-1_000],
        "reversal: 20 × −0.50: {:?}",
        line_summary(&invoice)
    );
    assert_eq!(totals(&invoice), (4_000, 0, 0, 4_000, 4_000));
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
}

/// The same plan written on ONE metric: commission and reversal both read Bandwidth (products
/// Bandwidth and API Calls), 100 units: +5000 and −5000. Both lines are shown, the invoice nets
/// to 0 and is issued (it has lines; only an empty period is Closed), nothing due.
#[tokio::test]
async fn s01b_a_reversal_on_the_commissions_own_metric_nets_it_to_zero() {
    let (env, usage) = usage_env().await;
    let commission = Charge::new(PRODUCT_BANDWIDTH_ID);
    let reversal = Charge::new(PRODUCT_API_CALLS_ID);
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Routing on one metric", version)
        .components(vec![
            SeedComp::usage(
                commission.component,
                "Routing commission",
                commission.product,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.50) },
            ),
            SeedComp::usage(
                reversal.component,
                "Routing commission reversal",
                reversal.product,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(-0.50) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    let sub = start(&env, version, CUST_UBER_ID, vec![]).await;
    set_usage(&usage, METRIC_BANDWIDTH, day(9, 1), day(10, 1), dec!(100));
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S1b one metric, 100 units", &invoice);

    assert_eq!(amounts_of(&invoice, commission), vec![5_000]);
    assert_eq!(amounts_of(&invoice, reversal), vec![-5_000]);
    assert_eq!(totals(&invoice), (0, 0, 0, 0, 0));
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
}

// ---------------------------------------------------------------------------------------------
// 2. Reversal larger than the commission
// ---------------------------------------------------------------------------------------------

/// 20 units of commission (+1000), 100 reversed (−5000): subtotal −4000. RULES R7 and
/// `finalize_or_settle_invoice_tx`: a negative invoice is issued at its negative total, nothing
/// is due, and what it owes the customer funds 4000 of paid, customer-wide NEGATIVE_INVOICE
/// credit (never a refund). OQ21 leaves only corrections of PAID invoices undecided.
#[tokio::test]
async fn s02_a_reversal_larger_than_the_commission_owes_the_customer_credit() {
    let (env, routing, _sub, invoice) = bill_routing(20, 100).await;
    show("S2 commission 20 / reversal 100", &invoice);

    assert_eq!(amounts_of(&invoice, routing.commission), vec![1_000]);
    assert_eq!(
        amounts_of(&invoice, routing.reversal),
        vec![-5_000],
        "{:?}",
        line_summary(&invoice)
    );
    assert_eq!(totals(&invoice), (-4_000, 0, 0, -4_000, 0));
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        invoice.applied_credits, 0,
        "a negative invoice draws no credit"
    );
    println!(
        "S2 customer credit after: {}",
        env.invoice_credit_balance_minor(CUST_UBER_ID).await
    );
    assert_eq!(
        negative_invoice_grants(&env, CUST_UBER_ID).await,
        vec![(4_000, Some(invoice.id))],
        "the 4000 owed funds NEGATIVE_INVOICE credit once"
    );
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 4_000);
}

// ---------------------------------------------------------------------------------------------
// 3. A negative flat recurring fee
// ---------------------------------------------------------------------------------------------

/// Platform +49.00 and Loyalty rebate −10.00 a month in advance, from Sep 1: the first invoice
/// bills +4900 and −1000, subtotal and total 3900, all due. MRR is the monthly net, 3900.
/// Started Sep 16 with its billing anchored on the 1st, the first period is 15 of September's 30
/// days: Platform 4900 × 15/30 = 2450, rebate −1000 × 15/30 = −500, total 1950. MRR stays the
/// monthly 3900.
/// The full September invoice (4900 − 1000) issued, then the subscription cancelled from Sep 16:
/// 15 of 30 days unused. A correct system gives back the unused part of the NET fee: Platform
/// −2450 and the rebate's unused −500 taken back (+500), so September nets Platform 2450, rebate
/// −500, 1950 in all — however the engine splits it between notes and invoices.
/// Scenario 1 for a customer taxed at 20% (manual resolver): commission 5000 taxed 1000,
/// reversal −1000 taxed −200; tax 800, total 4800, all due.
#[tokio::test]
async fn s04a_a_negative_line_carries_negative_tax() {
    let (env, usage) = usage_env().await;
    manual_tax(&env).await;
    let customer = customer_taxed_at_20_percent(&env).await;
    let routing = user_routing_plan(&env).await;
    let sub = start(&env, routing.version, customer, vec![]).await;
    september_usage(&usage, &routing, 100, 20);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S4a taxed 20%, commission 100 / reversal 20", &invoice);

    let tax_of = |charge: Charge| -> Vec<i64> {
        invoice
            .line_items
            .iter()
            .filter(|line| line.product_id == Some(charge.product))
            .map(|line| line.tax_amount)
            .collect()
    };
    assert_eq!(tax_of(routing.commission), vec![1_000]);
    assert_eq!(
        tax_of(routing.reversal),
        vec![-200],
        "{:?}",
        line_summary(&invoice)
    );
    assert_eq!(totals(&invoice), (4_000, 0, 800, 4_800, 4_800));
}

/// Scenario 2 taxed at 20%: commission 1000 taxed 200, reversal −5000 taxed −1000; tax −800,
/// total −4800, nothing due, 4800 owed as credit.
#[tokio::test]
async fn s04b_a_negative_invoice_carries_negative_tax_and_owes_it_back() {
    let (env, usage) = usage_env().await;
    manual_tax(&env).await;
    let customer = customer_taxed_at_20_percent(&env).await;
    let routing = user_routing_plan(&env).await;
    let sub = start(&env, routing.version, customer, vec![]).await;
    september_usage(&usage, &routing, 20, 100);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S4b taxed 20%, commission 20 / reversal 100", &invoice);

    assert_eq!(totals(&invoice), (-4_000, 0, -800, -4_800, 0));
    assert_eq!(env.invoice_credit_balance_minor(customer).await, 4_800);
}

// ---------------------------------------------------------------------------------------------
// 5. Coupons
// ---------------------------------------------------------------------------------------------

/// 10% off scenario 1's invoice: 10% of what the invoice bills, 4000, is 400 (R-020 applies a
/// percentage to the subtotal); spread over the positive line only (R-070). Total 3600.
#[tokio::test]
async fn s05a_a_percentage_coupon_takes_its_share_of_the_net() {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    let coupon = env.create_percentage_coupon("REVERSAL10", 10).await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![coupon]).await;
    september_usage(&usage, &routing, 100, 20);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S5a 10% coupon, commission 100 / reversal 20", &invoice);

    assert_eq!(totals(&invoice), (4_000, 400, 0, 3_600, 3_600));
}

/// A fixed 60.00 coupon, more than the positive part (5000) and than the net (4000): it takes off
/// what the invoice has left, 4000 (R-020/R-089), never more: total 0, nothing due, no credit.
#[tokio::test]
async fn s05b_a_fixed_coupon_larger_than_the_charges_takes_the_net_at_most() {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    let coupon = env.create_fixed_coupon("REVERSAL60", 60, "EUR").await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![coupon]).await;
    september_usage(&usage, &routing, 100, 20);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show(
        "S5b fixed 60.00 coupon, commission 100 / reversal 20",
        &invoice,
    );

    assert_eq!(totals(&invoice), (4_000, 4_000, 0, 0, 0));
    assert_eq!(
        env.invoice_credit_balance_minor(CUST_UBER_ID).await,
        0,
        "a coupon never turns into credit"
    );
}

// ---------------------------------------------------------------------------------------------
// 6. Minimum commitment
// ---------------------------------------------------------------------------------------------

/// A 50.00 all-components minimum with scenario 1's usage. The minimum is "a floor on what is
/// paid" (R-022): September paid 5000 − 1000 = 4000, so October's invoice, which closes
/// September, trues it up by 1000 over September: lines 5000, −1000, 1000; total 5000. (Counted
/// on the positive part only, it would true up nothing and bill 4000.)
#[tokio::test]
async fn s06_a_minimum_counts_the_spend_net_of_the_reversal() {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    plan_minimum(&env, routing.version, 5_000).await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![]).await;
    september_usage(&usage, &routing, 100, 20);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S6 minimum 50.00, commission 100 / reversal 20", &invoice);

    let shortfall: Vec<(i64, NaiveDate, NaiveDate)> = invoice
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| (line.amount_subtotal, line.start_date, line.end_date))
        .collect();
    assert_eq!(
        shortfall,
        vec![(1_000, day(9, 1), day(10, 1))],
        "{:?}",
        line_summary(&invoice)
    );
    assert_eq!(totals(&invoice), (5_000, 0, 0, 5_000, 5_000));
}

// ---------------------------------------------------------------------------------------------
// 7. Prepaid credit / customer balance
// ---------------------------------------------------------------------------------------------

/// 100.00 of invoice credit before September closes: scenario 1's 4000 is paid from it (R1),
/// nothing due, 6000 left.
#[tokio::test]
async fn s07a_credit_pays_the_net_of_commission_and_reversal() {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![]).await;
    env.fund_invoice_credit(CUST_UBER_ID, 10_000).await;
    september_usage(&usage, &routing, 100, 20);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S7a credit 100.00, commission 100 / reversal 20", &invoice);

    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (4_000, 4_000, 0)
    );
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 6_000);
}

/// 100.00 of credit, then scenario 2's −4000 invoice: it draws nothing and adds 4000 (R7):
/// the balance grows to 14000.
#[tokio::test]
async fn s07b_a_negative_invoice_grows_the_balance() {
    let (env, usage) = usage_env().await;
    let routing = user_routing_plan(&env).await;
    let sub = start(&env, routing.version, CUST_UBER_ID, vec![]).await;
    env.fund_invoice_credit(CUST_UBER_ID, 10_000).await;
    september_usage(&usage, &routing, 20, 100);
    close_september(&env).await;
    let invoice = september_invoice(&env, sub).await;
    show("S7b credit 100.00, commission 20 / reversal 100", &invoice);

    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (-4_000, 0, 0)
    );
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 14_000);
}

// ---------------------------------------------------------------------------------------------
// 8. Finalize and pay
// ---------------------------------------------------------------------------------------------

/// Scenario 1's invoice is issued owing 4000, unpaid; marked paid 40.00, it is paid, nothing due.
#[tokio::test]
async fn s08a_the_net_invoice_is_issued_and_paid() {
    let (env, _routing, _sub, invoice) = bill_routing(100, 20).await;
    show("S8a issued", &invoice);
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.amount_due, 4_000);
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Unpaid);

    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::new(4_000, 2),
            env.now().fixed_offset(),
            None,
        )
        .await
        .expect("marked paid");
    env.run_outbox_and_orchestration().await;
    let paid = reread(&env, &invoice).await;
    show("S8a after marked paid", &paid);
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(paid.amount_due, 0);
}

/// Scenario 2's −4000 invoice is issued with nothing to collect: paid on issue as an invoice of
/// nothing due is (R26), its 4000 owed as credit. A payment of a negative amount is refused.
#[tokio::test]
async fn s08b_a_negative_invoice_is_settled_with_nothing_to_collect() {
    let (env, _routing, _sub, invoice) = bill_routing(20, 100).await;
    show("S8b issued", &invoice);
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!((invoice.total, invoice.amount_due), (-4_000, 0));
    assert_eq!(
        invoice.payment_status,
        InvoicePaymentStatus::Paid,
        "nothing to collect: settled on issue"
    );

    let negative = env
        .services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::new(-4_000, 2),
            env.now().fixed_offset(),
            None,
        )
        .await;
    println!(
        "S8b marking it paid −40.00: {:?}",
        negative.as_ref().map(|d| d.invoice.payment_status)
    );
    assert!(negative.is_err(), "a negative payment is refused");
    let after = reread(&env, &invoice).await;
    show("S8b after the refused payment", &after);
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 4_000);
}

// ---------------------------------------------------------------------------------------------
// 9. Negative tiered, volume and package prices
// ---------------------------------------------------------------------------------------------

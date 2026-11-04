//! MRR on a tax-inclusive plan.
//!
//! A tax-inclusive plan quotes prices that already contain the tax, and MRR is read straight
//! off those prices. Revenue is stated net, so the subscription's MRR — and every figure
//! derived from it: the `Created` event's delta, the BI movement log the analytics projector
//! reads — must have the tax taken back out. A tax-exclusive subscription on the same plan and
//! the same prices must be untouched.

use crate::data::ids::{INVOICING_ENTITY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;
use chrono::NaiveDate;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId, PlanVersionId, SubscriptionId};
use diesel_async::RunQueryDsl;
use diesel_models::enums::{CustomerTaxStatusEnum, MrrMovementType, SubscriptionEventType};
use diesel_models::subscription_events::SubscriptionEventRow;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::{Address, InvoicingEntityPatch};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use rust_decimal_macros::dec;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;

/// LeetCode plan, quoted 35.00 a month.
const QUOTED_MRR: i64 = 3_500;
/// The same 35.00 read as tax-included at the French 20%: 35.00 / 1.2 = 29.1666… → 29.17.
const NET_MRR: i64 = 2_917;

/// The ad-hoc component an amendment adds, quoted 50.00 a month.
const SUPPORT_MRR: i64 = 5_000;
/// LeetCode + Support, quoted 85.00 and read as tax-included: 85.00 / 1.2 = 70.8333… → 70.83.
/// A cent under 29.17 + 41.67, which is the point: the recompute nets the total.
const NET_MRR_WITH_SUPPORT: i64 = 7_083;

/// Starter: Platform Fee 29.00 + one seat at 10.00 = 39.00 a month, netting to 32.50 exactly.
const STARTER_QUOTED_MRR: i64 = 3_900;
const STARTER_NET_MRR: i64 = 3_250;
/// Pro: Platform Fee 99.00 + one seat at 25.00 = 124.00 a month, netting to 103.3333… → 103.33.
const PRO_QUOTED_MRR: i64 = 12_400;
const PRO_NET_MRR: i64 = 10_333;
/// Pro with the seat the upgrade buys: 149.00 a month, netting to 124.1666… → 124.17.
const PRO_QUOTED_MRR_WITH_SEAT: i64 = 14_900;
const PRO_NET_MRR_WITH_SEAT: i64 = 12_417;

const START_DATE: Option<NaiveDate> = NaiveDate::from_ymd_opt(2024, 1, 1);

#[rstest::rstest]
#[case::usage_inclusive(true, true, 10_000)]
#[case::usage_exclusive(true, false, 12_000)]
#[case::exempt_fee_inclusive(false, true, 11_000)]
#[case::exempt_fee_exclusive(false, false, 12_000)]
#[tokio::test]
async fn test_minimum_commitment_mrr_matches_the_taxed_invoice(
    #[case] usage_only: bool,
    #[case] tax_inclusive: bool,
    #[case] expected_mrr: i64,
) {
    use crate::data::ids::{METRIC_BANDWIDTH, PRODUCT_BANDWIDTH_ID, PRODUCT_PLATFORM_FEE_ID};
    use crate::data::plans::{PlanSeed, SeedComp};
    use common_domain::ids::{PlanId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum;
    use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
    use meteroid_store::domain::LineItemKind;
    use meteroid_store::domain::price_components::UsagePricingModel;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let component = if usage_only {
        SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit { rate: dec!(0.1) },
        )
    } else {
        let category = env
            .store()
            .list_tax_categories(TENANT_ID)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.key == "nontaxable")
            .unwrap();
        env.store()
            .update_product(
                common_domain::actor::Actor::System,
                ProductUpdate {
                    id: PRODUCT_PLATFORM_FEE_ID,
                    tenant_id: TENANT_ID,
                    name: None,
                    description: None,
                    fee_type: None,
                    fee_structure: None,
                    tax_category_id: Some(Some(category.id)),
                },
            )
            .await
            .unwrap();
        SeedComp::rate(
            PriceComponentId::new(),
            "Exempt platform fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            dec!(60),
        )
    };
    let version_id = PlanVersionId::new();
    {
        let mut conn = env.conn().await;
        PlanSeed::new(PlanId::new(), "Minimum commitment", version_id)
            .components(vec![component])
            .seed(&mut conn)
            .await
            .unwrap();
        diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
            .values(&PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: version_id,
                amount_subunit: 12_000,
                includes_all_components: true,
            })
            .execute(&mut conn)
            .await
            .unwrap();
    }
    set_version_tax_inclusive(&env, version_id, tax_inclusive).await;
    let sub_id = subscription()
        .customer(create_french_consumer(&env).await)
        .plan_version(version_id)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // The commitment is trued up on the invoice that closes January, beside February's fee.
    assert!(
        !env.get_invoices(sub_id)
            .await
            .iter()
            .flat_map(|i| &i.line_items)
            .any(|l| l.kind == LineItemKind::MinimumShortfall),
        "no period has completed on the first invoice"
    );
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    let invoice = invoices
        .iter()
        .find(|i| {
            i.line_items
                .iter()
                .any(|l| l.kind == LineItemKind::MinimumShortfall)
        })
        .expect("the renewal carries the minimum true-up");
    assert_eq!(invoice.subtotal, expected_mrr);
    assert_eq!(created_mrr_delta(&env, sub_id).await, expected_mrr);
    if usage_only {
        assert_eq!(
            env.get_subscription(sub_id).await.tax_inclusive_rate,
            tax_inclusive.then_some(dec!(0.2)),
        );
    }
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, expected_mrr);
    assert_eq!(movement_log_change(&env, sub_id).await, expected_mrr);
}

#[tokio::test]
async fn test_mrr_is_net_of_the_tax_a_tax_inclusive_plan_quotes() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    // EU VAT resolver on the (French) invoicing entity, so a French customer resolves to the
    // domestic 20%.
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let customer_id = create_french_consumer(&env).await;

    // Control first, while the plan version still quotes net prices.
    let exclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    set_plan_version_tax_inclusive(&env, true).await;

    let inclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // The rate is snapshotted on the subscription, not re-resolved at read time.
    let inclusive = env.get_subscription(inclusive_id).await;
    assert!(inclusive.tax_inclusive, "plan version quotes tax-included");
    assert_eq!(
        inclusive.tax_inclusive_rate,
        Some(dec!(0.2)),
        "the French domestic VAT the quoted prices carry"
    );
    assert_eq!(
        inclusive.mrr_cents, NET_MRR,
        "MRR is the quoted 35.00 with the 20% taken back out"
    );

    let exclusive = env.get_subscription(exclusive_id).await;
    assert!(!exclusive.tax_inclusive);
    assert_eq!(
        exclusive.tax_inclusive_rate, None,
        "nothing is baked into a net price"
    );
    assert_eq!(
        exclusive.mrr_cents, QUOTED_MRR,
        "a tax-exclusive subscription on the same prices is untouched"
    );

    // The Created event's delta and the BI movement it flushed into carry the same net figure:
    // the analytics plane reads MRR from these, not from `mrr_cents`.
    assert_eq!(created_mrr_delta(&env, inclusive_id).await, NET_MRR);
    assert_eq!(created_mrr_delta(&env, exclusive_id).await, QUOTED_MRR);
    assert_eq!(movement_log_change(&env, inclusive_id).await, NET_MRR);
    assert_eq!(movement_log_change(&env, exclusive_id).await, QUOTED_MRR);

    // MRR is tied to the invoice, not merely to a rate that looks right: on a monthly plan
    // the first invoice's net subtotal *is* the monthly recurring revenue.
    assert_eq!(
        first_invoice_subtotal(&env, inclusive_id).await,
        NET_MRR,
        "the invoice carves the same tax out of the same quoted price"
    );
    assert_eq!(first_invoice_subtotal(&env, exclusive_id).await, QUOTED_MRR);

    // M4 (conservation): the churn movement must undo exactly what creation booked, so the
    // movement stream sums to the stored MRR at every point. A double-netted churn would
    // leave the subscription at a positive residual here.
    let cancel_date = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    cancel_at(&env, inclusive_id, cancel_date).await;
    let inclusive = env.get_subscription(inclusive_id).await;
    assert_eq!(
        inclusive.mrr_cents, 0,
        "a cancelled subscription earns nothing"
    );
    assert_eq!(
        churn_mrr_delta(&env, inclusive_id, cancel_date).await,
        -NET_MRR,
        "the churn undoes the net figure creation booked, not the quoted one"
    );
}

/// A customer who bears no tax leaves the quoted price whole — there is nothing to take out
/// of the MRR either, even though the plan quotes tax-included.
#[tokio::test]
async fn test_a_customer_bearing_no_tax_keeps_the_quoted_mrr() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    set_plan_version_tax_inclusive(&env, true).await;

    let exempt_id = subscription()
        .customer(create_french_customer(&env, CustomerTaxStatusEnum::Exempt).await)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let exempt = env.get_subscription(exempt_id).await;
    assert!(exempt.tax_inclusive);
    assert_eq!(
        exempt.tax_inclusive_rate, None,
        "an exempt customer's quoted price carries no tax to carve out"
    );
    assert_eq!(
        exempt.mrr_cents, QUOTED_MRR,
        "so the MRR is the quoted figure, unreduced"
    );
    assert_eq!(created_mrr_delta(&env, exempt_id).await, QUOTED_MRR);
}

/// The `Manual` resolver takes its tax entirely from the product/category configuration —
/// its customer-level answer is always "no tax". The rate MRR is netted by must come from
/// the same line-level resolution the invoice runs, or a Manual-mode tenant's MRR silently
/// stays gross.
#[tokio::test]
async fn test_manual_resolver_nets_mrr_by_the_line_level_tax() {
    use meteroid_store::domain::PaginationRequest;
    use meteroid_store::domain::accounting::{TaxRateNew, TaxRateRule};
    use meteroid_store::repositories::accounting::AccountingInterface;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let digital_services = env
        .store()
        .list_tax_categories(TENANT_ID)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.key == "digital_services")
        .expect("built-in digital_services category is seeded");

    let products = env
        .store()
        .list_products(
            TENANT_ID,
            None,
            false,
            PaginationRequest {
                per_page: Some(100),
                page: 0,
            },
            None,
        )
        .await
        .unwrap();
    for product in products.items {
        env.store()
            .update_product(
                common_domain::actor::Actor::System,
                ProductUpdate {
                    id: product.id,
                    tenant_id: TENANT_ID,
                    name: None,
                    description: None,
                    fee_type: None,
                    fee_structure: None,
                    tax_category_id: Some(Some(digital_services.id)),
                },
            )
            .await
            .unwrap();
    }

    env.store()
        .insert_custom_tax(
            TENANT_ID,
            TaxRateNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Digital services France".to_string(),
                tax_code: "DIGITAL_SERVICES_FR".to_string(),
                tax_category_id: Some(digital_services.id),
                rules: vec![TaxRateRule {
                    country: Some(CountryCode::from_str("FR").unwrap()),
                    region: None,
                    rate: dec!(0.20),
                }],
            },
        )
        .await
        .unwrap();

    set_plan_version_tax_inclusive(&env, true).await;

    let subscription_id = subscription()
        .customer(create_french_customer(&env, CustomerTaxStatusEnum::Taxable).await)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(subscription_id).await;
    assert_eq!(
        sub.tax_inclusive_rate,
        Some(dec!(0.2)),
        "the category's custom tax is what the quoted price carries"
    );
    assert_eq!(sub.mrr_cents, NET_MRR);
    assert_eq!(
        first_invoice_subtotal(&env, subscription_id).await,
        NET_MRR,
        "the invoice nets by the same line-level tax"
    );
}

/// A subscription mixing billing periods *and* tax rates: the one rate it stores is a blend,
/// and the blend has to be weighted by what each component contributes to the MRR — its
/// monthly share — not by the billing-period total its invoice line carries.
///
/// An annual component of 1,200.00 at 20% beside a monthly one of 100.00 at the 5.5% reduced
/// rate both contribute 100.00 a month. Weighted by the period totals (120,000 against
/// 10,000) the annual line's rate would dominate and the stored MRR would come out ~168.44
/// against a true 178.12 — 9.69 a month understated, and inherited by the `Created` event's
/// delta, the BI movement log and every later recompute.
#[tokio::test]
async fn test_mixed_periods_and_rates_blend_over_the_monthly_shares() {
    use crate::data::ids::{PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID};
    use meteroid_store::domain::PaginationRequest;
    use meteroid_store::domain::accounting::{TaxRateNew, TaxRateRule};
    use meteroid_store::repositories::accounting::AccountingInterface;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    // Manual resolver: the rate a line bears comes entirely from its product's tax category,
    // which is what lets the two components resolve differently.
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let categories = env.store().list_tax_categories(TENANT_ID).await.unwrap();
    let category_id = |key: &str| {
        categories
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("built-in {key} category is seeded"))
            .id
    };
    let digital_services = category_id("digital_services");
    let physical_goods = category_id("physical_goods");

    let products = env
        .store()
        .list_products(
            TENANT_ID,
            None,
            false,
            PaginationRequest {
                per_page: Some(100),
                page: 0,
            },
            None,
        )
        .await
        .unwrap();
    for (product_id, category) in [
        (PRODUCT_PLATFORM_FEE_ID, digital_services),
        (PRODUCT_SEATS_ID, physical_goods),
    ] {
        let product = products
            .items
            .iter()
            .find(|p| p.id == product_id)
            .expect("seeded product");
        env.store()
            .update_product(
                common_domain::actor::Actor::System,
                ProductUpdate {
                    id: product.id,
                    tenant_id: TENANT_ID,
                    name: None,
                    description: None,
                    fee_type: None,
                    fee_structure: None,
                    tax_category_id: Some(Some(category)),
                },
            )
            .await
            .unwrap();
    }

    for (name, code, category, rate) in [
        (
            "Digital services France",
            "DIGITAL_SERVICES_FR",
            digital_services,
            dec!(0.20),
        ),
        (
            "Reduced rate France",
            "REDUCED_RATE_FR",
            physical_goods,
            dec!(0.055),
        ),
    ] {
        env.store()
            .insert_custom_tax(
                TENANT_ID,
                TaxRateNew {
                    invoicing_entity_id: INVOICING_ENTITY_ID,
                    name: name.to_string(),
                    tax_code: code.to_string(),
                    tax_category_id: Some(category),
                    rules: vec![TaxRateRule {
                        country: Some(CountryCode::from_str("FR").unwrap()),
                        region: None,
                        rate,
                    }],
                },
            )
            .await
            .unwrap();
    }

    let plan_version_id = seed_mixed_period_plan(&env).await;
    set_version_tax_inclusive(&env, plan_version_id, true).await;

    let subscription_id = subscription()
        .customer(create_french_consumer(&env).await)
        .plan_version(plan_version_id)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // 1200.00/yr at 20% and 100.00/mo at 5.5%: 100.00 + 100.00 gross a month, netting to
    // 83.33 + 94.79.
    let net_by_line = 8_333 + 9_479;
    let sub = env.get_subscription(subscription_id).await;
    assert_eq!(
        sub.mrr_cents, net_by_line,
        "the blended rate must net the summed MRR to the sum of the lines' net, not let the \
         annual line's rate dominate"
    );
    assert_eq!(created_mrr_delta(&env, subscription_id).await, net_by_line);
}

/// Creation is not the only place MRR is read off the fees. Every later recompute — the
/// immediate amendment here, the immediate plan change and slot upgrade below, and the
/// `floor_subscription_mrr` the scheduled paths share — nets the *whole* recomputed figure and
/// differences it against the stored one, which is already net.
///
/// M4 (conservation) is what that has to hold to: the movement booked for a change is
/// `net(new) − net(old)`, never `net(new − old)`, or the movement stream stops summing to
/// `mrr_cents`. Adding a 50.00/mo component to the 35.00/mo tax-inclusive plan moves the MRR by
/// 41.66 — a cent under the 41.67 that component nets to on its own, and 41.66 is the figure
/// that reconciles.
#[tokio::test]
async fn test_an_immediate_amendment_books_the_movement_net_of_the_tax() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let customer_id = create_french_consumer(&env).await;
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // Control first, start to finish, while the plan version still quotes net prices.
    let exclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    add_support_component(&env, exclusive_id, change_date).await;

    let exclusive = env.get_subscription(exclusive_id).await;
    assert_eq!(
        exclusive.mrr_cents,
        QUOTED_MRR + SUPPORT_MRR,
        "a tax-exclusive subscription recomputes on the quoted prices"
    );
    assert_eq!(
        movement_log_change_at(&env, exclusive_id, change_date).await,
        SUPPORT_MRR,
        "and books the quoted figure as its movement"
    );

    set_plan_version_tax_inclusive(&env, true).await;

    let inclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    add_support_component(&env, inclusive_id, change_date).await;

    // (a) The stored MRR is the net of the *new* quoted total — 85.00 with the 20% taken back
    // out — not the old net plus the added component's own net.
    let inclusive = env.get_subscription(inclusive_id).await;
    assert_eq!(
        inclusive.mrr_cents, NET_MRR_WITH_SUPPORT,
        "the recompute nets the whole quoted total the amendment leaves behind"
    );

    // (b) And the movement undoes the difference of the two nets, so creation + amendment still
    // sum to what the subscription stores.
    assert_eq!(
        movement_log_change_at(&env, inclusive_id, change_date).await,
        NET_MRR_WITH_SUPPORT - NET_MRR,
        "the amendment's movement is net(new) - net(old)"
    );
    assert_eq!(
        created_mrr_delta(&env, inclusive_id).await
            + movement_log_change_at(&env, inclusive_id, change_date).await,
        inclusive.mrr_cents,
        "M4: the movement stream sums to the stored MRR"
    );
}

/// An immediate plan change on the day the subscription was created. Its adjustment
/// invoice is dated that day, and used to book the `Created` event a second time, so the MRR read
/// the new plan's plus the old plan's.
#[tokio::test]
async fn test_an_immediate_plan_change_on_the_creation_day_books_the_creation_once() {
    use crate::data::ids::{PLAN_VERSION_PRO_ID, PLAN_VERSION_STARTER_ID};
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::bi_mrr_movement_log::dsl;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
    let customer_id = create_french_consumer(&env).await;
    let start = START_DATE.unwrap();
    let id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(env.get_subscription(id).await.mrr_cents, STARTER_QUOTED_MRR);

    change_plan_immediately(&env, id, PLAN_VERSION_PRO_ID, start).await;
    let invoices = env.get_invoices(id).await;
    assert_eq!(invoices.len(), 2, "the period invoice and the adjustment");
    assert_eq!(
        env.get_subscription(id).await.mrr_cents,
        PRO_QUOTED_MRR,
        "the Pro MRR alone, not Pro plus the Starter creation booked again"
    );

    let mut changes: Vec<i64> = dsl::bi_mrr_movement_log
        .filter(dsl::subscription_id.eq(id))
        .select(dsl::net_mrr_change)
        .load(&mut env.conn().await)
        .await
        .unwrap();
    changes.sort_unstable();
    assert_eq!(
        changes,
        vec![STARTER_QUOTED_MRR, PRO_QUOTED_MRR - STARTER_QUOTED_MRR],
        "the creation once, then the switch; they sum to the stored MRR"
    );
}

/// The other two fee-derived recomputes, on the product-backed Starter/Pro pair: an immediate
/// plan change, then a slot upgrade on the plan it landed on. Both net the whole recomputed
/// figure by the rate the subscription snapshotted, and both difference it against a stored
/// figure that is already net.
///
/// The control has to run to completion first: a plan change refuses to cross tax bases, so the
/// tax-exclusive subscription can only change plans while both versions still quote net.
#[tokio::test]
async fn test_an_immediate_plan_change_and_slot_upgrade_book_their_movements_net() {
    use crate::data::ids::{COMP_PRO_SEATS_ID, PLAN_VERSION_PRO_ID, PLAN_VERSION_STARTER_ID};

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let customer_id = create_french_consumer(&env).await;
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let upgrade_date = NaiveDate::from_ymd_opt(2024, 1, 20).unwrap();

    // --- Control: both versions still quote net. ---
    let exclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(exclusive_id).await.mrr_cents,
        STARTER_QUOTED_MRR
    );

    change_plan_immediately(&env, exclusive_id, PLAN_VERSION_PRO_ID, change_date).await;
    assert_eq!(
        env.get_subscription(exclusive_id).await.mrr_cents,
        PRO_QUOTED_MRR
    );
    assert_eq!(
        movement_log_change_at(&env, exclusive_id, change_date).await,
        PRO_QUOTED_MRR - STARTER_QUOTED_MRR
    );

    add_one_seat(&env, exclusive_id, COMP_PRO_SEATS_ID, upgrade_date).await;
    assert_eq!(
        env.get_subscription(exclusive_id).await.mrr_cents,
        PRO_QUOTED_MRR_WITH_SEAT,
        "a tax-exclusive slot upgrade grows the MRR by the quoted seat"
    );
    assert_eq!(
        movement_log_change_at(&env, exclusive_id, upgrade_date).await,
        PRO_QUOTED_MRR_WITH_SEAT - PRO_QUOTED_MRR
    );

    // --- The same sequence once both versions quote tax-included. ---
    set_version_tax_inclusive(&env, PLAN_VERSION_STARTER_ID, true).await;
    set_version_tax_inclusive(&env, PLAN_VERSION_PRO_ID, true).await;

    let inclusive_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let inclusive = env.get_subscription(inclusive_id).await;
    assert_eq!(inclusive.tax_inclusive_rate, Some(dec!(0.2)));
    assert_eq!(inclusive.mrr_cents, STARTER_NET_MRR);

    change_plan_immediately(&env, inclusive_id, PLAN_VERSION_PRO_ID, change_date).await;
    assert_eq!(
        env.get_subscription(inclusive_id).await.mrr_cents,
        PRO_NET_MRR,
        "the plan change nets the new plan's quoted total by the snapshotted rate"
    );
    assert_eq!(
        movement_log_change_at(&env, inclusive_id, change_date).await,
        PRO_NET_MRR - STARTER_NET_MRR,
        "the expansion is the difference of the two nets"
    );

    add_one_seat(&env, inclusive_id, COMP_PRO_SEATS_ID, upgrade_date).await;
    assert_eq!(
        env.get_subscription(inclusive_id).await.mrr_cents,
        PRO_NET_MRR_WITH_SEAT,
        "so does the slot-aware recompute the upgrade runs"
    );
    assert_eq!(
        movement_log_change_at(&env, inclusive_id, upgrade_date).await,
        PRO_NET_MRR_WITH_SEAT - PRO_NET_MRR
    );

    // M4 across all three movements: creation, plan change, slot upgrade.
    assert_eq!(
        created_mrr_delta(&env, inclusive_id).await
            + movement_log_change_at(&env, inclusive_id, change_date).await
            + movement_log_change_at(&env, inclusive_id, upgrade_date).await,
        PRO_NET_MRR_WITH_SEAT,
        "the movement stream sums to the stored MRR"
    );
}

/// Amend the subscription immediately, adding one ad-hoc 50.00/mo fixed-rate component.
async fn add_support_component(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    change_date: NaiveDate,
) {
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            support_amendment(),
            change_date,
        )
        .await
        .expect("apply_amendment_immediate_at failed");
}

/// Adds a 50.00 monthly Support component, immediately.
fn support_amendment() -> meteroid_store::domain::subscription_amendment::SubscriptionAmendment {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use rust_decimal::Decimal;

    SubscriptionAmendment {
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
                    pricing: Pricing::Rate {
                        rate: Decimal::new(SUPPORT_MRR, 2),
                    },
                    credit_unit: None,
                }),
            }],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

/// Switch plans immediately, mid-period.
async fn change_plan_immediately(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    new_plan_version_id: PlanVersionId,
    change_date: NaiveDate,
) {
    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            new_plan_version_id,
            vec![],
            change_date,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");
}

/// Buy one more seat, billed optimistically — the slot-aware MRR recompute runs immediately.
async fn add_one_seat(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    price_component_id: common_domain::ids::PriceComponentId,
    upgrade_date: NaiveDate,
) {
    use meteroid_store::domain::SlotUpgradeBillingMode;

    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            subscription_id,
            price_component_id,
            1,
            SlotUpgradeBillingMode::Optimistic,
            Some(upgrade_date.and_time(chrono::NaiveTime::MIN)),
        )
        .await
        .expect("slot upgrade failed");
}

/// A plan quoting 1,200.00 a year for the platform fee and 100.00 a month for one seat —
/// the same 100.00 a month each, from two different billing periods.
async fn seed_mixed_period_plan(env: &TestEnv) -> PlanVersionId {
    use crate::data::ids::{PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID};
    use crate::data::plans::{PlanSeed, SeedComp};
    use common_domain::ids::{PlanId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    use rust_decimal::Decimal;

    let version_id = PlanVersionId::new();

    let mut conn = env.conn().await;
    PlanSeed::new(PlanId::new(), "Mixed periods", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                Decimal::new(120_000, 2),
            ),
            SeedComp::slot(
                PriceComponentId::new(),
                "Seats",
                PRODUCT_SEATS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(10_000, 2),
            ),
        ])
        .seed(&mut conn)
        .await
        .expect("seeding the mixed-period plan");

    version_id
}

/// The net subtotal of the subscription's first invoice.
async fn first_invoice_subtotal(env: &TestEnv, subscription_id: SubscriptionId) -> i64 {
    let invoices = env.get_invoices(subscription_id).await;
    invoices
        .into_iter()
        .next()
        .expect("creation issues an invoice")
        .subtotal
}

/// Cancel at `date` and run the lifecycle so the churn is booked.
async fn cancel_at(env: &TestEnv, subscription_id: SubscriptionId, date: NaiveDate) {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            Some("mrr conservation".to_string()),
            CancellationEffectiveAt::Date(date),
        )
        .await
        .expect("cancel_subscription failed");

    // The cancellation is scheduled, then the termination it schedules in turn.
    env.process_cycles().await;
    env.process_cycles().await;
}

/// The `Cancelled` event's MRR delta — the churn movement.
async fn churn_mrr_delta(env: &TestEnv, subscription_id: SubscriptionId, date: NaiveDate) -> i64 {
    let mut conn = env.conn().await;
    SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        &mut conn,
        subscription_id,
        SubscriptionEventType::Cancelled,
        date,
    )
    .await
    .unwrap()
    .expect("cancellation emits a Cancelled event")
    .mrr_delta
    .expect("the Cancelled event carries its MRR delta")
}

/// Flip the seeded plan version between quoting net and quoting tax-included. The store's own
/// patch only reaches draft versions, and the seeded one is published.
async fn set_plan_version_tax_inclusive(env: &TestEnv, tax_inclusive: bool) {
    set_version_tax_inclusive(env, PLAN_VERSION_1_LEETCODE_ID, tax_inclusive).await
}

/// The same, for a plan version the test seeded itself.
async fn set_version_tax_inclusive(
    env: &TestEnv,
    plan_version_id: PlanVersionId,
    tax_inclusive: bool,
) {
    use diesel::ExpressionMethods;
    use diesel_models::schema::plan_version::dsl;

    let mut conn = env.conn().await;
    diesel::update(dsl::plan_version)
        .filter(dsl::id.eq(plan_version_id))
        .filter(dsl::tenant_id.eq(TENANT_ID))
        .set(dsl::tax_inclusive.eq(tax_inclusive))
        .execute(&mut conn)
        .await
        .unwrap();
}

/// The `Created` event's MRR delta — the deferred movement `process_mrr` flushes.
async fn created_mrr_delta(env: &TestEnv, subscription_id: SubscriptionId) -> i64 {
    let mut conn = env.conn().await;
    SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        &mut conn,
        subscription_id,
        SubscriptionEventType::Created,
        START_DATE.unwrap(),
    )
    .await
    .unwrap()
    .expect("creation emits a Created event")
    .mrr_delta
    .expect("the Created event carries its MRR delta")
}

/// The single `bi_mrr_movement_log` row the subscription's creation booked.
async fn movement_log_change(env: &TestEnv, subscription_id: SubscriptionId) -> i64 {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::bi_mrr_movement_log::dsl;

    let mut conn = env.conn().await;
    let changes: Vec<i64> = dsl::bi_mrr_movement_log
        .filter(dsl::subscription_id.eq(subscription_id))
        .filter(dsl::movement_type.eq(MrrMovementType::NewBusiness))
        .select(dsl::net_mrr_change)
        .load(&mut conn)
        .await
        .unwrap();

    assert_eq!(changes.len(), 1, "one movement for the creation");
    changes[0]
}

/// The single `bi_mrr_movement_log` row a post-creation change booked at `date` — the movement
/// carries the change date, while the creation's carries the start date, so the two never mix.
/// A recompute books its figure only here: the `Switch` event it also writes leaves `mrr_delta`
/// null so `process_mrr` never books it a second time.
async fn movement_log_change_at(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    date: NaiveDate,
) -> i64 {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::bi_mrr_movement_log::dsl;

    let mut conn = env.conn().await;
    let changes: Vec<i64> = dsl::bi_mrr_movement_log
        .filter(dsl::subscription_id.eq(subscription_id))
        .filter(dsl::applies_to.eq(date))
        .select(dsl::net_mrr_change)
        .load(&mut conn)
        .await
        .unwrap();

    assert_eq!(changes.len(), 1, "one movement booked at {date}");
    changes[0]
}

/// A French consumer: no VAT number, so the supply is B2C and FR→FR domestic VAT applies.
async fn create_french_consumer(env: &TestEnv) -> CustomerId {
    create_french_customer(env, CustomerTaxStatusEnum::Taxable).await
}

/// The same French consumer, at a chosen tax status — `Exempt` for a customer who bears no
/// tax at all.
async fn create_french_customer(env: &TestEnv, tax_status: CustomerTaxStatusEnum) -> CustomerId {
    use diesel_models::customers::CustomerRowNew;

    let customer_id = CustomerId::new();
    let french_address = Address {
        line1: Some("12 rue de Rivoli".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str("FR").expect("country code")),
        state: Some("Île-de-France".to_string()),
        zip_code: Some("75004".to_string()),
    };

    let mut conn = env.conn().await;
    CustomerRowNew {
        id: customer_id,
        name: "Camille Durand".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&french_address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status,
        exemption_reason: None,
        vat_number_format_valid: false,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Individual,
        first_name: Some("Camille".to_string()),
        last_name: Some("Durand".to_string()),
    }
    .insert(&mut conn)
    .await
    .unwrap();

    customer_id
}

/// A 500.00 currency credit pack, monthly: taxed when its credit is used, never at sale.
const PACK_MRR: i64 = 50_000;

/// The stored tax-inclusive rate describes taxed fees only. Adding a currency credit pack by
/// amendment counts it at its price, and removing it leaves the fees netting exactly as before.
#[tokio::test]
async fn test_a_credit_pack_added_and_removed_never_changes_what_the_fees_net_to() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;

    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    let customer_id = create_french_consumer(&env).await;
    set_plan_version_tax_inclusive(&env, true).await;

    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let created = env.get_subscription(subscription_id).await;
    assert_eq!(created.mrr_cents, NET_MRR);
    let rate = created.tax_inclusive_rate;
    assert_eq!(rate, Some(dec!(0.2)));

    add_credit_pack(
        &env,
        subscription_id,
        NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
    )
    .await;
    let with_pack = env.get_subscription(subscription_id).await;
    assert_eq!(
        with_pack.mrr_cents,
        NET_MRR + PACK_MRR,
        "the pack counts at its price, with no tax carved out of it"
    );
    assert_eq!(with_pack.tax_inclusive_rate, rate);

    let pack = env
        .get_subscription_components(subscription_id)
        .await
        .into_iter()
        .find(|c| c.name == "Credit pack")
        .expect("the pack is a live component");
    remove_components(
        &env,
        subscription_id,
        vec![pack.id],
        NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
    )
    .await;
    let without_pack = env.get_subscription(subscription_id).await;
    assert_eq!(
        without_pack.mrr_cents, NET_MRR,
        "the fee nets to what it did before the pack came and went"
    );
    assert_eq!(without_pack.tax_inclusive_rate, rate);
}

async fn add_credit_pack(env: &TestEnv, subscription_id: SubscriptionId, change_date: NaiveDate) {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use rust_decimal::Decimal;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Credit pack".to_string(),
                        product: ProductRef::New {
                            name: "Credit Pack Product".to_string(),
                            fee_type: FeeTypeEnum::Credit,
                            fee_structure: FeeStructure::Credit {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Credit {
                                unit_price: Decimal::new(PACK_MRR, 2),
                                quantity: 1,
                                credit_amount: Decimal::new(55_000, 2),
                                expiry: None,
                                credit_unit_id: None,
                                rollover: None,
                                once: false,
                            },
                            credit_unit: None,
                        }),
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("adding a credit pack by amendment");
}

async fn remove_components(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    removed: Vec<common_domain::ids::SubscriptionPriceComponentId>,
    change_date: NaiveDate,
) {
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            subscription_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed,
                },
                add_on_changes: AddOnChanges::default(),
            },
            change_date,
        )
        .await
        .expect("removing components by amendment");
}

/// A plan change to a plan that sells a currency pack counts the pack at its price, and the
/// change back leaves the fee netting as before; the rate stays the fee's.
#[tokio::test]
async fn test_a_plan_change_to_and_from_a_plan_with_a_credit_pack_nets_only_the_fees() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    enable_eu_vat(&env).await;
    let customer_id = create_french_consumer(&env).await;
    set_plan_version_tax_inclusive(&env, true).await;
    let packed = seed_inclusive_plan(&env, "Packed", true).await;

    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(subscription_id).await.mrr_cents,
        NET_MRR
    );

    change_plan_immediately(
        &env,
        subscription_id,
        packed,
        NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
    )
    .await;
    let on_packed = env.get_subscription(subscription_id).await;
    assert_eq!(on_packed.mrr_cents, NET_MRR + PACK_MRR);
    assert_eq!(on_packed.tax_inclusive_rate, Some(dec!(0.2)));

    change_plan_immediately(
        &env,
        subscription_id,
        PLAN_VERSION_1_LEETCODE_ID,
        NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
    )
    .await;
    let back = env.get_subscription(subscription_id).await;
    assert_eq!(back.mrr_cents, NET_MRR);
    assert_eq!(back.tax_inclusive_rate, Some(dec!(0.2)));
}

/// A subscription born with nothing taxed (a currency pack alone) stores no rate; the taxed fee
/// an amendment adds re-resolves it, so the fee nets like one the subscription was created with.
#[tokio::test]
async fn test_a_taxed_fee_added_where_none_was_taxed_resolves_the_rate() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    enable_eu_vat(&env).await;
    let customer_id = create_french_consumer(&env).await;
    let pack_only = seed_inclusive_plan(&env, "Pack only", false).await;

    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(pack_only)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let created = env.get_subscription(subscription_id).await;
    assert_eq!(created.mrr_cents, PACK_MRR);
    assert_eq!(
        created.tax_inclusive_rate, None,
        "nothing taxed, nothing to resolve"
    );

    let preview = env
        .services()
        .preview_amendment(subscription_id, TENANT_ID, support_amendment())
        .await
        .expect("preview_amendment");
    add_support_component(
        &env,
        subscription_id,
        NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
    )
    .await;
    let amended = env.get_subscription(subscription_id).await;
    assert_eq!(
        preview.mrr_after_cents, amended.mrr_cents as i64,
        "the preview resolves the rate the apply stores"
    );
    assert_eq!(amended.tax_inclusive_rate, Some(dec!(0.2)));
    // 50.00 quoted with 20% included: 41.666… → 41.67, beside the pack at its price.
    assert_eq!(amended.mrr_cents, 4_167 + PACK_MRR);
}

/// A credit unit pack a tax-inclusive plan sells is taxed with the unit's tax product (10 %), at
/// creation as when MRR is restated: a fee added then removed leaves MRR and the rate as created.
#[tokio::test]
async fn test_a_unit_pack_keeps_its_unit_s_rate_when_mrr_is_restated() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::credit_units::CreditUnitNew;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::repositories::CreditUnitsInterface;
    use meteroid_store::repositories::price_components::PriceComponentInterface;
    use rust_decimal::Decimal;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    enable_eu_vat(&env).await;
    let customer_id = create_french_consumer(&env).await;
    let unit_id = env
        .store()
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "CREDITS", "Credit", "Credits", None, 2).unwrap(),
        )
        .await
        .expect("credit unit")
        .id;
    crate::helpers::credit::reduced_tax_unit_product(env.store(), unit_id).await;
    crate::helpers::plans::as_draft(
        env.store(),
        PLAN_VERSION_1_LEETCODE_ID,
        env.store().create_price_component_from_ref(
            "Credit pack".to_owned(),
            ProductRef::New {
                name: "Credit pack".to_owned(),
                fee_type: FeeTypeEnum::Credit,
                fee_structure: FeeStructure::Credit {},
            },
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: Decimal::new(4_000, 2),
                    quantity: 1,
                    credit_amount: Decimal::new(5_000, 0),
                    expiry: None,
                    credit_unit_id: Some(unit_id),
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            })],
            PLAN_VERSION_1_LEETCODE_ID,
            TENANT_ID,
        ),
    )
    .await
    .expect("a unit pack");
    set_plan_version_tax_inclusive(&env, true).await;

    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(START_DATE.unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let created = env.get_subscription(subscription_id).await;
    let rate = created.tax_inclusive_rate.expect("a blended rate");
    assert!(
        rate > dec!(0.1) && rate < dec!(0.2),
        "the fee at 20 %, the pack at its unit's 10 %: {rate}"
    );

    add_support_component(
        &env,
        subscription_id,
        NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
    )
    .await;
    let support = env
        .get_subscription_components(subscription_id)
        .await
        .into_iter()
        .find(|c| c.name == "Support")
        .expect("the support component");
    remove_components(
        &env,
        subscription_id,
        vec![support.id],
        NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
    )
    .await;
    let restated = env.get_subscription(subscription_id).await;
    assert_eq!(
        (restated.mrr_cents, restated.tax_inclusive_rate),
        (created.mrr_cents, Some(rate))
    );
}

async fn enable_eu_vat(env: &TestEnv) {
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
}

/// A published tax-inclusive EUR plan selling a 500.00 currency pack, beside a 35.00 monthly
/// fee when `with_fee`.
async fn seed_inclusive_plan(env: &TestEnv, name: &str, with_fee: bool) -> PlanVersionId {
    use common_domain::ids::{PlanId, PriceComponentId, ProductId};
    use diesel_models::enums::{FeeTypeEnum, PlanStatusEnum, PlanTypeEnum};
    use diesel_models::plan_versions::PlanVersionRowNew;
    use diesel_models::plans::{PlanRowNew, PlanRowPatch};
    use diesel_models::price_components::PriceComponentRowNew;
    use diesel_models::products::ProductRowNew;
    use meteroid_store::domain::prices::FeeStructure;
    use meteroid_store::domain::{BillingPeriodEnum, FeeType, TermRate};
    use rust_decimal::Decimal;

    let plan_id = PlanId::new();
    let plan_version_id = PlanVersionId::new();
    let mut conn = env.conn().await;
    PlanRowNew {
        id: plan_id,
        name: name.to_string(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
        plan_type: PlanTypeEnum::Standard,
        status: PlanStatusEnum::Active,
    }
    .insert(&mut conn)
    .await
    .unwrap();
    PlanVersionRowNew {
        id: plan_version_id,
        is_draft_version: false,
        plan_id,
        version: 1,
        trial_duration_days: None,
        tenant_id: TENANT_ID,
        period_start_day: None,
        net_terms: 0,
        currency: "EUR".to_string(),
        billing_cycles: None,
        trialing_plan_id: None,
        trial_is_free: false,
        uses_product_pricing: true,
        tax_inclusive: true,
        credit_exhausted_rule: None,
        credit_exhausted_caps: serde_json::json!({}),
    }
    .insert(&mut conn)
    .await
    .unwrap();
    PlanRowPatch {
        id: plan_id,
        tenant_id: TENANT_ID,
        name: None,
        description: None,
        active_version_id: Some(Some(plan_version_id)),
        draft_version_id: None,
        self_service_rank: None,
        issues_license: None,
    }
    .update(&mut conn)
    .await
    .unwrap();

    let mut components = vec![(
        "Credit pack",
        FeeTypeEnum::Credit,
        FeeStructure::Credit {},
        FeeType::Credit {
            unit_price: Decimal::new(PACK_MRR, 2),
            quantity: 1,
            credit_amount: Decimal::new(55_000, 2),
            expiry: None,
            once: false,
        },
    )];
    if with_fee {
        components.push((
            "Subscription Rate",
            FeeTypeEnum::Rate,
            FeeStructure::Rate {},
            FeeType::Rate {
                rates: vec![TermRate {
                    term: BillingPeriodEnum::Monthly,
                    price: Decimal::new(QUOTED_MRR, 2),
                }],
            },
        ));
    }
    for (component_name, fee_type, fee_structure, fee) in components {
        let product_id = ProductId::new();
        ProductRowNew {
            id: product_id,
            name: format!("{name} {component_name}"),
            description: None,
            tenant_id: TENANT_ID,
            product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
            fee_type,
            fee_structure: serde_json::to_value(&fee_structure).unwrap(),
            catalog: true,
            tax_category_id: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
        PriceComponentRowNew {
            id: PriceComponentId::new(),
            name: component_name.to_string(),
            legacy_fee: Some(fee.try_into().unwrap()),
            plan_version_id,
            product_id: Some(product_id),
            billable_metric_id: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }
    plan_version_id
}

/// An immediate upgrade credits the unused plan on a credit note against the invoice that billed
/// it, reversing the tax exactly as it was charged, in both modes; the adjustment invoice only
/// bills the new plan.
#[tokio::test]
async fn an_upgrade_credits_the_unused_plan_with_the_tax_it_bore_in_both_modes() {
    use crate::data::ids::{PLAN_VERSION_PRO_ID, PLAN_VERSION_STARTER_ID};
    use meteroid_store::domain::enums::InvoiceType;
    use meteroid_store::repositories::CreditNoteInterface;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    env.store()
        .patch_invoicing_entity(
            common_domain::actor::Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
    let customer_id = create_french_consumer(&env).await;
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    for tax_inclusive in [false, true] {
        set_version_tax_inclusive(&env, PLAN_VERSION_STARTER_ID, tax_inclusive).await;
        set_version_tax_inclusive(&env, PLAN_VERSION_PRO_ID, tax_inclusive).await;
        let subscription_id = subscription()
            .customer(customer_id)
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(START_DATE.unwrap())
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
        change_plan_immediately(&env, subscription_id, PLAN_VERSION_PRO_ID, change_date).await;
        let invoices = env.get_invoices(subscription_id).await;
        let adjustment = invoices
            .iter()
            .find(|invoice| invoice.invoice_type == InvoiceType::Adjustment)
            .expect("the upgrade's adjustment invoice");
        assert!(
            adjustment
                .line_items
                .iter()
                .all(|line| line.amount_subtotal > 0),
            "the adjustment bills the new plan only, tax_inclusive={tax_inclusive}"
        );
        let original = invoices
            .iter()
            .find(|invoice| invoice.invoice_type == InvoiceType::Recurring)
            .expect("the Starter invoice");
        let notes = env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, original.id)
            .await
            .unwrap();
        assert_eq!(notes.len(), 1, "tax_inclusive={tax_inclusive}");
        for credited in &notes[0].line_items {
            let billed = original
                .line_items
                .iter()
                .find(|line| line.local_id == credited.local_id)
                .expect("the credited line");
            assert!(credited.tax_amount < 0, "tax_inclusive={tax_inclusive}");
            // The unused 16 of 31 days of the net billed, never of the price quoted with tax.
            assert_eq!(
                -credited.amount_subtotal,
                (rust_decimal::Decimal::from(billed.amount_subtotal * 16)
                    / rust_decimal::Decimal::from(31))
                .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                .to_string()
                .parse::<i64>()
                .unwrap(),
                "tax_inclusive={tax_inclusive}"
            );
            assert_eq!(credited.tax_rate, billed.tax_rate);
            // The tax reversed is the billed tax, in the share of the line credited.
            let expected = -(rust_decimal::Decimal::from(billed.tax_amount)
                * rust_decimal::Decimal::from(-credited.amount_subtotal)
                / rust_decimal::Decimal::from(billed.amount_subtotal))
            .round();
            assert_eq!(
                rust_decimal::Decimal::from(credited.tax_amount),
                expected,
                "tax_inclusive={tax_inclusive}"
            );
        }
    }
}

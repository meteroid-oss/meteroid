//! Prices by currency and plan option: a subscription bills the option it buys in its customer's
//! currency, each charge its cell there, whatever cadence each cell bills on; a plan change keeps
//! the currency and the option's alias unless an option is asked; a draft's cells are set and
//! cleared by option and currency.
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::plan_options::{PlanOption, PlanOptionRef};
use meteroid_store::domain::price_cells::{priced_options, sold_options};
use meteroid_store::domain::price_components::{
    PriceComponentNew, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_changes::PlanTarget;
use meteroid_store::domain::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionAddOns,
    CreateSubscriptionComponents, SubscriptionFee, UsagePricingModel,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::PlanOptionInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::price_components::{
    PriceComponentInterface, PriceComponentInterfaceAuto,
};
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice};
use crate::harness::{TestEnv, subscription, test_env};

use BillingPeriodEnum::{Annual, Monthly, Quarterly};

fn day(d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2025, 1, d).unwrap()
}

fn rate(currency: &str, cadence: BillingPeriodEnum, rate: Decimal) -> PriceInput {
    PriceInput {
        cadence,
        currency: currency.to_string(),
        pricing: Pricing::Rate { rate },
        credit_unit: None,
    }
}

fn context(err: &error_stack::Report<StoreError>) -> &StoreError {
    err.current_context()
}

fn refused(err: &error_stack::Report<StoreError>, says: &str) -> bool {
    err.current_context().to_string().contains(says) || format!("{err:?}").contains(says)
}

async fn set_draft(env: &TestEnv, version: PlanVersionId, draft: bool) {
    use diesel_async::RunQueryDsl;
    diesel::sql_query("UPDATE plan_version SET is_draft_version = $2 WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(version.as_uuid())
        .bind::<diesel::sql_types::Bool, _>(draft)
        .execute(&mut env.conn().await)
        .await
        .expect("the version's state");
}

/// A plan whose only version is a draft, without components, sold in its one default option.
async fn draft_version(env: &TestEnv) -> PlanVersionId {
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Pro", version)
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    set_draft(env, version, true).await;
    version
}

/// The options of the draft `version` named `names`: its default renamed after the first, a copy
/// of it for each other; aliases are the names in lowercase.
async fn options(env: &TestEnv, version: PlanVersionId, names: &[&str]) -> Vec<PlanOption> {
    let store = env.store();
    let alias = |name: &str| name.to_ascii_lowercase();
    let default = store
        .list_plan_options(TENANT_ID, version)
        .await
        .expect("the options")
        .remove(0);
    let mut options = vec![
        store
            .update_plan_option(
                TENANT_ID,
                default.id,
                Some(names[0].to_string()),
                Some(alias(names[0])),
                None,
            )
            .await
            .expect("the default renamed"),
    ];
    for name in &names[1..] {
        options.push(
            store
                .create_plan_option(
                    TENANT_ID,
                    version,
                    name.to_string(),
                    Some(alias(name)),
                    None,
                )
                .await
                .expect("an option"),
        );
    }
    options
}

/// A charge `name` on `version`, its `prices` in every option.
async fn charge(
    env: &TestEnv,
    version: PlanVersionId,
    name: &str,
    prices: Vec<PriceInput>,
) -> PriceComponentId {
    env.store()
        .create_price_component_with_prices(
            PriceComponentNew {
                name: name.to_string(),
                product_id: Some(PRODUCT_PLATFORM_FEE_ID),
                plan_version_id: version,
                fee_structure: crate::data::plans::catalog_structure(PRODUCT_PLATFORM_FEE_ID),
            },
            prices,
            TENANT_ID,
        )
        .await
        .expect("a charge")
        .id
}

/// `component`'s cell of `option` in the price's currency: `price` alone.
async fn cell(env: &TestEnv, component: PriceComponentId, option: &PlanOption, price: PriceInput) {
    env.store()
        .set_component_price(TENANT_ID, component, vec![option.id], price)
        .await
        .expect("the cell");
}

/// A published plan of one option whose platform fee is priced by `prices`, in as many
/// currencies as given.
async fn plan(env: &TestEnv, prices: Vec<PriceInput>) -> (PlanVersionId, PriceComponentId) {
    let version = draft_version(env).await;
    let component = charge(env, version, "Platform", prices).await;
    set_draft(env, version, false).await;
    (version, component)
}

/// A published plan sold `names[0]` (its platform fee monthly) or `names[1]` (on `second`), at
/// `(currency, monthly, second)` in each currency.
async fn two_options(
    env: &TestEnv,
    names: [&str; 2],
    second: BillingPeriodEnum,
    prices: &[(&str, Decimal, Decimal)],
) -> (PlanVersionId, PriceComponentId, Vec<PlanOption>) {
    let version = draft_version(env).await;
    let options = options(env, version, &names).await;
    let component = charge(
        env,
        version,
        "Platform",
        prices
            .iter()
            .map(|(currency, monthly, _)| rate(currency, Monthly, *monthly))
            .collect(),
    )
    .await;
    for (currency, _, amount) in prices {
        cell(env, component, &options[1], rate(currency, second, *amount)).await;
    }
    set_draft(env, version, false).await;
    (version, component, options)
}

/// A published plan sold Monthly or Annual.
async fn monthly_or_annual(
    env: &TestEnv,
    prices: &[(&str, Decimal, Decimal)],
) -> (PlanVersionId, PriceComponentId, Vec<PlanOption>) {
    two_options(env, ["Monthly", "Annual"], Annual, prices).await
}

/// `customer`, an EUR one in the fixtures, pays in `currency` from now on.
async fn customer_in(env: &TestEnv, customer: CustomerId, currency: &str) {
    use diesel_async::RunQueryDsl;
    diesel::sql_query("UPDATE customer SET currency = $2 WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(customer.as_uuid())
        .bind::<diesel::sql_types::Text, _>(currency)
        .execute(&mut env.conn().await)
        .await
        .expect("the customer's currency");
}

/// Asks `component` on `cadence`, as integrators chose a term before options.
fn on(component: PriceComponentId, cadence: BillingPeriodEnum) -> CreateSubscriptionComponents {
    CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: component,
            parameters: ComponentParameters {
                initial_slot_count: None,
                billing_period: Some(cadence),
                committed_capacity: None,
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    }
}

fn alias(alias: &str) -> Option<PlanOptionRef> {
    Some(PlanOptionRef::Alias(alias.to_string()))
}

async fn subscribe(
    env: &TestEnv,
    customer: CustomerId,
    version: PlanVersionId,
    option: Option<PlanOptionRef>,
    components: Option<CreateSubscriptionComponents>,
    add_ons: Option<CreateSubscriptionAddOns>,
) -> meteroid_store::StoreResult<SubscriptionId> {
    let mut request = subscription()
        .customer(customer)
        .plan_version(version)
        .start_date(day(1))
        .on_start()
        .no_trial()
        .request();
    request.option = option;
    request.price_components = components;
    request.add_ons = add_ons;
    env.services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .map(|created| created.id)
}

/// The charge `name` of `sub`: its rate and period.
async fn fee_of(
    env: &TestEnv,
    sub: SubscriptionId,
    name: &str,
) -> (Decimal, SubscriptionFeeBillingPeriod) {
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details");
    let component = details
        .price_components
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("the charge {name}"));
    match &component.fee {
        SubscriptionFee::Rate { rate } => (*rate, component.period),
        fee => panic!("a rate, got {fee:?}"),
    }
}

async fn currency_of(env: &TestEnv, sub: SubscriptionId) -> String {
    env.store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details")
        .subscription
        .currency
}

/// The alias of the option `sub` bills.
async fn option_of(env: &TestEnv, sub: SubscriptionId) -> Option<String> {
    env.store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details")
        .plan_option
        .map(|option| option.alias)
}

async fn change(
    env: &TestEnv,
    sub: SubscriptionId,
    target: impl Into<PlanTarget>,
) -> meteroid_store::StoreResult<()> {
    env.services()
        .apply_plan_change_immediate_at(Actor::System, sub, TENANT_ID, target, vec![], day(15))
        .await
        .map(|_| ())
}

/// One plan version sells in EUR and USD: each customer is billed in their own currency, in the
/// option asked by alias, else the default.
#[tokio::test]
async fn a_subscription_bills_its_option_in_its_customers_currency() {
    let env = test_env().await;
    let (version, _, _) = monthly_or_annual(
        &env,
        &[("EUR", dec!(20), dec!(200)), ("USD", dec!(22), dec!(220))],
    )
    .await;
    customer_in(&env, CUST_SPOTIFY_ID, "USD").await;

    let eur = subscribe(&env, CUST_UBER_ID, version, alias("annual"), None, None)
        .await
        .expect("in EUR");
    assert_eq!(currency_of(&env, eur).await, "EUR");
    assert_eq!(
        fee_of(&env, eur, "Platform").await,
        (dec!(200), SubscriptionFeeBillingPeriod::Annual)
    );
    assert_eq!(option_of(&env, eur).await.as_deref(), Some("annual"));

    let usd = subscribe(&env, CUST_SPOTIFY_ID, version, None, None, None)
        .await
        .expect("in USD");
    assert_eq!(currency_of(&env, usd).await, "USD");
    assert_eq!(
        fee_of(&env, usd, "Platform").await,
        (dec!(22), SubscriptionFeeBillingPeriod::Monthly),
        "the default option"
    );
    assert_eq!(option_of(&env, usd).await.as_deref(), Some("monthly"));
}

/// An alias the plan does not have is refused, never read as the default.
#[tokio::test]
async fn an_unknown_option_is_refused() {
    let env = test_env().await;
    let (version, _, _) = monthly_or_annual(&env, &[("EUR", dec!(20), dec!(200))]).await;
    let err = subscribe(&env, CUST_UBER_ID, version, alias("weekly"), None, None)
        .await
        .expect_err("no such option");
    assert!(
        matches!(context(&err), StoreError::InvalidArgument(_)) && refused(&err, "weekly"),
        "{err:?}"
    );
}

/// A request naming no option but the cadence of a charge, as before options, buys the option
/// whose cell bills on it; a cadence no option bills on is refused, naming the charge.
#[tokio::test]
async fn a_cadence_asked_buys_the_option_billing_on_it() {
    let env = test_env().await;
    let (version, component, _) = monthly_or_annual(&env, &[("EUR", dec!(20), dec!(200))]).await;

    let sub = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        None,
        Some(on(component, Annual)),
        None,
    )
    .await
    .expect("annual");
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("annual"));
    assert_eq!(
        fee_of(&env, sub, "Platform").await,
        (dec!(200), SubscriptionFeeBillingPeriod::Annual)
    );

    let err = subscribe(
        &env,
        CUST_SPOTIFY_ID,
        version,
        None,
        Some(on(component, Quarterly)),
        None,
    )
    .await
    .expect_err("no option bills quarterly");
    assert!(
        matches!(context(&err), StoreError::InvalidArgument(_)) && refused(&err, "Platform"),
        "{err:?}"
    );
}

/// A customer whose currency the option is not sold in is billed in the plan's (EUR here), else
/// the first currency the option is sold in.
#[tokio::test]
async fn an_option_not_sold_in_the_customers_currency_falls_back() {
    let env = test_env().await;
    customer_in(&env, CUST_SPOTIFY_ID, "GBP").await;
    let (both, _) = plan(
        &env,
        vec![
            rate("EUR", Monthly, dec!(20)),
            rate("USD", Monthly, dec!(22)),
        ],
    )
    .await;
    let sub = subscribe(&env, CUST_SPOTIFY_ID, both, None, None, None)
        .await
        .expect("in the plan's currency");
    assert_eq!(currency_of(&env, sub).await, "EUR");
    assert_eq!(fee_of(&env, sub, "Platform").await.0, dec!(20));

    let (usd_only, _) = plan(&env, vec![rate("USD", Monthly, dec!(30))]).await;
    let sub = subscribe(&env, CUST_UBER_ID, usd_only, None, None, None)
        .await
        .expect("in the one currency it is sold in");
    assert_eq!(currency_of(&env, sub).await, "USD");
    assert_eq!(fee_of(&env, sub, "Platform").await.0, dec!(30));
}

/// An option bills each charge on its own cell's cadence: Annual bills the platform fee yearly
/// and support quarterly; Monthly bills both monthly.
#[tokio::test]
async fn an_option_mixes_cadences() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let options = options(&env, version, &["Monthly", "Annual"]).await;
    let platform = charge(
        &env,
        version,
        "Platform",
        vec![rate("EUR", Monthly, dec!(20))],
    )
    .await;
    cell(&env, platform, &options[1], rate("EUR", Annual, dec!(200))).await;
    let support = charge(
        &env,
        version,
        "Support",
        vec![rate("EUR", Monthly, dec!(5))],
    )
    .await;
    cell(&env, support, &options[1], rate("EUR", Quarterly, dec!(14))).await;
    set_draft(&env, version, false).await;

    let components = env
        .store()
        .list_price_components(version, TENANT_ID)
        .await
        .expect("components");
    assert_eq!(sold_options(&options, &components, "EUR").len(), 2);
    assert_eq!(
        priced_options(&options, &components),
        vec![
            ("EUR".to_string(), options[0].id),
            ("EUR".to_string(), options[1].id)
        ]
    );

    let annual = subscribe(&env, CUST_UBER_ID, version, alias("annual"), None, None)
        .await
        .expect("annual");
    assert_eq!(
        fee_of(&env, annual, "Platform").await,
        (dec!(200), SubscriptionFeeBillingPeriod::Annual)
    );
    assert_eq!(
        fee_of(&env, annual, "Support").await,
        (dec!(14), SubscriptionFeeBillingPeriod::Quarterly)
    );
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, annual)
        .await
        .expect("details");
    assert_eq!(
        details.subscription.period, Quarterly,
        "it renews on its shortest fee's period"
    );

    let monthly = subscribe(&env, CUST_SPOTIFY_ID, version, None, None, None)
        .await
        .expect("monthly");
    assert_eq!(
        fee_of(&env, monthly, "Platform").await,
        (dec!(20), SubscriptionFeeBillingPeriod::Monthly)
    );
    assert_eq!(
        fee_of(&env, monthly, "Support").await,
        (dec!(5), SubscriptionFeeBillingPeriod::Monthly)
    );
}

/// A plan change keeps the currency and the option's alias: Annual moves to the target's Annual
/// in USD; a target without that alias takes its default; one not sold in the subscription's
/// currency is refused.
#[tokio::test]
async fn a_plan_change_keeps_the_currency_and_the_option() {
    let env = test_env().await;
    customer_in(&env, CUST_SPOTIFY_ID, "USD").await;
    let (from, _, _) = monthly_or_annual(&env, &[("USD", dec!(20), dec!(200))]).await;
    let (to, _, _) = monthly_or_annual(
        &env,
        &[("EUR", dec!(38), dec!(380)), ("USD", dec!(40), dec!(400))],
    )
    .await;
    let (no_annual, _, _) = two_options(
        &env,
        ["Monthly", "Quarterly"],
        Quarterly,
        &[("USD", dec!(50), dec!(140))],
    )
    .await;
    let (eur_only, _) = plan(&env, vec![rate("EUR", Annual, dec!(500))]).await;

    let yearly = subscribe(&env, CUST_SPOTIFY_ID, from, alias("annual"), None, None)
        .await
        .expect("yearly in USD");
    let err = change(&env, yearly, eur_only)
        .await
        .expect_err("not priced in USD");
    assert!(refused(&err, "Platform has no price in USD"), "{err:?}");

    change(&env, yearly, to)
        .await
        .expect("annual to the target's annual");
    assert_eq!(currency_of(&env, yearly).await, "USD");
    assert_eq!(
        fee_of(&env, yearly, "Platform").await,
        (dec!(400), SubscriptionFeeBillingPeriod::Annual),
        "the target's USD annual price, never its EUR one"
    );
    assert_eq!(option_of(&env, yearly).await.as_deref(), Some("annual"));

    let other = subscribe(&env, CUST_SPOTIFY_ID, from, alias("annual"), None, None)
        .await
        .expect("yearly in USD");
    change(&env, other, no_annual)
        .await
        .expect("to the target's default");
    assert_eq!(
        fee_of(&env, other, "Platform").await,
        (dec!(50), SubscriptionFeeBillingPeriod::Monthly)
    );
    assert_eq!(option_of(&env, other).await.as_deref(), Some("monthly"));
}

/// `component` made a pre-currency charge: one monthly `rate` in its version's currency.
async fn to_legacy(env: &TestEnv, component: PriceComponentId, rate: Decimal) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::{FeeType, TermRate};
    let fee = serde_json::to_string(&FeeType::Rate {
        rates: vec![TermRate {
            term: Monthly,
            price: rate,
        }],
    })
    .expect("a legacy fee");
    let mut conn = env.conn().await;
    diesel::sql_query("DELETE FROM plan_component_price WHERE plan_component_id = $1")
        .bind::<diesel::sql_types::Uuid, _>(component.as_uuid())
        .execute(&mut conn)
        .await
        .expect("its prices dropped");
    diesel::sql_query("UPDATE price_component SET legacy_fee = $2::jsonb WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(component.as_uuid())
        .bind::<diesel::sql_types::Text, _>(fee)
        .execute(&mut conn)
        .await
        .expect("its legacy fee");
}

/// A pre-currency charge has one amount, in its version's currency (EUR): a USD subscription is
/// refused the move, never billed €49 as $49; an EUR one moves.
#[tokio::test]
async fn a_plan_change_to_a_legacy_plan_in_another_currency_is_refused() {
    let env = test_env().await;
    customer_in(&env, CUST_SPOTIFY_ID, "USD").await;
    let (from, _) = plan(
        &env,
        vec![
            rate("EUR", Monthly, dec!(20)),
            rate("USD", Monthly, dec!(22)),
        ],
    )
    .await;
    let (legacy, legacy_component) = plan(&env, vec![rate("EUR", Monthly, dec!(49))]).await;
    to_legacy(&env, legacy_component, dec!(49)).await;

    let usd = subscribe(&env, CUST_SPOTIFY_ID, from, None, None, None)
        .await
        .expect("in USD");
    assert_eq!(currency_of(&env, usd).await, "USD");
    let err = change(&env, usd, legacy)
        .await
        .expect_err("a EUR legacy price on a USD subscription");
    assert!(
        matches!(context(&err), StoreError::InvalidArgument(_))
            && refused(&err, "Platform is priced in EUR only"),
        "{err:?}"
    );
    assert_eq!(fee_of(&env, usd, "Platform").await.0, dec!(22), "unchanged");

    let eur = subscribe(&env, CUST_UBER_ID, from, None, None, None)
        .await
        .expect("in EUR");
    change(&env, eur, legacy).await.expect("EUR to EUR");
    assert_eq!(fee_of(&env, eur, "Platform").await.0, dec!(49));
}

/// A move to another option of the plan it is on: the same version, the option asked; the same
/// option is no change.
#[tokio::test]
async fn an_option_asked_moves_the_subscription_to_it() {
    let env = test_env().await;
    let (version, _, options) = monthly_or_annual(&env, &[("EUR", dec!(20), dec!(200))]).await;
    let sub = subscribe(&env, CUST_UBER_ID, version, None, None, None)
        .await
        .expect("monthly");

    assert!(
        change(&env, sub, version).await.is_err(),
        "the plan and option it is on"
    );
    change(
        &env,
        sub,
        PlanTarget {
            version,
            option: Some(PlanOptionRef::Id(options[1].id)),
        },
    )
    .await
    .expect("a switch to annual");
    assert_eq!(
        fee_of(&env, sub, "Platform").await,
        (dec!(200), SubscriptionFeeBillingPeriod::Annual)
    );
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("annual"));
}

/// A platform fee on each of `prices`, beside monthly bandwidth usage, in EUR, in one option.
async fn fee_with_usage(
    env: &TestEnv,
    prices: Vec<SeedPrice>,
) -> (PlanVersionId, PriceComponentId) {
    let version = PlanVersionId::new();
    let platform_id = PriceComponentId::new();
    let mut platform = SeedComp::rate(
        platform_id,
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        prices[0].id,
        prices[0].cadence.clone(),
        dec!(1),
    );
    platform.prices = prices;
    let usage = SeedComp::usage(
        PriceComponentId::new(),
        "Bandwidth",
        PRODUCT_BANDWIDTH_ID,
        METRIC_BANDWIDTH,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        UsagePricingModel::PerUnit { rate: dec!(0.10) },
    );
    PlanSeed::new(PlanId::new(), "Usage", version)
        .components(vec![platform, usage])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    (version, platform_id)
}

fn seed_price(cadence: DieselBillingPeriodEnum, rate: Decimal) -> SeedPrice {
    SeedPrice {
        id: PriceId::new(),
        cadence,
        pricing: Pricing::Rate { rate },
    }
}

/// A change naming no option keeps its alias over the target's default: an annual fee beside
/// monthly usage moves to the target's option of the same alias, never its first one.
#[tokio::test]
async fn a_plan_change_keeps_the_alias_over_the_default() {
    let env = test_env().await;
    let (from, _) = fee_with_usage(
        &env,
        vec![seed_price(DieselBillingPeriodEnum::Annual, dec!(1200))],
    )
    .await;
    let (to, platform) = fee_with_usage(
        &env,
        vec![seed_price(DieselBillingPeriodEnum::Annual, dec!(1500))],
    )
    .await;
    // The target ranks a Monthly option before the one named like the subscription's.
    set_draft(&env, to, true).await;
    let standard = env
        .store()
        .list_plan_options(TENANT_ID, to)
        .await
        .expect("options")
        .remove(0);
    let monthly = env
        .store()
        .create_plan_option(
            TENANT_ID,
            to,
            "Monthly".to_string(),
            Some("monthly".to_string()),
            None,
        )
        .await
        .expect("an option");
    cell(&env, platform, &monthly, rate("EUR", Monthly, dec!(150))).await;
    env.store()
        .reorder_plan_options(TENANT_ID, to, vec![monthly.id, standard.id])
        .await
        .expect("monthly first");
    set_draft(&env, to, false).await;

    let sub = subscribe(&env, CUST_UBER_ID, from, None, None, None)
        .await
        .expect("an annual fee with monthly usage");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details");
    assert_eq!(details.subscription.period, Monthly);
    assert_eq!(
        fee_of(&env, sub, "Platform").await,
        (dec!(1200), SubscriptionFeeBillingPeriod::Annual)
    );

    change(&env, sub, to).await.expect("the upgrade");
    assert_eq!(
        fee_of(&env, sub, "Platform").await,
        (dec!(1500), SubscriptionFeeBillingPeriod::Annual),
        "the target's standard option, not 150.00 a month"
    );
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("standard"));
}

/// A 100.00 monthly add-on, priced in `currency`.
async fn add_on_in(env: &TestEnv, currency: &str) -> AddOnId {
    env.store()
        .create_add_on_from_ref(
            "Priority support".to_string(),
            ProductRef::New {
                name: "Priority support".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(rate(currency, Monthly, dec!(100))),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on")
        .id
}

/// An add-on priced in another currency than the customer's is refused.
#[tokio::test]
async fn an_add_on_in_another_currency_is_refused() {
    let env = test_env().await;
    let (version, component) = plan(&env, vec![rate("EUR", Monthly, dec!(20))]).await;
    let with = |add_on| {
        Some(CreateSubscriptionAddOns {
            add_ons: vec![CreateSubscriptionAddOn {
                add_on_id: add_on,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
        })
    };
    // A cadence asked of a plan of one option buys it, as before options.
    let monthly = || Some(on(component, Monthly));

    let in_usd = add_on_in(&env, "USD").await;
    let err = subscribe(&env, CUST_UBER_ID, version, None, monthly(), with(in_usd))
        .await
        .expect_err("not priced in EUR");
    assert!(
        refused(&err, "Priority support has no price in EUR")
            || refused(&err, "Priority support is priced in USD"),
        "{err:?}"
    );

    let in_eur = add_on_in(&env, "EUR").await;
    let sub = subscribe(
        &env,
        CUST_SPOTIFY_ID,
        version,
        None,
        monthly(),
        with(in_eur),
    )
    .await
    .expect("priced in EUR");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details");
    assert_eq!(details.add_ons.len(), 1);
}

async fn prices_of(
    env: &TestEnv,
    version: PlanVersionId,
    component: PriceComponentId,
) -> Vec<meteroid_store::domain::Price> {
    env.store()
        .list_price_components(version, TENANT_ID)
        .await
        .expect("the components")
        .into_iter()
        .find(|c| c.id == component)
        .expect("the component")
        .prices
}

fn rate_of(price: &meteroid_store::domain::Price) -> Option<Decimal> {
    match price.pricing {
        Pricing::Rate { rate } => Some(rate),
        _ => None,
    }
}

/// A cell is one option in one currency: setting it again replaces its price, the other
/// option's and currency's cells stay; a price set in every option is one price; a price of an
/// unknown currency is refused.
#[tokio::test]
async fn a_cell_is_set_by_option_and_currency() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let options = options(&env, version, &["Monthly", "Annual"]).await;
    let fee = charge(
        &env,
        version,
        "Platform",
        vec![rate("USD", Monthly, dec!(30))],
    )
    .await;
    cell(&env, fee, &options[1], rate("USD", Annual, dec!(300))).await;

    for amount in [dec!(35), dec!(40)] {
        cell(&env, fee, &options[0], rate("USD", Monthly, amount)).await;
    }
    let euros = env
        .store()
        .set_component_price(TENANT_ID, fee, vec![], rate("EUR", Monthly, dec!(28)))
        .await
        .expect("the EUR cells set");
    assert_eq!(
        euros.prices.len(),
        3,
        "40 and 300 in USD, 28 in EUR in both options"
    );

    let component = env
        .store()
        .list_price_components(version, TENANT_ID)
        .await
        .expect("the components")
        .into_iter()
        .find(|c| c.id == fee)
        .expect("the component");
    let in_cell = |option: &PlanOption, currency: &str| {
        meteroid_store::domain::price_cells::option_cell(&component, option.id, currency)
            .into_iter()
            .filter_map(rate_of)
            .collect::<Vec<_>>()
    };
    assert_eq!(in_cell(&options[0], "USD"), vec![dec!(40)]);
    assert_eq!(in_cell(&options[1], "USD"), vec![dec!(300)]);
    assert_eq!(in_cell(&options[0], "EUR"), vec![dec!(28)]);
    assert_eq!(in_cell(&options[1], "EUR"), vec![dec!(28)]);

    let err = env
        .store()
        .set_component_price(TENANT_ID, fee, vec![], rate("XYZ", Monthly, dec!(1)))
        .await
        .expect_err("an unknown currency");
    assert!(matches!(context(&err), StoreError::InvalidArgument(_)));
}

/// Clearing a currency's cells leaves the others; the charge's last price is refused.
#[tokio::test]
async fn a_cell_is_cleared_by_currency() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let fee = charge(
        &env,
        version,
        "Platform",
        vec![
            rate("EUR", Monthly, dec!(20)),
            rate("USD", Monthly, dec!(22)),
        ],
    )
    .await;

    let cleared = env
        .store()
        .clear_component_price(TENANT_ID, fee, vec![], "USD".to_string())
        .await
        .expect("the USD cell cleared");
    assert_eq!(cleared.prices.len(), 1);
    assert_eq!(cleared.prices[0].currency, "EUR");

    let last = env
        .store()
        .clear_component_price(TENANT_ID, fee, vec![], "EUR".to_string())
        .await
        .expect_err("the charge's last price cleared");
    assert!(matches!(context(&last), StoreError::FailedPrecondition(_)));
}

/// A published version's cells never change.
#[tokio::test]
async fn a_published_version_cell_is_refused() {
    let env = test_env().await;
    let (version, fee) = plan(
        &env,
        vec![
            rate("USD", Monthly, dec!(30)),
            rate("USD", Annual, dec!(300)),
        ],
    )
    .await;
    let set = env
        .store()
        .set_component_price(TENANT_ID, fee, vec![], rate("USD", Monthly, dec!(35)))
        .await
        .expect_err("a published cell set");
    assert!(matches!(context(&set), StoreError::FailedPrecondition(_)));
    let clear = env
        .store()
        .clear_component_price(TENANT_ID, fee, vec![], "USD".to_string())
        .await
        .expect_err("a published cell cleared");
    assert!(matches!(context(&clear), StoreError::FailedPrecondition(_)));
    assert_eq!(prices_of(&env, version, fee).await.len(), 2);
}

/// Editing a draft's prices in one currency keeps those of another it names to keep.
#[tokio::test]
async fn editing_one_currency_keeps_the_others() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let component = charge(
        &env,
        version,
        "Platform",
        vec![
            rate("EUR", Monthly, dec!(20)),
            rate("USD", Monthly, dec!(22)),
        ],
    )
    .await;
    let usd = prices_of(&env, version, component)
        .await
        .into_iter()
        .find(|p| p.currency == "USD")
        .expect("the USD price")
        .id;
    let current = env
        .store()
        .get_price_component_by_id(TENANT_ID, component)
        .await
        .expect("the component");
    let edited = env
        .store()
        .update_price_component_with_prices(
            current,
            vec![rate("EUR", Monthly, dec!(21))],
            vec![usd],
            TENANT_ID,
            version,
        )
        .await
        .expect("the EUR price edited");
    assert_eq!(edited.prices.len(), 2);
    assert!(edited.prices.iter().any(|price| price.id == usd));
    assert!(
        edited
            .prices
            .iter()
            .any(|p| p.currency == "EUR" && rate_of(p) == Some(dec!(21)))
    );
}

/// A capacity cell holds a level per commitment: setting one replaces that level only, a new
/// commitment adds one.
#[tokio::test]
async fn editing_one_capacity_level_keeps_the_others() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let capacity = |rate: Decimal, included: u64| PriceInput {
        cadence: Monthly,
        currency: "USD".to_string(),
        pricing: Pricing::Capacity {
            rate,
            included,
            overage_rate: dec!(0.10),
        },
        credit_unit: None,
    };
    let fee = env
        .store()
        .create_price_component_with_prices(
            PriceComponentNew {
                name: "Bandwidth".to_string(),
                product_id: Some(PRODUCT_CAPACITY_ID),
                plan_version_id: version,
                fee_structure: crate::data::plans::catalog_structure(PRODUCT_CAPACITY_ID),
            },
            vec![capacity(dec!(99), 10_000), capacity(dec!(499), 100_000)],
            TENANT_ID,
        )
        .await
        .expect("a capacity charge")
        .id;
    let levels = |prices: Vec<meteroid_store::domain::Price>| {
        let mut levels: Vec<(u64, Decimal)> = prices
            .iter()
            .filter_map(|p| match p.pricing {
                Pricing::Capacity { included, rate, .. } => Some((included, rate)),
                _ => None,
            })
            .collect();
        levels.sort();
        levels
    };

    env.store()
        .set_component_price(TENANT_ID, fee, vec![], capacity(dec!(89), 10_000))
        .await
        .expect("a level set");
    assert_eq!(
        levels(prices_of(&env, version, fee).await),
        vec![(10_000, dec!(89)), (100_000, dec!(499))]
    );

    env.store()
        .set_component_price(TENANT_ID, fee, vec![], capacity(dec!(1999), 1_000_000))
        .await
        .expect("a level added");
    assert_eq!(
        levels(prices_of(&env, version, fee).await),
        vec![
            (10_000, dec!(89)),
            (100_000, dec!(499)),
            (1_000_000, dec!(1999))
        ]
    );
}

/// The plan editor writes a capacity cell whole: the levels sent are the cell, those left out
/// leave it, a level whose commitment changed replaces the old one instead of adding to it.
#[tokio::test]
async fn setting_a_capacity_cell_writes_every_level() {
    let env = test_env().await;
    let version = draft_version(&env).await;
    let capacity = |rate: Decimal, included: u64, overage_rate: Decimal| PriceInput {
        cadence: Monthly,
        currency: "EUR".to_string(),
        pricing: Pricing::Capacity {
            rate,
            included,
            overage_rate,
        },
        credit_unit: None,
    };
    let fee = env
        .store()
        .create_price_component_with_prices(
            PriceComponentNew {
                name: "Volume".to_string(),
                product_id: Some(PRODUCT_CAPACITY_ID),
                plan_version_id: version,
                fee_structure: crate::data::plans::catalog_structure(PRODUCT_CAPACITY_ID),
            },
            vec![
                capacity(dec!(479), 50_000, dec!(0.005)),
                capacity(dec!(899), 125_000, dec!(0.005)),
                capacity(dec!(1499), 250_000, dec!(0.005)),
            ],
            TENANT_ID,
        )
        .await
        .expect("a capacity charge")
        .id;
    let levels = |prices: Vec<meteroid_store::domain::Price>| {
        let mut levels: Vec<(u64, Decimal, Decimal)> = prices
            .iter()
            .filter_map(|p| match p.pricing {
                Pricing::Capacity {
                    included,
                    rate,
                    overage_rate,
                } => Some((included, rate, overage_rate)),
                _ => None,
            })
            .collect();
        levels.sort();
        levels
    };

    // One level removed, one moved to another commitment, one overage of its own.
    env.store()
        .set_component_prices(
            TENANT_ID,
            fee,
            vec![],
            vec![
                capacity(dec!(479), 50_000, dec!(0.005)),
                capacity(dec!(999), 150_000, dec!(0.004)),
            ],
        )
        .await
        .expect("the cell set");
    assert_eq!(
        levels(prices_of(&env, version, fee).await),
        vec![
            (50_000, dec!(479), dec!(0.005)),
            (150_000, dec!(999), dec!(0.004))
        ]
    );

    let refused_with = |says: &'static str| {
        move |err: error_stack::Report<StoreError>| assert!(refused(&err, says), "{err:?}")
    };
    env.store()
        .set_component_prices(
            TENANT_ID,
            fee,
            vec![],
            vec![
                capacity(dec!(479), 50_000, dec!(0.005)),
                capacity(dec!(499), 50_000, dec!(0.005)),
            ],
        )
        .await
        .map(|_| panic!("two levels of one commitment"))
        .unwrap_or_else(refused_with("same amount"));
    let mut in_usd = capacity(dec!(999), 150_000, dec!(0.004));
    in_usd.currency = "USD".to_string();
    env.store()
        .set_component_prices(
            TENANT_ID,
            fee,
            vec![],
            vec![capacity(dec!(479), 50_000, dec!(0.005)), in_usd],
        )
        .await
        .map(|_| panic!("levels in two currencies"))
        .unwrap_or_else(refused_with("one currency"));
    env.store()
        .set_component_prices(
            TENANT_ID,
            fee,
            vec![],
            vec![
                capacity(dec!(479), 50_000, dec!(0.005)),
                rate("EUR", Monthly, dec!(10)),
            ],
        )
        .await
        .map(|_| panic!("a rate among levels"))
        .unwrap_or_else(refused_with("levels share a cell"));
    // Refused writes leave the cell as it was.
    assert_eq!(levels(prices_of(&env, version, fee).await).len(), 2);
}

/// Another option on the cadence it is on, at once: its price and its alias.
#[tokio::test]
async fn an_option_on_the_same_cadence_applies_at_once() {
    let env = test_env().await;
    let (version, _, options) = two_options(
        &env,
        ["Standard", "Startup"],
        Monthly,
        &[("EUR", dec!(20), dec!(15))],
    )
    .await;
    let sub = subscribe(&env, CUST_UBER_ID, version, None, None, None)
        .await
        .expect("standard");
    change(
        &env,
        sub,
        PlanTarget {
            version,
            option: Some(PlanOptionRef::Id(options[1].id)),
        },
    )
    .await
    .expect("a switch to startup");
    assert_eq!(
        fee_of(&env, sub, "Platform").await,
        (dec!(15), SubscriptionFeeBillingPeriod::Monthly)
    );
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("startup"));
}

/// Another option on the cadence it is on, at the period end: the old price until then, the
/// new one and its alias after.
#[tokio::test]
async fn an_option_on_the_same_cadence_applies_at_the_period_end() {
    let env = test_env().await;
    let (version, _, options) = two_options(
        &env,
        ["Standard", "Startup"],
        Monthly,
        &[("EUR", dec!(20), dec!(15))],
    )
    .await;
    let sub = subscribe(&env, CUST_UBER_ID, version, None, None, None)
        .await
        .expect("standard");
    env.services()
        .schedule_plan_change(
            Actor::System,
            sub,
            TENANT_ID,
            PlanTarget {
                version,
                option: Some(PlanOptionRef::Id(options[1].id)),
            },
            vec![],
        )
        .await
        .expect("a switch to startup at the period end");
    assert_eq!(fee_of(&env, sub, "Platform").await.0, dec!(20));
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("standard"));

    env.process_cycles().await;
    assert_eq!(fee_of(&env, sub, "Platform").await.0, dec!(15));
    assert_eq!(option_of(&env, sub).await.as_deref(), Some("startup"));
}

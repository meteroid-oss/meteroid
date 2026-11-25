//! Add-ons priced per currency and billing period, as plan charges: a subscription buys an add-on
//! at its price in the subscription's currency, on its term or its one cadence; an add-on with
//! no price there is refused. An add-on of a single price bills as it always did.
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use meteroid_store::domain::add_ons::{AddOn, AddOnPatch, AddOnPriceChange};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::price_components::{
    PriceComponentNew, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionAddOns,
    CreateSubscriptionComponents, SubscriptionFee,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::PlanSeed;
use crate::harness::{TestEnv, subscription, test_env};

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

/// A published plan whose platform fee is priced by `prices`.
async fn plan(env: &TestEnv, prices: Vec<PriceInput>) -> (PlanVersionId, PriceComponentId) {
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Pro", version)
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    set_draft(env, version, true).await;
    let component = env
        .store()
        .create_price_component_with_prices(
            PriceComponentNew {
                name: "Platform".to_string(),
                product_id: Some(PRODUCT_PLATFORM_FEE_ID),
                plan_version_id: version,
            },
            prices,
            TENANT_ID,
        )
        .await
        .expect("a charge")
        .id;
    set_draft(env, version, false).await;
    (version, component)
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

/// "Priority support", priced by `prices`.
async fn add_on(env: &TestEnv, prices: Vec<PriceInput>) -> AddOn {
    env.store()
        .create_add_on_with_prices(
            "Priority support".to_string(),
            ProductRef::New {
                name: "Priority support".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            prices.into_iter().map(PriceEntry::New).collect(),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on")
}

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

fn with(add_on: AddOnId) -> CreateSubscriptionAddOns {
    CreateSubscriptionAddOns {
        add_ons: vec![CreateSubscriptionAddOn {
            add_on_id: add_on,
            customization: SubscriptionAddOnCustomization::None,
            quantity: 1,
        }],
    }
}

async fn subscribe(
    env: &TestEnv,
    customer: CustomerId,
    version: PlanVersionId,
    components: CreateSubscriptionComponents,
    add_ons: Option<CreateSubscriptionAddOns>,
) -> meteroid_store::StoreResult<SubscriptionId> {
    let mut request = subscription()
        .customer(customer)
        .plan_version(version)
        .start_date(day(1))
        .on_start()
        .no_trial()
        .request();
    request.price_components = Some(components);
    request.add_ons = add_ons;
    env.services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .map(|created| created.id)
}

/// The add-on `sub` bought: its rate, period and price.
async fn add_on_fee_of(
    env: &TestEnv,
    sub: SubscriptionId,
) -> (Decimal, SubscriptionFeeBillingPeriod, Option<PriceId>) {
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details");
    let [bought] = details.add_ons.as_slice() else {
        panic!("one add-on, got {:?}", details.add_ons);
    };
    match &bought.fee {
        SubscriptionFee::Rate { rate } => (*rate, bought.period, bought.price_id),
        fee => panic!("a rate, got {fee:?}"),
    }
}

fn price_in(add_on: &AddOn, currency: &str, cadence: BillingPeriodEnum) -> PriceId {
    add_on
        .prices
        .iter()
        .find(|p| p.currency == currency && p.cadence == cadence)
        .unwrap_or_else(|| panic!("a {currency} {cadence:?} price"))
        .id
}

/// An add-on priced in EUR and USD: each subscription buys it at the price of its own currency.
#[tokio::test]
async fn a_eur_subscription_buys_the_add_on_at_its_eur_price() {
    let env = test_env().await;
    let (version, component) = plan(
        &env,
        vec![
            rate("EUR", BillingPeriodEnum::Monthly, dec!(20)),
            rate("USD", BillingPeriodEnum::Monthly, dec!(22)),
        ],
    )
    .await;
    let support = add_on(
        &env,
        vec![
            rate("USD", BillingPeriodEnum::Monthly, dec!(100)),
            rate("EUR", BillingPeriodEnum::Monthly, dec!(90)),
        ],
    )
    .await;
    assert_eq!(support.prices.len(), 2);
    assert_eq!(
        support.price_id,
        price_in(&support, "USD", BillingPeriodEnum::Monthly),
        "its first price stays its price_id"
    );
    customer_in(&env, CUST_SPOTIFY_ID, "USD").await;

    let eur = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        on(component, BillingPeriodEnum::Monthly),
        Some(with(support.id)),
    )
    .await
    .expect("in EUR");
    assert_eq!(
        add_on_fee_of(&env, eur).await,
        (
            dec!(90),
            SubscriptionFeeBillingPeriod::Monthly,
            Some(price_in(&support, "EUR", BillingPeriodEnum::Monthly))
        )
    );

    let usd = subscribe(
        &env,
        CUST_SPOTIFY_ID,
        version,
        on(component, BillingPeriodEnum::Monthly),
        Some(with(support.id)),
    )
    .await
    .expect("in USD");
    assert_eq!(add_on_fee_of(&env, usd).await.0, dec!(100));
}

/// A USD-only add-on is never sold to an EUR subscription: refused at creation and when added
/// later, never billed in USD.
#[tokio::test]
async fn a_usd_only_add_on_is_refused_on_a_eur_subscription() {
    let env = test_env().await;
    let (version, component) = plan(
        &env,
        vec![rate("EUR", BillingPeriodEnum::Monthly, dec!(20))],
    )
    .await;
    let support = add_on(
        &env,
        vec![
            rate("USD", BillingPeriodEnum::Monthly, dec!(100)),
            rate("USD", BillingPeriodEnum::Annual, dec!(1000)),
        ],
    )
    .await;

    let err = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        on(component, BillingPeriodEnum::Monthly),
        Some(with(support.id)),
    )
    .await
    .expect_err("not priced in EUR");
    assert!(
        refused(&err, "Priority support has no price in EUR"),
        "{err:?}"
    );

    let sub = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        on(component, BillingPeriodEnum::Monthly),
        None,
    )
    .await
    .expect("without the add-on");
    let err = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: with(support.id).add_ons,
                    edited: vec![],
                    removed: vec![],
                },
            },
            day(10),
        )
        .await
        .expect_err("added later, still not priced in EUR");
    assert!(
        refused(&err, "Priority support has no price in EUR"),
        "{err:?}"
    );
}

/// An annual subscription takes the add-on's annual price, a monthly one its monthly price.
#[tokio::test]
async fn an_annual_subscription_takes_the_add_ons_annual_price() {
    let env = test_env().await;
    let (version, component) = plan(
        &env,
        vec![
            rate("EUR", BillingPeriodEnum::Monthly, dec!(20)),
            rate("EUR", BillingPeriodEnum::Annual, dec!(200)),
        ],
    )
    .await;
    let support = add_on(
        &env,
        vec![
            rate("EUR", BillingPeriodEnum::Monthly, dec!(10)),
            rate("EUR", BillingPeriodEnum::Annual, dec!(100)),
        ],
    )
    .await;

    let annual = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        on(component, BillingPeriodEnum::Annual),
        Some(with(support.id)),
    )
    .await
    .expect("annual");
    assert_eq!(
        add_on_fee_of(&env, annual).await,
        (
            dec!(100),
            SubscriptionFeeBillingPeriod::Annual,
            Some(price_in(&support, "EUR", BillingPeriodEnum::Annual))
        )
    );

    let monthly = subscribe(
        &env,
        CUST_SPOTIFY_ID,
        version,
        on(component, BillingPeriodEnum::Monthly),
        Some(with(support.id)),
    )
    .await
    .expect("monthly");
    assert_eq!(
        add_on_fee_of(&env, monthly).await.0,
        dec!(10),
        "the monthly price, never the annual one"
    );
}

/// An add-on of one price (as every add-on was) bills as before: its price on any term, on its
/// own cadence; one whose price was never linked (before the migration) still reads it.
#[tokio::test]
async fn a_legacy_single_price_add_on_is_unchanged() {
    let env = test_env().await;
    let (version, component) = plan(
        &env,
        vec![
            rate("EUR", BillingPeriodEnum::Monthly, dec!(20)),
            rate("EUR", BillingPeriodEnum::Annual, dec!(200)),
        ],
    )
    .await;
    let legacy = env
        .store()
        .create_add_on_from_ref(
            "Priority support".to_string(),
            ProductRef::New {
                name: "Priority support".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(rate("EUR", BillingPeriodEnum::Monthly, dec!(100))),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("add-on");
    assert_eq!(legacy.prices.len(), 1);
    assert_eq!(legacy.prices[0].id, legacy.price_id);
    assert_eq!(legacy.price.as_ref().map(|p| p.id), Some(legacy.price_id));

    {
        use diesel_async::RunQueryDsl;
        diesel::sql_query("DELETE FROM add_on_price WHERE add_on_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(legacy.id.as_uuid())
            .execute(&mut env.conn().await)
            .await
            .expect("unlinked, as before the migration");
    }
    let read = env
        .store()
        .get_add_on_by_id(TENANT_ID, legacy.id)
        .await
        .expect("the add-on");
    assert_eq!(
        read.prices.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![legacy.price_id]
    );

    let annual = subscribe(
        &env,
        CUST_UBER_ID,
        version,
        on(component, BillingPeriodEnum::Annual),
        Some(with(legacy.id)),
    )
    .await
    .expect("an annual subscription with the monthly add-on");
    assert_eq!(
        add_on_fee_of(&env, annual).await,
        (
            dec!(100),
            SubscriptionFeeBillingPeriod::Monthly,
            Some(legacy.price_id)
        ),
        "priced monthly only, it bills monthly under every term"
    );
}

/// Repricing an add-on keeps the price left as it was, adds the new cell and unlinks the one
/// left out; two prices in one cell are refused.
#[tokio::test]
async fn editing_the_prices_keeps_an_unchanged_one() {
    let env = test_env().await;
    let support = add_on(
        &env,
        vec![
            rate("EUR", BillingPeriodEnum::Monthly, dec!(10)),
            rate("USD", BillingPeriodEnum::Monthly, dec!(11)),
        ],
    )
    .await;
    let eur_monthly = price_in(&support, "EUR", BillingPeriodEnum::Monthly);
    let patch = || AddOnPatch {
        id: support.id,
        tenant_id: TENANT_ID,
        name: None,
        description: None,
        self_serviceable: None,
        max_instances_per_subscription: None,
    };

    let edited = env
        .store()
        .update_add_on_with_prices(
            Actor::System,
            patch(),
            Some(AddOnPriceChange::All(vec![
                rate("EUR", BillingPeriodEnum::Monthly, dec!(10)),
                rate("EUR", BillingPeriodEnum::Annual, dec!(100)),
            ])),
        )
        .await
        .expect("repriced");
    assert_eq!(
        edited.price_id, eur_monthly,
        "its first price, unchanged, stays"
    );
    assert_eq!(edited.prices.len(), 2);
    assert!(edited.prices.iter().any(|p| p.id == eur_monthly));
    assert!(
        edited.prices.iter().all(|p| p.currency == "EUR"),
        "USD unlinked"
    );

    let err = env
        .store()
        .update_add_on_with_prices(
            Actor::System,
            patch(),
            Some(AddOnPriceChange::All(vec![
                rate("EUR", BillingPeriodEnum::Monthly, dec!(10)),
                rate("EUR", BillingPeriodEnum::Monthly, dec!(12)),
            ])),
        )
        .await
        .expect_err("two prices in one cell");
    assert!(
        refused(&err, "Priority support has two prices in EUR for Monthly"),
        "{err:?}"
    );
}

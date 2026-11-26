//! A product line's pricing page: its active ladder priced in one currency in one
//! option, each plan with the copy it carries.
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, EntitlementEntityId, PlanId, PlanVersionId, PriceComponentId, PriceId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::UsagePricingModel;
use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
use meteroid_store::domain::entitlements::{
    EntitlementNew, EntitlementSpec, FeatureKind, FeatureNew, LimitCountedBy,
};
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::plan_options::PlanOptionRef;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::pricing_pages::{
    CardExtraKind, Highlight, PageOption, PlanPricingPage, PricingCard, PricingPage,
    PricingPageRequest,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::{
    EntitlementsInterface, PlanOptionInterface, PlansInterface, PricingPagesInterface,
};
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp, SeedPrice, default_option_id, option_id};
use crate::harness::{TestEnv, test_env};

async fn ladder(env: &TestEnv, plans: Vec<PlanId>) {
    env.store()
        .set_self_serve_ladder(Actor::System, TENANT_ID, PRODUCT_FAMILY_ID, plans)
        .await
        .expect("a ladder");
}

async fn page_of(env: &TestEnv, request: PricingPageRequest) -> PricingPage {
    env.store()
        .pricing_page(TENANT_ID, PRODUCT_FAMILY_ID, request)
        .await
        .expect("a pricing page")
}

async fn page(env: &TestEnv, option: Option<&str>) -> PricingPage {
    page_of(
        env,
        PricingPageRequest {
            option: option.map(str::to_owned),
            ..Default::default()
        },
    )
    .await
}

/// Monthly, then Annual: what the page switches between.
fn monthly_and_annual() -> Vec<PageOption> {
    vec![
        PageOption {
            alias: "monthly".to_string(),
            name: "Monthly".to_string(),
        },
        PageOption {
            alias: "annual".to_string(),
            name: "Annual".to_string(),
        },
    ]
}

/// An option per cadence of a plan priced on several.
const MONTHLY_AND_ANNUAL: &[(&str, DieselBillingPeriodEnum)] = &[
    ("Monthly", DieselBillingPeriodEnum::Monthly),
    ("Annual", DieselBillingPeriodEnum::Annual),
];

fn card<'a>(page: &'a PricingPage, name: &str) -> &'a PricingCard {
    page.cards
        .iter()
        .find(|card| card.plan_name == name)
        .unwrap_or_else(|| panic!("a card for {name}"))
}

fn amount(page: &PricingPage, name: &str) -> Option<rust_decimal::Decimal> {
    card(page, name).price.as_ref().map(|p| p.amount)
}

/// A plan billed `rate` a month in EUR.
async fn rate_plan(
    env: &TestEnv,
    name: &'static str,
    rate: rust_decimal::Decimal,
) -> (PlanId, PlanVersionId, PriceComponentId, PriceId) {
    let (plan, version, component, price) = (
        PlanId::new(),
        PlanVersionId::new(),
        PriceComponentId::new(),
        PriceId::new(),
    );
    PlanSeed::new(plan, name, version)
        .components(vec![SeedComp::rate(
            component,
            "Platform",
            PRODUCT_PLATFORM_FEE_ID,
            price,
            DieselBillingPeriodEnum::Monthly,
            rate,
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    (plan, version, component, price)
}

fn seed_rate(cadence: DieselBillingPeriodEnum, rate: rust_decimal::Decimal) -> SeedPrice {
    SeedPrice {
        id: PriceId::new(),
        cadence,
        pricing: Pricing::Rate { rate },
    }
}

/// "Pro": Monthly at 20 a month or Annual at 200 a year, in EUR; its version and platform fee.
async fn pro_plan(env: &TestEnv, plan: PlanId) -> (PlanVersionId, PriceComponentId) {
    let (version, component) = (PlanVersionId::new(), PriceComponentId::new());
    let mut platform = SeedComp::rate(
        component,
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(20),
    );
    platform.prices = vec![
        seed_rate(DieselBillingPeriodEnum::Monthly, dec!(20)),
        seed_rate(DieselBillingPeriodEnum::Annual, dec!(200)),
    ];
    PlanSeed::new(plan, "Pro", version)
        .options(MONTHLY_AND_ANNUAL)
        .components(vec![platform])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    (version, component)
}

async fn free_plan(env: &TestEnv, name: &'static str) -> PlanId {
    let plan = PlanId::new();
    PlanSeed::new(plan, name, PlanVersionId::new())
        .free()
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan");
    plan
}

/// A price of `component` of `version` in `currency`, billed `rate` on `cadence`, in its default
/// option.
async fn price_in(
    env: &TestEnv,
    version: PlanVersionId,
    component: PriceComponentId,
    currency: &str,
    cadence: DieselBillingPeriodEnum,
    rate: rust_decimal::Decimal,
) -> PriceId {
    use diesel_models::plan_component_prices::PlanComponentPriceRowNew;
    use diesel_models::prices::PriceRowNew;
    let id = PriceId::new();
    let mut conn = env.conn().await;
    PriceRowNew {
        id,
        product_id: PRODUCT_PLATFORM_FEE_ID,
        cadence,
        currency: currency.to_string(),
        pricing: serde_json::to_value(Pricing::Rate { rate }).expect("pricing"),
        tenant_id: TENANT_ID,
        catalog: true,
        credit_unit_id: None,
        credit_unit_rate: None,
    }
    .insert(&mut conn)
    .await
    .expect("a price");
    PlanComponentPriceRowNew::insert_batch(
        &mut conn,
        &[PlanComponentPriceRowNew {
            plan_component_id: component,
            price_id: id,
            option_id: default_option_id(version),
        }],
    )
    .await
    .expect("the component's price");
    id
}

fn copy(plan_id: PlanId) -> PlanPricingPage {
    PlanPricingPage {
        plan_id,
        tagline: None,
        popular: false,
        cta_label: None,
        highlights: vec![],
        sales_led: false,
        hide_price: false,
        contact_url: None,
        product_order: vec![],
        hidden_product_ids: vec![],
    }
}

/// The ladder reads in rank order, each plan "everything in" the one below it; a plan with no
/// price is free in every currency.
#[tokio::test]
async fn the_ladder_reads_in_rank_order_each_plan_including_the_one_below() {
    let env = test_env().await;
    let starter = free_plan(&env, "Starter").await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    let (business, ..) = rate_plan(&env, "Business", dec!(50)).await;
    ladder(&env, vec![starter, pro, business]).await;

    let page = page(&env, None).await;
    let names: Vec<_> = page.cards.iter().map(|c| c.plan_name.as_str()).collect();
    assert_eq!(names, vec!["Starter", "Pro", "Business"]);
    let includes: Vec<_> = page
        .cards
        .iter()
        .map(|c| c.includes_plan_name.as_deref())
        .collect();
    assert_eq!(includes, vec![None, Some("Starter"), Some("Pro")]);
    assert_eq!(page.currency, "EUR");
    assert_eq!(page.option.as_deref(), Some("monthly"));
    assert_eq!(
        page.options,
        monthly_and_annual(),
        "a plan of one option, Business, shows on each"
    );

    let starter = card(&page, "Starter");
    assert!(starter.free);
    assert_eq!(starter.price.as_ref().map(|p| p.amount), Some(dec!(0)));
    let pro = card(&page, "Pro");
    assert!(!pro.free);
    let price = pro.price.as_ref().expect("priced");
    assert_eq!(price.amount, dec!(20));
    assert_eq!(price.period, Some(BillingPeriodEnum::Monthly));
    assert_eq!(price.monthly_equivalent, None);
    assert_eq!(price.saving_percent, None);
    let business = card(&page, "Business");
    assert_eq!(business.options, vec!["standard".to_string()]);
    assert_eq!(amount(&page, "Business"), Some(dec!(50)));
}

/// The page defaults to the tenant's currency; another is asked by currency. A plan not sold in
/// the currency is left off it, named apart.
#[tokio::test]
async fn a_currency_is_asked() {
    let env = test_env().await;
    let (global, global_version, component, _) = rate_plan(&env, "Global", dec!(20)).await;
    price_in(
        &env,
        global_version,
        component,
        "USD",
        DieselBillingPeriodEnum::Monthly,
        dec!(22),
    )
    .await;
    let (local, ..) = rate_plan(&env, "Local", dec!(15)).await;
    ladder(&env, vec![global, local]).await;

    let by_default = page(&env, None).await;
    assert_eq!(by_default.currency, "EUR");
    assert_eq!(by_default.currencies, vec!["EUR", "USD"]);
    assert_eq!(amount(&by_default, "Global"), Some(dec!(20)));

    let in_usd = page_of(
        &env,
        PricingPageRequest {
            currency: Some("usd".to_string()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(in_usd.currency, "USD");
    assert_eq!(in_usd.cards.len(), 1, "a plan not sold in USD is left off");
    assert_eq!(amount(&in_usd, "Global"), Some(dec!(22)));
    assert_eq!(in_usd.not_sold, vec!["Local".to_owned()]);
    assert!(by_default.not_sold.is_empty());

    let unknown = page_of(
        &env,
        PricingPageRequest {
            currency: Some("JPY".to_string()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(unknown.currency, "EUR", "a currency not sold falls back");
}

/// Paid yearly, the card shows the month it amounts to and what it saves against the option
/// billing the plan monthly.
#[tokio::test]
async fn a_yearly_option_shows_its_month_and_its_saving() {
    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    ladder(&env, vec![pro]).await;

    let page = page(&env, Some("annual")).await;
    assert_eq!(page.option.as_deref(), Some("annual"));
    let price = card(&page, "Pro").price.as_ref().expect("priced");
    assert_eq!(price.amount, dec!(200));
    assert_eq!(price.period, Some(BillingPeriodEnum::Annual));
    assert_eq!(price.monthly_equivalent, Some(dec!(16.67)));
    assert_eq!(price.saving_percent, Some(16), "200 against 12 × 20");
}

/// A card reads in the period of its option's main fee, a fee billed on another shown beside it;
/// a plan billing only usage claims no period.
#[tokio::test]
async fn a_card_reads_in_its_main_fees_period() {
    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    let (mixed, mixed_version) = (PlanId::new(), PlanVersionId::new());
    let mut platform = SeedComp::rate(
        PriceComponentId::new(),
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(10),
    );
    platform.prices = vec![
        seed_rate(DieselBillingPeriodEnum::Monthly, dec!(10)),
        seed_rate(DieselBillingPeriodEnum::Annual, dec!(100)),
    ];
    PlanSeed::new(mixed, "Mixed", mixed_version)
        .options(MONTHLY_AND_ANNUAL)
        .components(vec![
            platform,
            SeedComp::rate(
                PriceComponentId::new(),
                "Support",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(30),
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    let metered = PlanId::new();
    PlanSeed::new(metered, "Metered", PlanVersionId::new())
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit { rate: dec!(0.10) },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    ladder(&env, vec![pro, mixed, metered]).await;

    let monthly = page(&env, Some("monthly")).await;
    let mixed_card = card(&monthly, "Mixed");
    let monthly_price = mixed_card.price.as_ref().expect("sold monthly");
    assert_eq!(
        monthly_price.amount,
        dec!(10),
        "the yearly support never joins a monthly amount"
    );
    assert_eq!(monthly_price.period, Some(BillingPeriodEnum::Monthly));
    let support = monthly_price
        .extras
        .iter()
        .find(|e| e.name == "Support")
        .expect("the yearly support beside it");
    assert_eq!(support.amount, Some(dec!(30)));
    assert_eq!(support.period, Some(BillingPeriodEnum::Annual));
    assert_eq!(
        mixed_card.options,
        vec!["monthly".to_string(), "annual".to_string()]
    );
    let usage_only = card(&monthly, "Metered")
        .price
        .as_ref()
        .expect("its one option, on every page");
    assert_eq!(usage_only.period, None, "no period of its own");
    assert!(!card(&monthly, "Metered").free, "usage is charged");

    let yearly = page(&env, Some("annual")).await;
    let price = card(&yearly, "Mixed").price.as_ref().expect("priced");
    assert_eq!(
        price.amount,
        dec!(130),
        "both yearly fees, never a monthly one"
    );
    assert_eq!(price.period, Some(BillingPeriodEnum::Annual));
    assert_eq!(
        price.saving_percent, None,
        "130 a year saves nothing against 10 a month"
    );
    assert_eq!(amount(&yearly, "Pro"), Some(dec!(200)));
    assert_eq!(
        card(&yearly, "Metered")
            .price
            .as_ref()
            .and_then(|p| p.period),
        None
    );
}

/// Only active plans are shown: one made inactive leaves the page.
#[tokio::test]
async fn an_inactive_plan_is_hidden() {
    use diesel_async::RunQueryDsl;
    let env = test_env().await;
    let (live, ..) = rate_plan(&env, "Live", dec!(10)).await;
    let (retired, ..) = rate_plan(&env, "Retired", dec!(20)).await;
    ladder(&env, vec![live, retired]).await;
    diesel::sql_query("UPDATE plan SET status = 'INACTIVE' WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(retired.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("an inactive plan");

    let page = page(&env, None).await;
    let names: Vec<_> = page.cards.iter().map(|c| c.plan_name.as_str()).collect();
    assert_eq!(names, vec!["Live"]);
}

/// A sales-led plan stays on the page with its contact link, its price shown unless hidden; a
/// self-serve plan keeps neither.
#[tokio::test]
async fn a_sales_led_plan_shows_with_its_contact_link() {
    let env = test_env().await;
    let (team, ..) = rate_plan(&env, "Team", dec!(30)).await;
    let (enterprise, ..) = rate_plan(&env, "Enterprise", dec!(500)).await;
    ladder(&env, vec![team, enterprise]).await;
    let store = env.store();
    let contact = "https://acme.test/contact".to_string();

    let saved = store
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                sales_led: true,
                contact_url: Some(format!("  {contact} ")),
                ..copy(enterprise)
            },
        )
        .await
        .expect("sales-led");
    assert_eq!(saved.contact_url.as_deref(), Some(contact.as_str()));
    let kept = store
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                hide_price: true,
                contact_url: Some(contact.clone()),
                ..copy(team)
            },
        )
        .await
        .expect("self-serve");
    assert!(
        kept.hide_price && kept.contact_url.as_deref() == Some(contact.as_str()),
        "kept while sold self-serve, for when it is sales-led again"
    );

    let shown = page(&env, None).await;
    let names: Vec<_> = shown.cards.iter().map(|c| c.plan_name.as_str()).collect();
    assert_eq!(names, vec!["Team", "Enterprise"]);
    let card_of = card(&shown, "Enterprise");
    assert!(card_of.sales_led && !card_of.hide_price);
    assert_eq!(card_of.contact_url.as_deref(), Some(contact.as_str()));
    assert_eq!(amount(&shown, "Enterprise"), Some(dec!(500)));
    assert!(!card(&shown, "Team").sales_led);

    store
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                sales_led: true,
                hide_price: true,
                contact_url: Some(contact.clone()),
                ..copy(enterprise)
            },
        )
        .await
        .expect("price hidden");
    let hidden = page(&env, None).await;
    let card_of = card(&hidden, "Enterprise");
    assert!(card_of.hide_price);
    assert!(
        card_of.price.is_none(),
        "a hidden price never leaves the store"
    );
    assert!(!card_of.free);
    assert_eq!(amount(&hidden, "Team"), Some(dec!(30)));
}

/// Seats, usage and a one-time fee come on top of the fixed amount.
#[tokio::test]
async fn seats_usage_and_setup_are_charged_on_top() {
    let env = test_env().await;
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(plan, "Team", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(10),
            ),
            SeedComp::slot(
                PriceComponentId::new(),
                "Seats",
                PRODUCT_SEATS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(5),
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
            SeedComp::one_time(
                PriceComponentId::new(),
                "Setup",
                PRODUCT_ONETIME_SETUP_ID,
                PriceId::new(),
                dec!(99),
                1,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    ladder(&env, vec![plan]).await;

    let page = page(&env, None).await;
    let team = card(&page, "Team");
    assert!(!team.free, "free of a fixed fee, not of charges");
    let price = team.price.as_ref().expect("priced");
    assert_eq!(price.amount, dec!(10));
    assert!(!price.partial);
    let extra = |kind| {
        price
            .extras
            .iter()
            .find(|e| e.kind == kind)
            .unwrap_or_else(|| panic!("a {kind:?} extra"))
    };
    let seats = extra(CardExtraKind::PerSeat);
    assert_eq!(seats.name, "Seats");
    assert_eq!(seats.amount, Some(dec!(5)));
    assert_eq!(seats.unit.as_deref(), Some("Seats"));
    assert_eq!(seats.period, Some(BillingPeriodEnum::Monthly));
    let usage = extra(CardExtraKind::Usage);
    assert_eq!(usage.name, "Bandwidth");
    assert_eq!(usage.amount, Some(dec!(0.10)));
    assert!(usage.unit.is_some(), "the metric's unit or name");
    let setup = extra(CardExtraKind::OneTime);
    assert_eq!(setup.amount, Some(dec!(99)));
    let base = extra(CardExtraKind::Base);
    assert_eq!(base.amount, Some(dec!(10)));
    assert!(base.lead, "the base price leads as computed");
    assert_eq!(
        price.extras.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            CardExtraKind::Base,
            CardExtraKind::PerSeat,
            CardExtraKind::Usage,
            CardExtraKind::OneTime
        ]
    );
}

/// Capacity levels are never dropped: the card reads "from" the lowest one.
#[tokio::test]
async fn capacity_levels_show_from_the_lowest() {
    let env = test_env().await;
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    let mut capacity = SeedComp::capacity(
        PriceComponentId::new(),
        "Bandwidth",
        PRODUCT_CAPACITY_ID,
        METRIC_BANDWIDTH,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(99),
        10_000,
        dec!(0.10),
    );
    capacity.prices.push(SeedPrice {
        id: PriceId::new(),
        cadence: DieselBillingPeriodEnum::Monthly,
        pricing: Pricing::Capacity {
            rate: dec!(49),
            included: 1_000,
            overage_rate: dec!(0.20),
        },
    });
    PlanSeed::new(plan, "Scale", version)
        .components(vec![capacity])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    ladder(&env, vec![plan]).await;

    let page = page(&env, None).await;
    let scale = card(&page, "Scale");
    let price = scale.price.as_ref().expect("priced");
    assert!(price.from_lowest);
    assert_eq!(price.amount, dec!(49));
    let overage = price
        .extras
        .iter()
        .find(|e| e.kind == CardExtraKind::Usage)
        .expect("its overage");
    assert_eq!(overage.amount, None, "each level its own overage rate");
    assert!(!scale.free);
}

/// Highlights read what the published version resolves, its plan's rows included: a switch by its
/// name, a limit by its value, no limit as unlimited; a feature the plan does not grant, or one
/// twice, is refused.
#[tokio::test]
async fn highlights_render_the_versions_entitlements() {
    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    ladder(&env, vec![pro]).await;

    let feature = |name: &str, code: &str, kind| FeatureNew {
        tenant_id: TENANT_ID,
        product_id: None,
        name: name.to_owned(),
        code: code.to_owned(),
        description: None,
        kind,
        unit: None,
        level_type_id: None,
        policy: None,
        default: None,
    };
    let store = env.store();
    let create = |new| async move { store.create_feature(Actor::System, new).await };
    let sso = create(feature("SSO", "sso", FeatureKind::Boolean))
        .await
        .expect("sso");
    let events = create(feature(
        "events",
        "events",
        FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
    ))
    .await
    .expect("events");
    let projects = create(feature(
        "projects",
        "projects",
        FeatureKind::Limit(LimitCountedBy::Events(METRIC_DATABASE_SIZE)),
    ))
    .await
    .expect("projects");
    let audit = create(feature("Audit log", "audit", FeatureKind::Boolean))
        .await
        .expect("audit");

    let grant = |feature_id, value| EntitlementNew {
        tenant_id: TENANT_ID,
        entity: EntitlementEntityId::Plan(pro),
        resource_id: None,
        spec: EntitlementSpec::new(feature_id, value),
    };
    for new in [
        grant(sso.id, GrantValue::Boolean(true)),
        grant(
            events.id,
            GrantValue::Limit(LimitValue::Amount(dec!(10000))),
        ),
        grant(projects.id, GrantValue::Limit(LimitValue::Unlimited)),
    ] {
        env.store()
            .create_entitlement(&Actor::System, new)
            .await
            .expect("an entitlement");
    }

    for (highlights, why) in [
        (
            vec![Highlight::Feature(sso.id), Highlight::Feature(audit.id)],
            "a feature the plan does not grant",
        ),
        (
            vec![Highlight::Feature(sso.id), Highlight::Feature(sso.id)],
            "a feature twice",
        ),
    ] {
        assert!(
            env.store()
                .update_plan_pricing_page(
                    TENANT_ID,
                    PlanPricingPage {
                        highlights,
                        ..copy(pro)
                    },
                )
                .await
                .is_err(),
            "{why}"
        );
    }
    env.store()
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                highlights: vec![
                    Highlight::Feature(sso.id),
                    Highlight::Feature(events.id),
                    Highlight::Feature(projects.id),
                    Highlight::Text("  Priority support ".to_string()),
                ],
                tagline: Some("For growing teams".to_string()),
                cta_label: Some("Start free trial".to_string()),
                ..copy(pro)
            },
        )
        .await
        .expect("highlights saved");

    let page = page(&env, None).await;
    let pro = card(&page, "Pro");
    let texts: Vec<_> = pro.highlights.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "SSO",
            "10,000 events",
            "Unlimited projects",
            "Priority support"
        ]
    );
    assert_eq!(pro.highlights[0].feature_id, Some(sso.id));
    assert_eq!(pro.highlights[3].feature_id, None);
    assert_eq!(pro.tagline.as_deref(), Some("For growing teams"));
    assert_eq!(pro.cta_label.as_deref(), Some("Start free trial"));
}

/// One popular plan per product line: marking another unmarks the first. Copy is checked.
#[tokio::test]
async fn one_plan_is_popular_and_copy_is_checked() {
    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    let (team, ..) = rate_plan(&env, "Team", dec!(30)).await;
    let store = env.store();

    store
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                popular: true,
                ..copy(pro)
            },
        )
        .await
        .expect("pro popular");
    store
        .update_plan_pricing_page(
            TENANT_ID,
            PlanPricingPage {
                popular: true,
                ..copy(team)
            },
        )
        .await
        .expect("team popular");
    let popular = |plan| async move {
        store
            .get_plan_pricing_page(TENANT_ID, plan)
            .await
            .expect("copy")
            .popular
    };
    assert!(!popular(pro).await, "unmarked by the other");
    assert!(popular(team).await);

    let too_long = "x".repeat(121);
    for (page, why) in [
        (
            PlanPricingPage {
                tagline: Some(too_long.clone()),
                ..copy(pro)
            },
            "a tagline past 120 characters",
        ),
        (
            PlanPricingPage {
                cta_label: Some(too_long.clone()),
                ..copy(pro)
            },
            "a button past 120 characters",
        ),
        (
            PlanPricingPage {
                highlights: vec![Highlight::Text("  ".to_string())],
                ..copy(pro)
            },
            "a blank highlight",
        ),
        (
            PlanPricingPage {
                sales_led: true,
                contact_url: Some("javascript:alert(1)".to_string()),
                ..copy(pro)
            },
            "a contact link that is not http(s)",
        ),
        (
            PlanPricingPage {
                highlights: (0..13)
                    .map(|i| Highlight::Text(format!("line {i}")))
                    .collect(),
                ..copy(pro)
            },
            "more than 12 highlights",
        ),
        (
            PlanPricingPage {
                highlights: vec![Highlight::Feature(common_domain::ids::FeatureId::new())],
                ..copy(pro)
            },
            "a feature the plan does not grant",
        ),
    ] {
        assert!(
            store
                .update_plan_pricing_page(TENANT_ID, page)
                .await
                .is_err(),
            "{why}"
        );
    }
}

/// Through the API, "default" names the tenant's first product line and amounts read as money.
#[tokio::test]
async fn the_api_serves_the_default_product_lines_page() {
    use meteroid_grpc::meteroid::api::pricingpage::v1::GetPricingPageRequest;
    use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    ladder(&env, vec![pro]).await;

    let auth = crate::meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    let clients = crate::meteroid_it::clients::AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let page = clients
        .pricing_page
        .clone()
        .get_pricing_page(GetPricingPageRequest {
            product_family_id: "default".to_string(),
            currency: None,
            option: Some("annual".to_string()),
            draft: None,
        })
        .await
        .expect("the page")
        .into_inner()
        .page
        .expect("a page");
    assert_eq!(page.product_family_id, PRODUCT_FAMILY_ID.as_proto());
    assert_eq!(page.option.as_deref(), Some("annual"));
    assert_eq!(page.currency, "EUR");
    assert_eq!(page.currencies, vec!["EUR".to_string()]);
    let price = page.cards[0].price.as_ref().expect("priced");
    assert_eq!(price.amount, "200.00");
    assert_eq!(price.period, Some(BillingPeriod::Annual as i32));
    assert_eq!(price.monthly_equivalent.as_deref(), Some("16.67"));
    assert_eq!(price.saving_percent, Some(16));
}

/// A plan seeded with an option per `(name, cadence, rate)`, its platform fee priced in each on
/// its cadence, in EUR.
async fn options_plan(
    env: &TestEnv,
    name: &'static str,
    options: &[(&'static str, DieselBillingPeriodEnum, rust_decimal::Decimal)],
) -> (PlanId, PlanVersionId) {
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    let mut platform = SeedComp::rate(
        PriceComponentId::new(),
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        dec!(1),
    );
    platform.prices = options
        .iter()
        .map(|(_, cadence, rate)| seed_rate(cadence.clone(), *rate))
        .collect();
    let named: Vec<(&'static str, DieselBillingPeriodEnum)> = options
        .iter()
        .map(|(name, cadence, _)| (*name, cadence.clone()))
        .collect();
    PlanSeed::new(plan, name, version)
        .options(&named)
        .components(vec![platform])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    (plan, version)
}

/// Hides `version`'s option of `alias` as published: the store hides only a draft's.
async fn hide(env: &TestEnv, version: PlanVersionId, alias: &str) {
    diesel_models::plan_version_options::PlanVersionOptionRow::set_hidden(
        &mut env.conn().await,
        TENANT_ID,
        option_id(version, alias),
        true,
    )
    .await
    .expect("a hidden option");
}

/// The switch lists the options two plans sell at least: one plan's Quarterly is not on it, and
/// asking for it shows the first option.
#[tokio::test]
async fn the_switch_lists_the_options_plans_share() {
    let env = test_env().await;
    let (solo, _) = options_plan(
        &env,
        "Solo",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(10)),
            ("Quarterly", DieselBillingPeriodEnum::Quarterly, dec!(27)),
        ],
    )
    .await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    let (team, _) = options_plan(
        &env,
        "Team",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(40)),
            ("Annual", DieselBillingPeriodEnum::Annual, dec!(400)),
        ],
    )
    .await;
    ladder(&env, vec![solo, pro, team]).await;

    let page_of_monthly = page(&env, None).await;
    assert_eq!(
        page_of_monthly.options,
        monthly_and_annual(),
        "Quarterly is Solo's alone"
    );
    assert_eq!(
        card(&page_of_monthly, "Solo").options,
        vec!["monthly".to_string(), "quarterly".to_string()],
        "the card still says what the plan sells"
    );
    let quarterly = page(&env, Some("quarterly")).await;
    assert_eq!(
        quarterly.option.as_deref(),
        Some("monthly"),
        "not on the switch"
    );
    assert_eq!(amount(&quarterly, "Solo"), Some(dec!(10)));
}

/// A plan lacking the option shown is priced in its default, with a note: its option's name when
/// it sells several, how it bills when it sells one. Only a plan unsold in the currency has none.
#[tokio::test]
async fn a_plan_without_the_option_shown_reads_in_its_default() {
    let env = test_env().await;
    let pro = PlanId::new();
    pro_plan(&env, pro).await;
    let (team, _) = options_plan(
        &env,
        "Team",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(40)),
            ("Annual", DieselBillingPeriodEnum::Annual, dec!(400)),
        ],
    )
    .await;
    let (solo, _) = options_plan(
        &env,
        "Solo",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(10)),
            ("Quarterly", DieselBillingPeriodEnum::Quarterly, dec!(27)),
        ],
    )
    .await;
    let (business, ..) = rate_plan(&env, "Business", dec!(50)).await;
    ladder(&env, vec![solo, pro, team, business]).await;

    let annual = page(&env, Some("annual")).await;
    assert_eq!(annual.option.as_deref(), Some("annual"));
    let solo = card(&annual, "Solo");
    let price = solo
        .price
        .as_ref()
        .expect("its default, never 'not offered'");
    assert_eq!(price.amount, dec!(10));
    assert_eq!(price.period, Some(BillingPeriodEnum::Monthly));
    assert_eq!(solo.option_note.as_deref(), Some("Monthly only"));
    let business = card(&annual, "Business");
    assert_eq!(amount(&annual, "Business"), Some(dec!(50)));
    assert_eq!(business.option_note.as_deref(), Some("Billed monthly"));
    assert_eq!(card(&annual, "Pro").option_note, None, "sold in Annual");
    assert_eq!(amount(&annual, "Pro"), Some(dec!(200)));

    let monthly = page(&env, Some("monthly")).await;
    assert!(
        monthly.cards.iter().all(|card| card.option_note.is_none()),
        "every plan sells Monthly"
    );
}

/// A hidden option is off the switch and the cards, never shown in a plan's place, but a
/// subscription is still made on it when named.
#[tokio::test]
async fn a_hidden_option_is_sold_when_named_only() {
    let env = test_env().await;
    let pro = PlanId::new();
    let (pro_version, _) = pro_plan(&env, pro).await;
    let (team, _) = options_plan(
        &env,
        "Team",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(40)),
            ("Annual", DieselBillingPeriodEnum::Annual, dec!(400)),
        ],
    )
    .await;
    let (scale, scale_version) = options_plan(
        &env,
        "Scale",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(90)),
            ("Annual", DieselBillingPeriodEnum::Annual, dec!(900)),
        ],
    )
    .await;
    ladder(&env, vec![pro, team, scale]).await;
    hide(&env, scale_version, "annual").await;

    let annual = page(&env, Some("annual")).await;
    assert_eq!(
        annual.options,
        monthly_and_annual(),
        "Pro and Team sell Annual"
    );
    let scale_card = card(&annual, "Scale");
    assert_eq!(scale_card.options, vec!["monthly".to_string()]);
    assert_eq!(
        amount(&annual, "Scale"),
        Some(dec!(90)),
        "its Monthly, not its hidden Annual"
    );
    assert_eq!(scale_card.option_note.as_deref(), Some("Billed monthly"));

    // Hidden on two plans of three: Annual is Team's alone, no longer on the switch.
    hide(&env, pro_version, "annual").await;
    let page = page(&env, None).await;
    assert!(
        page.options.iter().all(|o| o.alias != "annual"),
        "one plan's Annual is no switch"
    );

    let mut request = crate::harness::subscription()
        .customer(CUST_UBER_ID)
        .plan_version(scale_version)
        .start_date(chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .request();
    request.option = Some(PlanOptionRef::Alias("annual".to_string()));
    let created = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("a subscription on the hidden option");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, created.id)
        .await
        .expect("details");
    assert_eq!(
        details.plan_option.map(|option| option.alias).as_deref(),
        Some("annual")
    );
}

/// The default option is always shown, and never becomes a hidden one's place by a reorder.
#[tokio::test]
async fn the_default_option_cannot_be_hidden() {
    use diesel_async::RunQueryDsl;
    let env = test_env().await;
    let (_, version) = options_plan(
        &env,
        "Pro",
        &[
            ("Monthly", DieselBillingPeriodEnum::Monthly, dec!(20)),
            ("Annual", DieselBillingPeriodEnum::Annual, dec!(200)),
        ],
    )
    .await;
    diesel::sql_query("UPDATE plan_version SET is_draft_version = TRUE WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(version.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("a draft");
    let store = env.store();
    let (monthly, annual) = (option_id(version, "monthly"), option_id(version, "annual"));

    let refused = store
        .update_plan_option(TENANT_ID, monthly, None, None, Some(true))
        .await
        .expect_err("the default is shown");
    let says_shown = matches!(
        refused.current_context(),
        StoreError::InvalidArgument(message) if message == "The default option is always shown"
    );
    assert!(says_shown, "{refused:?}");

    let hidden = store
        .update_plan_option(TENANT_ID, annual, None, None, Some(true))
        .await
        .expect("another option is hidden");
    assert!(hidden.hidden);
    let _ = store
        .reorder_plan_options(TENANT_ID, version, vec![annual, monthly])
        .await
        .expect_err("a hidden option never becomes the default");
    let shown = store
        .update_plan_option(TENANT_ID, annual, None, None, Some(false))
        .await
        .expect("shown again");
    assert!(!shown.hidden);
}

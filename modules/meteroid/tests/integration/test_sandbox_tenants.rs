//! Sandboxes are core: without any module, a Sandbox tenant is seeded from the default preset,
//! the AI credits preset stays behind its gate, and a new organization gets no sandbox.

use std::sync::Arc;
use std::time::Duration;

use common_domain::ids::TenantId;
use meteroid::module::{ApiContext, Modules};
use meteroid_app::features::OssEntitlements;
use meteroid_app::services::egress::Egress;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::tenants::v1::{SandboxPreset, TenantEnvironmentEnum};
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::sandbox_seeds::SandboxSeedStatus;
use meteroid_store::hooks::no_analytics_hook;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::sandbox::SandboxInterface;

use crate::data::ids::ORGANIZATION_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

/// Seeding runs in the background.
async fn await_seeded(setup: &MeteroidSetup, tenant_id: TenantId) {
    for _ in 0..240 {
        let seed = setup.store.get_sandbox_seed(tenant_id).await.unwrap();
        match seed.map(|s| s.status) {
            Some(SandboxSeedStatus::Completed) => return,
            Some(SandboxSeedStatus::Failed) => panic!("the default sandbox seed failed"),
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    panic!("the default sandbox seed did not complete");
}

#[tokio::test]
async fn core_seeds_sandbox_tenants_but_none_at_signup() {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    let tenant = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Demo".to_string(),
            environment: TenantEnvironmentEnum::Sandbox as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: SandboxPreset::Default as i32,
            copy_from: None,
            sandbox_template: None,
        })
        .await
        .expect("core creates a sandbox without the sandboxes module")
        .into_inner()
        .tenant
        .unwrap();
    assert_eq!(tenant.environment(), TenantEnvironmentEnum::Sandbox);
    assert_eq!(tenant.name, "Demo");

    let tenant_id = TenantId::from_proto(tenant.id.as_str()).unwrap();
    await_seeded(&setup, tenant_id).await;
    let customers = setup
        .store
        .list_customers(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(10),
            },
            None,
            None,
            Some(false),
        )
        .await
        .unwrap();
    assert!(
        !customers.items.is_empty(),
        "the default preset seeds its customers"
    );

    let refused = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Halyard".to_string(),
            environment: TenantEnvironmentEnum::Sandbox as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: SandboxPreset::AiCredits as i32,
            copy_from: None,
            sandbox_template: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::PermissionDenied);

    let ctx = ApiContext {
        config: setup.config.clone(),
        store: setup.store.clone(),
        services: setup.services.clone(),
        feature_resolver: Arc::new(OssEntitlements),
        feature_gates: Arc::new([]),
        rate_limiter: Default::default(),
        redis: None,
        analytics: no_analytics_hook(),
        egress: Egress::new(setup.config.webhook_out.clone(), None),
        object_store: setup.object_store.clone(),
    };
    let signup = Modules::core()
        .sandboxes(&ctx)
        .provision_for_signup(ORGANIZATION_ID)
        .await
        .unwrap();
    assert!(signup.is_none(), "core gives a new organization no sandbox");
}

/// The currencies and coupons preset prices every plan of the ladder in USD and EUR, monthly and
/// annually, and Starter and Pro in BRL monthly; each customer subscribes in their own currency,
/// and two coupons are applied: YC 2026 to a startup, Students in Brazil to a verified student.
/// Enterprise is sold by sales, its price off the page; Students, off the ladder, is for verified
/// students; Pro and Business price LLM tokens by a rate card; products carry features.
#[tokio::test]
async fn the_currencies_and_coupons_preset_prices_three_currencies_and_applies_coupons() {
    use meteroid_store::domain::coupons::{CouponDiscount, CouponFilter};
    use meteroid_store::domain::eligibility::PropertyMatch;
    use meteroid_store::domain::enums::CustomPropertyEntityType;
    use meteroid_store::domain::plans::PlanVersionFilter;
    use meteroid_store::domain::pricing_pages::PricingPageRequest;
    use meteroid_store::domain::rate_cards::rates_at;
    use meteroid_store::domain::{PlanFilters, Pricing, UsagePricingModel};
    use meteroid_store::repositories::coupons::CouponInterface;
    use meteroid_store::repositories::product_families::ProductFamilyInterface;
    use meteroid_store::repositories::subscriptions::SubscriptionInterface;
    use meteroid_store::repositories::{
        CustomPropertyDefinitionsInterface, EntitlementsInterface, PlansInterface,
        PricingPagesInterface, RateCardsInterface,
    };
    use rust_decimal_macros::dec;

    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let tenant = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Acme Analytics".to_string(),
            environment: TenantEnvironmentEnum::Sandbox as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: SandboxPreset::CurrenciesAndCoupons as i32,
            copy_from: None,
            sandbox_template: None,
        })
        .await
        .expect("open to every tenant")
        .into_inner()
        .tenant
        .unwrap();
    let tenant_id = TenantId::from_proto(tenant.id.as_str()).unwrap();
    await_seeded(&setup, tenant_id).await;

    let family = setup
        .store
        .find_default_product_family(tenant_id)
        .await
        .unwrap();
    let page = |currency: &str, option: &str| PricingPageRequest {
        currency: Some(currency.to_string()),
        option: Some(option.to_string()),
        draft: None,
    };
    let usd = setup
        .store
        .pricing_page(tenant_id, family.id, page("USD", "monthly"))
        .await
        .expect("the USD page");
    let names: Vec<&str> = usd.cards.iter().map(|c| c.plan_name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Starter", "Pro", "Business", "Enterprise"],
        "the ladder in rank order, Students off it"
    );
    assert_eq!(
        usd.currencies,
        vec!["BRL".to_string(), "EUR".to_string(), "USD".to_string()]
    );
    assert_eq!(
        usd.options
            .iter()
            .map(|o| o.alias.as_str())
            .collect::<Vec<_>>(),
        vec!["monthly", "annual"]
    );
    assert!(usd.cards[1].popular, "Pro is the popular plan");
    assert_eq!(usd.cards[0].price.as_ref().unwrap().amount, dec!(29));
    let enterprise = &usd.cards[3];
    assert!(enterprise.sales_led && enterprise.hide_price);
    assert!(enterprise.price.is_none(), "Enterprise shows no price");
    assert_eq!(enterprise.tagline.as_deref(), Some("For large teams"));
    assert_eq!(enterprise.cta_label.as_deref(), Some("Talk to sales"));
    assert_eq!(
        enterprise.contact_url.as_deref(),
        Some("https://example.com/contact-sales")
    );
    let lines = |card: usize| -> Vec<String> {
        usd.cards[card]
            .highlights
            .iter()
            .map(|h| h.text.clone())
            .collect()
    };
    assert_eq!(lines(0)[0], "3 Seats included");
    assert!(lines(1).contains(&"25,000,000 LLM tokens per month".to_string()));
    assert!(lines(1).contains(&"API access".to_string()));
    assert!(lines(2).contains(&"SSO".to_string()));
    assert!(lines(3).contains(&"SSO".to_string()));

    let eur = setup
        .store
        .pricing_page(tenant_id, family.id, page("EUR", "annual"))
        .await
        .expect("the EUR page");
    assert_eq!(eur.currency, "EUR");
    assert_eq!(eur.cards[1].price.as_ref().unwrap().amount, dec!(890));

    let brl = setup
        .store
        .pricing_page(tenant_id, family.id, page("BRL", "annual"))
        .await
        .expect("the BRL page");
    assert_eq!(brl.currency, "BRL");
    assert_eq!(
        brl.options
            .iter()
            .map(|o| o.alias.as_str())
            .collect::<Vec<_>>(),
        vec!["monthly"],
        "BRL sells Monthly only"
    );
    assert_eq!(brl.option.as_deref(), Some("monthly"));
    assert_eq!(brl.cards[0].price.as_ref().unwrap().amount, dec!(149));
    assert_eq!(brl.cards[1].price.as_ref().unwrap().amount, dec!(499));
    assert!(
        brl.cards[2].price.is_none() && brl.cards[2].options.is_empty(),
        "Business is not sold in BRL"
    );

    let plans = setup
        .store
        .list_plans(
            tenant_id,
            None,
            PlanFilters {
                search: None,
                filter_status: vec![],
                filter_type: vec![],
                filter_currency: None,
            },
            PaginationRequest {
                page: 0,
                per_page: Some(20),
            },
            None,
        )
        .await
        .unwrap()
        .items;
    let plan = |name: &str| {
        plans
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("the {name} plan"))
            .clone()
    };
    let students = plan("Students");
    assert_eq!(
        students.self_service_rank, None,
        "Students is off the ladder"
    );
    assert_eq!(
        students
            .eligibility
            .as_ref()
            .map(|rule| rule.properties.clone()),
        Some(vec![PropertyMatch {
            key: "student".to_string(),
            values: vec!["verified".to_string()],
        }]),
        "Students is for verified students"
    );
    let enterprise = plan("Enterprise");
    assert_eq!(enterprise.self_service_rank, Some(4));
    assert!(enterprise.pricing_sales_led && enterprise.pricing_hide_price);

    let cards = setup
        .store
        .list_rate_cards(tenant_id, None, None)
        .await
        .unwrap();
    let [card] = cards.as_slice() else {
        panic!("one rate card, got {}", cards.len());
    };
    assert_eq!(card.code, "llm-tokens");
    assert_eq!(card.dimensions, vec!["model", "token_type"]);
    assert_eq!(card.unit_size, dec!(1000000), "per 1M tokens");
    let (_, entries) = setup.store.get_rate_card(tenant_id, card.id).await.unwrap();
    let in_force = rates_at(&entries, chrono::Utc::now().date_naive());
    for currency in ["USD", "EUR", "BRL"] {
        let keys = in_force
            .iter()
            .filter(|e| e.currency.as_deref() == Some(currency))
            .count();
        assert_eq!(keys, 6, "every key rated in {currency}");
    }
    let rate = |key: [&str; 2], currency: &str| {
        in_force
            .iter()
            .find(|e| e.key_text() == key && e.currency.as_deref() == Some(currency))
            .and_then(|e| e.rate)
    };
    assert_eq!(rate(["gpt-4o", "input"], "USD"), Some(dec!(2.50)));
    assert_eq!(rate(["*", "*"], "BRL"), Some(dec!(27)));
    for (name, multiplier) in [("Pro", None), ("Business", Some(dec!(0.8)))] {
        let full = setup
            .store
            .get_full_plan(plan(name).id, tenant_id, PlanVersionFilter::Active)
            .await
            .unwrap();
        let llm = full
            .price_components
            .iter()
            .find(|c| c.name == "LLM tokens")
            .unwrap_or_else(|| panic!("{name} charges LLM tokens"));
        assert!(
            llm.prices.iter().all(|price| matches!(
                &price.pricing,
                Pricing::Usage(UsagePricingModel::RateCard { rate_card_id, multiplier: m })
                    if *rate_card_id == card.id && *m == multiplier
            )),
            "{name} prices LLM tokens by the card"
        );
    }

    let features = setup
        .store
        .list_features(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(20),
            },
            None,
            None,
            None,
            None,
            meteroid_store::domain::entitlements::FeatureListOrder::default(),
        )
        .await
        .unwrap()
        .items;
    let product_of = |code: &str| {
        features
            .iter()
            .find(|f| f.code == code)
            .unwrap_or_else(|| panic!("the {code} feature"))
            .product
            .as_ref()
            .map(|p| p.name.clone())
    };
    assert_eq!(product_of("seats_included").as_deref(), Some("Seats"));
    assert_eq!(product_of("sso").as_deref(), Some("Platform fee"));
    assert_eq!(product_of("api_access").as_deref(), Some("Platform fee"));
    assert_eq!(
        product_of("llm_tokens_per_month").as_deref(),
        Some("LLM tokens")
    );

    let customers = setup
        .store
        .list_customers(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(100),
            },
            None,
            None,
            Some(false),
        )
        .await
        .unwrap()
        .items;
    let currencies: std::collections::HashSet<&str> =
        customers.iter().map(|c| c.currency.as_str()).collect();
    assert!(["USD", "EUR", "BRL"].iter().all(|c| currencies.contains(c)));
    for customer in &customers {
        let subscriptions = setup
            .store
            .list_subscriptions(
                tenant_id,
                Some(customer.id),
                None,
                None,
                None,
                vec![],
                None,
                PaginationRequest {
                    page: 0,
                    per_page: Some(10),
                },
            )
            .await
            .unwrap()
            .items;
        assert!(!subscriptions.is_empty(), "{} subscribes", customer.name);
        for sub in subscriptions {
            assert_eq!(
                sub.currency, customer.currency,
                "{} bills in their currency",
                customer.name
            );
        }
    }

    let coupons = setup
        .store
        .list_coupons(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(10),
            },
            None,
            CouponFilter::ALL,
            None,
            None,
        )
        .await
        .unwrap()
        .items;
    let coupon = |code: &str| {
        coupons
            .iter()
            .find(|c| c.code == code)
            .unwrap_or_else(|| panic!("the {code} coupon"))
            .clone()
    };
    let yc = coupon("YC26");
    assert_eq!(yc.discount, CouponDiscount::Percentage(dec!(50)));
    assert_eq!(yc.plan_ids.len(), 1, "YC 2026 is on Pro only");
    assert_eq!(yc.eligibility, None, "YC 2026 is for whoever has the code");
    assert_eq!(yc.duration_months, Some(12), "YC 2026 lasts 12 months");
    let students = coupon("ESTUDANTE-BR");
    assert_eq!(students.discount, CouponDiscount::Percentage(dec!(40)));
    assert_eq!(students.redemption_limit, Some(500));
    assert_eq!(
        students.duration_months,
        Some(6),
        "Students in Brazil lasts 6 months"
    );
    assert!(students.valid_from.is_some() && students.expires_at.is_some());
    let rule = students.eligibility.as_ref().expect("a rule");
    assert_eq!(rule.countries, vec!["BR".to_string()]);
    assert_eq!(
        rule.properties,
        vec![PropertyMatch {
            key: "student".to_string(),
            values: vec!["verified".to_string()],
        }]
    );

    for (coupon, customer) in [(&yc, "Quillstack"), (&students, "Lucas Almeida")] {
        let applied = setup
            .store
            .list_applied_coupons_by_coupon_id(
                tenant_id,
                coupon.id,
                PaginationRequest {
                    page: 0,
                    per_page: Some(10),
                },
            )
            .await
            .unwrap()
            .items;
        let names: Vec<&str> = applied.iter().map(|a| a.customer_name.as_str()).collect();
        assert_eq!(names, vec![customer], "{} is applied", coupon.code);
    }

    // `student` is a customer custom property, so the customer page shows it and an edit keeps it.
    let definitions = setup
        .store
        .list_custom_property_definitions(
            tenant_id,
            Some(CustomPropertyEntityType::Customer),
            false,
        )
        .await
        .unwrap();
    let student = definitions
        .iter()
        .find(|d| d.key == "student")
        .expect("the student property");
    let options = student.config.options.as_deref().unwrap_or_default();
    assert!(options.iter().any(|o| o.value == "verified"));
}

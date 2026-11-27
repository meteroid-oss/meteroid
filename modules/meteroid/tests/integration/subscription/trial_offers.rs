//! Trial offers: a catalog trial redeemed at creation sets the subscription's trial and writes
//! its end (R-153). Starter is 29.00 + Seats 10.00 a month in advance.

use common_domain::actor::Actor;
use meteroid_store::domain::trials::{TrialEligibility, TrialEndAction, TrialOfferNew};
use rstest::rstest;

use super::phases::{at, day, documents, seed_annual_platform};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

async fn offer(
    env: &TestEnv,
    end_action: TrialEndAction,
    eligibility: TrialEligibility,
) -> common_domain::ids::TrialOfferId {
    env.services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "Two weeks free".to_string(),
                code: None,
                duration_days: 14,
                free: true,
                trial_plan_id: None,
                end_action,
                eligibility,
            },
        )
        .await
        .expect("an offer")
        .id
}

async fn redeem(
    env: &TestEnv,
    offer: common_domain::ids::TrialOfferId,
) -> meteroid_store::StoreResult<meteroid_store::domain::CreatedSubscription> {
    env.services()
        .insert_subscription(
            Actor::System,
            subscription()
                .plan_version(PLAN_VERSION_STARTER_ID)
                .start_date(day(1, 1))
                .on_start()
                .trial_offer(offer)
                .request(),
            TENANT_ID,
        )
        .await
}

/// Two weeks free, then billed from January 15. Once the customer had a trial on the plan, an
/// offer given once per plan is refused; one given once per offer is given once.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_free_trial_offer_bills_from_its_end_once_per_customer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let by_offer = offer(&env, TrialEndAction::Convert, TrialEligibility::Offer).await;
    let by_plan = offer(&env, TrialEndAction::Convert, TrialEligibility::Plan).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_offer(by_plan)
        .create(env.services())
        .await;
    let again = redeem(&env, by_plan).await;
    assert!(format!("{:?}", again.expect_err("plan redeemed")).contains("already had"));
    redeem(&env, by_offer).await.expect("the offer, once");
    let again = redeem(&env, by_offer).await;
    assert!(format!("{:?}", again.expect_err("offer redeemed")).contains("already had"));

    env.run_until(day(2, 16)).await;
    assert_eq!(
        documents(&env, sub_id)
            .await
            .into_iter()
            .filter(|line| !line.ends_with(" 0"))
            .collect::<Vec<_>>(),
        vec![
            "Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 2900",
            "Recurring 2024-01-15: Seats 2024-01-15..2024-02-15 1000",
            "Recurring 2024-02-15: Platform Fee 2024-02-15..2024-03-15 2900",
            "Recurring 2024-02-15: Seats 2024-02-15..2024-03-15 1000",
        ]
    );
}

/// A trial cancelling at its end bills nothing and completes on January 15: its end date, no term.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_trial_offer_cancelling_ends_with_its_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let ends = offer(&env, TrialEndAction::Cancel, TrialEligibility::Offer).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_offer(ends)
        .create(env.services())
        .await;
    env.run_until(day(2, 16)).await;
    assert!(
        documents(&env, sub_id)
            .await
            .iter()
            .all(|line| line.ends_with(" 0")),
        "{:?}",
        documents(&env, sub_id).await
    );
    assert_eq!(
        env.get_subscription(sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Completed
    );
}

/// A trial falling back to the annual plan switches to it on January 15: 290.00 a year from then.
/// Rules: R-153, R-150
#[rstest]
#[tokio::test]
async fn a_trial_offer_falling_back_switches_plan_at_its_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let (annual, _) = seed_annual_platform(&env).await;
    let falls_back = offer(
        &env,
        TrialEndAction::Fallback {
            plan_version_id: annual,
        },
        TrialEligibility::Offer,
    )
    .await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_offer(falls_back)
        .create(env.services())
        .await;
    env.run_until(day(1, 16)).await;
    assert_eq!(
        documents(&env, sub_id)
            .await
            .into_iter()
            .filter(|line| !line.ends_with(" 0"))
            .collect::<Vec<_>>(),
        vec!["Recurring 2024-01-15: Platform Fee 2024-01-15..2025-01-15 29000"]
    );
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, annual);
}

fn starter_platform_at(
    price: rust_decimal::Decimal,
) -> meteroid_store::domain::subscription_phases::PhaseComponent {
    meteroid_store::domain::subscription_phases::PhaseComponent {
        lineage: None,
        plan_component: Some(COMP_STARTER_PLATFORM_FEE_ID),
        price,
        list_rate: None,
        name: None,
    }
}

/// Starter's template: 19.00 for its first three months, then 29.00. A later phase leaving the
/// intro price out is refused: it would bill it forever.
/// Rules: R-154, R-146
#[rstest]
#[tokio::test]
async fn a_plan_template_prices_a_subscription_from_its_start(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::PhaseTerms;
    use meteroid_store::domain::trial_offers::PlanPhaseTemplate;
    use rust_decimal_macros::dec;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let template = |list: Option<rust_decimal::Decimal>| {
        vec![
            PlanPhaseTemplate {
                offset_months: 0,
                label: Some("Intro".into()),
                terms: PhaseTerms {
                    components: vec![starter_platform_at(dec!(19.00))],
                    ..PhaseTerms::default()
                },
            },
            PlanPhaseTemplate {
                offset_months: 3,
                label: Some("List".into()),
                terms: PhaseTerms {
                    components: list.map(starter_platform_at).into_iter().collect(),
                    ..PhaseTerms::default()
                },
            },
        ]
    };
    let sticky = env
        .services()
        .set_plan_version_phases(TENANT_ID, PLAN_VERSION_STARTER_ID, template(None))
        .await;
    assert!(format!("{:?}", sticky.expect_err("intro forever")).contains("absolute"));
    env.services()
        .set_plan_version_phases(
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            template(Some(dec!(29.00))),
        )
        .await
        .expect("an intro price");
    let request = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .request();
    let (preview, _) = env
        .services()
        .preview_create_subscription(&request, TENANT_ID)
        .await
        .expect("a preview");
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("created")
        .id;
    env.run_until(day(4, 2)).await;
    let mut invoices = env.get_invoices(sub_id).await;
    invoices.sort_by_key(|invoice| invoice.billing_period_start);
    assert_eq!(
        preview.subtotal, invoices[0].subtotal,
        "the preview a checkout charges bills the template's start"
    );
    assert_eq!(
        documents(&env, sub_id)
            .await
            .into_iter()
            .filter(|line| line.contains("Platform Fee"))
            .collect::<Vec<_>>(),
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 1900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 1900",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 1900",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
        ]
    );
}

/// An accepted quote on Starter selling its platform fee alone, with `phases`, converted.
async fn converted_quote(
    env: &TestEnv,
    phases: Vec<meteroid_store::domain::trial_offers::PlanPhaseTemplate>,
) -> common_domain::ids::SubscriptionId {
    converted_quote_with(
        env,
        phases,
        Some(0),
        meteroid_store::domain::enums::SubscriptionFeeBillingPeriod::Monthly,
    )
    .await
}

async fn converted_quote_with(
    env: &TestEnv,
    phases: Vec<meteroid_store::domain::trial_offers::PlanPhaseTemplate>,
    trial_duration_days: Option<i32>,
    period: meteroid_store::domain::enums::SubscriptionFeeBillingPeriod,
) -> common_domain::ids::SubscriptionId {
    use meteroid_store::repositories::quotes::QuotesInterface;
    let quote_id = draft_quote(env, trial_duration_days, period).await;
    env.services()
        .set_quote_phases(TENANT_ID, quote_id, phases)
        .await
        .expect("the quote's phases");
    env.store()
        .publish_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("published");
    let sent = env
        .services()
        .set_quote_phases(TENANT_ID, quote_id, vec![])
        .await;
    assert!(format!("{:?}", sent.expect_err("sent")).contains("draft"));
    env.store()
        .accept_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("accepted");
    let sub_id = env
        .services()
        .convert_quote_to_subscription(Actor::System, TENANT_ID, quote_id)
        .await
        .expect("converted")
        .subscription
        .id;
    let late = env
        .services()
        .set_quote_phases(TENANT_ID, quote_id, vec![])
        .await;
    assert!(format!("{:?}", late.expect_err("converted")).contains("draft"));
    sub_id
}

/// A draft quote on Starter for Uber, selling its platform fee alone at 29.00 every `period`.
pub(super) async fn draft_quote(
    env: &TestEnv,
    trial_duration_days: Option<i32>,
    period: meteroid_store::domain::enums::SubscriptionFeeBillingPeriod,
) -> common_domain::ids::QuoteId {
    use common_domain::ids::{BaseId, QuoteId};
    use meteroid_store::domain::SubscriptionActivationCondition;
    use meteroid_store::domain::enums::QuoteStatusEnum;
    use meteroid_store::domain::quotes::{QuoteNew, QuotePriceComponentNew};
    use meteroid_store::domain::subscription_components::SubscriptionFee;
    use meteroid_store::repositories::quotes::QuotesInterface;
    let quote_id = QuoteId::new();
    env.store()
        .insert_quote_with_details(
            QuoteNew {
                id: quote_id,
                status: QuoteStatusEnum::Draft,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                plan_version_id: PLAN_VERSION_STARTER_ID,
                currency: "EUR".to_string(),
                quote_number: format!("Q-{}", quote_id.as_base62()),
                trial_duration_days,
                trial_free: None,
                trial_plan_id: None,
                resource_id: None,
                billing_start_date: Some(day(1, 1)),
                billing_end_date: None,
                billing_day_anchor: None,
                activation_condition: SubscriptionActivationCondition::OnStart,
                valid_until: None,
                expires_at: None,
                internal_notes: None,
                cover_image: None,
                overview: None,
                terms_and_services: None,
                net_terms: 0,
                attachments: vec![],
                pdf_document_id: None,
                sharing_key: None,
                recipients: vec![],
                auto_advance_invoices: true,
                charge_automatically: false,
                invoice_memo: None,
                invoice_threshold: None,
                create_subscription_on_acceptance: false,
                payment_methods_config: None,
                custom_properties: serde_json::Value::Null,
                entitlements: vec![],
                owner_id: None,
                template_id: None,
                display_options: Default::default(),
                sections: vec![],
                signature_level: Default::default(),
                consent_text: None,
                countersigners: vec![],
                hubspot_deal_id: None,
                max_discount_percent: None,
                allow_questions: true,
                allow_change_requests: true,
                allow_decline: true,
                plan_option: None,
            },
            vec![QuotePriceComponentNew {
                name: "Platform Fee".to_string(),
                quote_id,
                price_component_id: Some(COMP_STARTER_PLATFORM_FEE_ID),
                product_id: None,
                period,
                fee: SubscriptionFee::Rate {
                    rate: rust_decimal::Decimal::new(2900, 2),
                },
                is_override: false,
                price_id: None,
                example_usage_quantity: None,
                description: None,
                sort_order: 0,
                discount_percent: None,
                list_fee: None,
            }],
            vec![],
            vec![],
            vec![],
            Actor::System,
        )
        .await
        .expect("a quote");
    quote_id
}

fn platform_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|line| line.contains("Platform Fee"))
        .collect()
}

/// A quote's escalator, 10 % every two months twice: 29.00, then 31.90 from March, 35.09 from May.
/// Rules: R-155, R-154
#[rstest]
#[tokio::test]
async fn a_quote_escalator_bills_each_step(#[future] test_env: TestEnv) {
    use rust_decimal_macros::dec;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let phases = meteroid_store::domain::trial_offers::escalator(
        &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
        dec!(10),
        2,
        2,
    );
    let sub_id = converted_quote(&env, phases).await;
    env.run_until(day(5, 2)).await;
    assert_eq!(
        platform_lines(documents(&env, sub_id).await),
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 3190",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 3509",
        ]
    );
}

/// Two months free on a quote: nothing billed for January and February, 29.00 from March.
/// Rules: R-155
#[rstest]
#[tokio::test]
async fn a_quote_with_free_months_bills_from_their_end(#[future] test_env: TestEnv) {
    use rust_decimal_macros::dec;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let phases = meteroid_store::domain::trial_offers::free_months(
        &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
        2,
    );
    let sub_id = converted_quote(&env, phases).await;
    env.run_until(day(3, 2)).await;
    assert_eq!(
        platform_lines(documents(&env, sub_id).await),
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 0",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 0",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
        ]
    );
}

/// A quote's phases start on renewals of the fees they change: six months into a fee billed
/// yearly is refused when they are saved, and when the quote is sent.
/// Rules: R-155
#[rstest]
#[tokio::test]
async fn a_quote_phase_inside_a_yearly_period_is_refused(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::SubscriptionFeeBillingPeriod;
    use rust_decimal_macros::dec;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let quote_id = draft_quote(&env, Some(0), SubscriptionFeeBillingPeriod::Annual).await;
    let refused = env
        .services()
        .set_quote_phases(
            TENANT_ID,
            quote_id,
            meteroid_store::domain::trial_offers::free_months(
                &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
                6,
            ),
        )
        .await;
    let message = format!("{:?}", refused.expect_err("inside a year"));
    assert!(
        message.contains("billed every 12 months") && message.contains("6 months"),
        "{message}"
    );
    env.services()
        .set_quote_phases(
            TENANT_ID,
            quote_id,
            meteroid_store::domain::trial_offers::free_months(
                &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
                12,
            ),
        )
        .await
        .expect("a year free is on its renewal");
}

/// A quote's two free months with a 14-day free trial count from the billing start the trial
/// moves: billed from January 15, free until March 15, 29.00 from then. The schedule it signs
/// names no day, as the months count from there.
/// Rules: R-155, R-153
#[rstest]
#[tokio::test]
async fn a_quote_ramp_counts_from_the_billing_start_after_its_trial(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::SubscriptionFeeBillingPeriod;
    use rust_decimal_macros::dec;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let phases = meteroid_store::domain::trial_offers::free_months(
        &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
        2,
    );
    let sub_id = converted_quote_with(
        &env,
        phases,
        Some(14),
        SubscriptionFeeBillingPeriod::Monthly,
    )
    .await;
    env.run_until(day(3, 16)).await;
    assert_eq!(
        platform_lines(documents(&env, sub_id).await)
            .into_iter()
            .filter(|line| !line.ends_with(" 0") || line.contains("2024-01-15"))
            .collect::<Vec<_>>(),
        vec![
            "Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 0",
            "Recurring 2024-03-15: Platform Fee 2024-03-15..2024-04-15 2900",
        ]
    );
}

/// Starter's trial, two weeks free without limit: every subscription created on it asking no trial
/// gets one, the same customer's second too; one asking for no trial bills from its start.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_plans_trial_offer_gives_every_subscription_its_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let plan_trial = offer(&env, TrialEndAction::Convert, TrialEligibility::None).await;
    env.services()
        .set_plan_trial_offer(Actor::System, TENANT_ID, PLAN_STARTER_ID, Some(plan_trial))
        .await
        .expect("Starter's trial");
    let starter = || {
        subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(day(1, 1))
            .on_start()
    };
    let first = starter().create(env.services()).await;
    let second = starter().create(env.services()).await;
    let untrialed = starter().no_trial().create(env.services()).await;
    env.run_until(day(1, 16)).await;
    let first_invoice = |lines: Vec<String>| {
        lines
            .into_iter()
            .find(|line| line.contains("Platform Fee") && !line.ends_with(" 0"))
    };
    for sub in [first, second] {
        assert_eq!(
            first_invoice(documents(&env, sub).await).as_deref(),
            Some("Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 2900")
        );
    }
    assert_eq!(
        first_invoice(documents(&env, untrialed).await).as_deref(),
        Some("Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900")
    );
}

/// A reverse trial: two weeks of Pro on a Starter subscription, through the trial plan entitlements
/// resolve (ADR 0011 §3.9): Pro's feature is on during the trial and off after it. The trial plan is
/// the trial fact's: the offer edited since gives none, yet the running trial keeps Pro.
/// Rules: R-153, R-100
#[rstest]
#[tokio::test]
async fn a_reverse_trial_gives_the_offers_plan_and_only_it(#[future] test_env: TestEnv) {
    use common_domain::ids::{AliasOr, EntitlementEntityId};
    use meteroid_store::domain::entitlement_checks::CheckRequest;
    use meteroid_store::domain::entitlement_fold::GrantValue;
    use meteroid_store::domain::entitlements::{
        EntitlementNew, EntitlementSpec, FeatureKind, FeatureNew,
    };
    use meteroid_store::domain::subscription_trial::EffectivePlanSource;
    use meteroid_store::repositories::entitlements::EntitlementsInterface;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let feature = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "Audit log".into(),
                code: "audit_log".into(),
                description: None,
                kind: FeatureKind::Boolean,
                unit: None,
                level_type_id: None,
                policy: None,
                default: None,
            },
        )
        .await
        .expect("a feature no plan gives but Pro");
    env.store()
        .create_entitlement(
            &Actor::System,
            EntitlementNew {
                tenant_id: TENANT_ID,
                entity: EntitlementEntityId::Plan(PLAN_PRO_ID),
                resource_id: None,
                spec: EntitlementSpec::new(feature.id, GrantValue::Boolean(true)),
            },
        )
        .await
        .expect("Pro turns it on");
    let reverse = env
        .services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "Two weeks of Pro".to_string(),
                code: None,
                duration_days: 14,
                free: true,
                trial_plan_id: Some(PLAN_PRO_ID),
                end_action: TrialEndAction::Convert,
                eligibility: TrialEligibility::None,
            },
        )
        .await
        .expect("a reverse trial")
        .id;
    let reversed = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_offer(reverse)
        .create(env.services())
        .await;
    let enabled = |customer| {
        let env = &env;
        async move {
            env.store()
                .check_entitlement(
                    TENANT_ID,
                    customer,
                    AliasOr::Id(feature.id),
                    CheckRequest::default(),
                )
                .await
                .expect("checked")
                .allowed
        }
    };
    let effective = |sub| {
        let env = &env;
        async move {
            let mut conn = env.conn().await;
            env.services()
                .get_subscription_effective_plan(&mut conn, TENANT_ID, sub)
                .await
                .expect("the plan in effect")
        }
    };

    let pro = effective(reversed).await;
    assert_eq!(
        (pro.plan_id, pro.source),
        (PLAN_PRO_ID, EffectivePlanSource::TrialingPlan)
    );
    assert!(
        enabled(CUST_UBER_ID).await,
        "Pro's feature during the trial"
    );
    env.services()
        .update_trial_offer(
            TENANT_ID,
            reverse,
            meteroid_store::domain::trials::TrialOfferPatch {
                trial_plan_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("the offer gives no plan from now on");
    assert_eq!(
        effective(reversed).await.plan_id,
        PLAN_PRO_ID,
        "the running trial keeps the plan it started with"
    );

    env.run_until(day(1, 16)).await;
    assert_eq!(effective(reversed).await.plan_id, PLAN_STARTER_ID);
    assert!(!enabled(CUST_UBER_ID).await, "off once the trial ended");
}

/// An archived offer is no plan's trial any more: new subscriptions on the plan get none and are
/// still created.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn an_archived_offer_leaves_its_plans_without_a_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let plan_trial = offer(&env, TrialEndAction::Convert, TrialEligibility::None).await;
    env.services()
        .set_plan_trial_offer(Actor::System, TENANT_ID, PLAN_STARTER_ID, Some(plan_trial))
        .await
        .expect("Starter's trial");
    let plans = env
        .services()
        .archive_trial_offer(TENANT_ID, plan_trial)
        .await
        .expect("archived");
    assert_eq!(plans, vec![PLAN_STARTER_ID]);
    let sub = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .create(env.services())
        .await;
    env.run_until(day(1, 2)).await;
    assert!(
        documents(&env, sub)
            .await
            .iter()
            .any(|line| line.ends_with("Platform Fee 2024-01-01..2024-02-01 2900")),
        "billed from its start"
    );
}

/// The trial a plan's screens and pricing page show, as Starter's overview and card read it.
async fn shown_trial(env: &TestEnv) -> (Option<(u32, bool)>, Option<u32>) {
    use meteroid_store::domain::pricing_pages::PricingPageRequest;
    use meteroid_store::repositories::{PlansInterface, PricingPagesInterface};
    let overview = env
        .store()
        .get_plan_overview(PLAN_STARTER_ID, TENANT_ID)
        .await
        .expect("Starter");
    let page = env
        .store()
        .pricing_page(TENANT_ID, PRODUCT_FAMILY_ID, PricingPageRequest::default())
        .await
        .expect("a pricing page");
    let card = page
        .cards
        .iter()
        .find(|card| card.plan_name == "Starter")
        .expect("Starter's card");
    (
        overview
            .trial
            .map(|trial| (trial.offer.duration_days, trial.offer.free)),
        card.trial_days,
    )
}

/// What the plan says it gives a new subscriber is the trial it gives, and an archived offer is no
/// longer shown.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_plans_trial_is_the_trial_its_screens_show(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::PlansInterface;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    env.store()
        .set_self_serve_ladder(
            Actor::System,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![PLAN_STARTER_ID],
        )
        .await
        .expect("a ladder");
    assert_eq!(shown_trial(&env).await, (None, None), "no trial yet");

    let plan_trial = offer(&env, TrialEndAction::Convert, TrialEligibility::None).await;
    env.services()
        .set_plan_trial_offer(Actor::System, TENANT_ID, PLAN_STARTER_ID, Some(plan_trial))
        .await
        .expect("Starter's trial");
    assert_eq!(shown_trial(&env).await, (Some((14, true)), Some(14)));
    assert_eq!(
        env.store()
            .plan_trial(TENANT_ID, PLAN_STARTER_ID)
            .await
            .expect("its trial")
            .map(|trial| trial.offer.id),
        Some(plan_trial)
    );

    env.services()
        .archive_trial_offer(TENANT_ID, plan_trial)
        .await
        .expect("archived");
    assert_eq!(shown_trial(&env).await, (None, None), "none again");
}

/// A free trial extended by a week bills from its new end; ended early, it bills from today; an
/// offer edited gives its new length to redemptions to come only, the trials it started keep
/// theirs. Starter is 29.00 + Seats 10.00 a month in advance.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_trial_is_extended_or_ended_and_an_edited_offer_holds_for_what_comes(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::trials::{SubscriptionTrial, TrialEnd, TrialMove, TrialOfferPatch};
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let trial = offer(&env, TrialEndAction::Convert, TrialEligibility::None).await;
    let starter = || {
        subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(day(1, 1))
            .on_start()
            .trial_offer(trial)
    };
    let extended = starter().create(env.services()).await;
    let ended = starter().create(env.services()).await;
    let kept = starter().create(env.services()).await;
    env.services()
        .update_trial_offer(
            TENANT_ID,
            trial,
            TrialOfferPatch {
                duration_days: Some(21),
                ..TrialOfferPatch::default()
            },
        )
        .await
        .expect("three weeks from now on");
    let longer = starter().create(env.services()).await;

    env.set_now(at(1, 5, 12));
    let moved = env
        .services()
        .move_trial(Actor::System, TENANT_ID, extended, TrialMove::Days(7))
        .await
        .expect("a week more");
    assert_eq!(
        moved,
        SubscriptionTrial {
            from: day(1, 1),
            until: day(1, 22),
            free: true,
            trial_plan_id: None,
            offer_id: Some(trial),
            end_action: TrialEnd::Convert,
        }
    );
    env.services()
        .move_trial(Actor::System, TENANT_ID, ended, TrialMove::Now)
        .await
        .expect("billed from today");
    let shorter = env
        .services()
        .move_trial(Actor::System, TENANT_ID, kept, TrialMove::Until(day(1, 10)))
        .await;
    assert!(
        format!("{:?}", shorter.expect_err("an extension ends later")).contains("after the trial"),
    );

    env.run_until(day(1, 23)).await;
    let first = |lines: Vec<String>| {
        lines
            .into_iter()
            .find(|line| line.contains("Platform Fee") && !line.ends_with(" 0"))
    };
    for (sub, expected) in [
        (
            extended,
            "Recurring 2024-01-22: Platform Fee 2024-01-22..2024-02-22 2900",
        ),
        (
            ended,
            "Recurring 2024-01-05: Platform Fee 2024-01-05..2024-02-05 2900",
        ),
        (
            kept,
            "Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 2900",
        ),
        (
            longer,
            "Recurring 2024-01-22: Platform Fee 2024-01-22..2024-02-22 2900",
        ),
    ] {
        assert_eq!(first(documents(&env, sub).await).as_deref(), Some(expected));
    }
}

/// A trial cancelling at its end, extended, cancels on its new end (its end date, no term); it is
/// not ended early.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_cancelling_trial_extended_cancels_later_and_is_not_ended_early(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::trials::TrialMove;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let ends = offer(&env, TrialEndAction::Cancel, TrialEligibility::None).await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_offer(ends)
        .create(env.services())
        .await;
    env.set_now(at(1, 5, 12));
    let early = env
        .services()
        .move_trial(Actor::System, TENANT_ID, sub_id, TrialMove::Now)
        .await;
    assert!(format!("{:?}", early.expect_err("not ended early")).contains("cancelling"));
    env.services()
        .move_trial(Actor::System, TENANT_ID, sub_id, TrialMove::Days(7))
        .await
        .expect("a week more");
    assert_eq!(
        env.get_subscription(sub_id).await.end_date,
        Some(day(1, 22))
    );
}

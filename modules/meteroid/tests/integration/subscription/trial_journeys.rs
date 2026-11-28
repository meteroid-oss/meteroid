//! A trial along the customer's journey (R-153, R-179, R-167): a checkout session keeps the trial
//! it was sent, a checkout starts a trial asking no card without one, a quote decides at the end,
//! lists narrow to trials ending or awaiting, the portal reads what follows, and a sign-up whose
//! trial asks a card goes through checkout. Starter is 29.00 + Seats 10.00 a month in advance.

use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId};
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::trials::{
    TrialEligibility, TrialEnd, TrialEndAction, TrialLength, TrialListFilter, TrialOfferNew,
    TrialRequest, TrialStatus, TrialTerms, TrialWithoutCard,
};
use meteroid_store::domain::{CreateCheckoutSession, PaginationRequest};
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::subscriptions::{
    SubscriptionInterface, SubscriptionInterfaceAuto,
};
use rstest::rstest;

use super::phases::{at, day};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

fn session(
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
    trial: TrialRequest,
) -> CreateCheckoutSession {
    CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id,
        plan_version_id,
        option: None,
        billing_start_date: None,
        billing_day_anchor: None,
        net_terms: None,
        offer: Default::default(),
        trial,
        end_date: None,
        auto_advance_invoices: true,
        charge_automatically: true,
        invoice_memo: None,
        invoice_threshold: None,
        purchase_order: None,
        payment_methods_config: None,
        components: None,
        add_ons: None,
        coupon_code: None,
        coupon_ids: vec![],
        expires_in_hours: Some(24),
        metadata: None,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: None,
        cancel_url: None,
        resource_id: None,
    }
}

/// Starter's trial: 14 days, its card `card_required`.
async fn starter_trial(env: &TestEnv, card_required: bool) {
    env.store()
        .set_plan_trial(
            Actor::System,
            TENANT_ID,
            PLAN_STARTER_ID,
            Some(TrialTerms {
                card_required: Some(card_required),
                ..TrialTerms::days(14, None)
            }),
        )
        .await
        .expect("Starter's trial");
}

fn page() -> PaginationRequest {
    PaginationRequest {
        page: 0,
        per_page: Some(100),
    }
}

/// Every trial a session is sent reads back as sent: the plan's asked or by default, none, an
/// offer, days or an end date, and whose features.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_checkout_session_reads_back_the_trial_it_was_sent(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let offer = env
        .services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "A week".to_string(),
                code: None,
                duration_days: 7,
                free: true,
                trial_plan_id: None,
                card_required: true,
                end_action: TrialEndAction::Convert,
                without_card: TrialWithoutCard::AwaitPayment,
                eligibility: TrialEligibility::None,
            },
        )
        .await
        .expect("an offer")
        .id;
    let sent = vec![
        TrialRequest::Default,
        TrialRequest::Plan,
        TrialRequest::None,
        TrialRequest::Offer(offer),
        TrialRequest::Custom {
            length: TrialLength::Days(10),
            free: None,
            trial_plan: Some(None),
            manual: false,
        },
        TrialRequest::Custom {
            length: TrialLength::Until(day(3, 1)),
            free: None,
            trial_plan: Some(Some(PLAN_ENTERPRISE_ID)),
            manual: false,
        },
        TrialRequest::Custom {
            length: TrialLength::Days(5),
            free: None,
            trial_plan: None,
            manual: false,
        },
    ];
    for trial in sent {
        let created = env
            .store()
            .create_checkout_session(session(
                CUST_UBER_ID,
                PLAN_VERSION_STARTER_ID,
                trial.clone(),
            ))
            .await
            .expect("a session");
        assert_eq!(created.trial_request(), trial);
        let read = env
            .store()
            .get_checkout_session(TENANT_ID, created.id)
            .await
            .expect("read");
        assert_eq!(read.trial_request(), trial);
    }
}

/// A plan whose trial asks no card starts it from checkout without one; asking a card, or with
/// no trial, the checkout is refused without a payment method.
/// Rules: R-153, R-179
#[rstest]
#[tokio::test]
async fn a_checkout_starts_a_trial_asking_no_card_without_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    starter_trial(&env, false).await;
    let open = env
        .store()
        .create_checkout_session(session(
            CUST_UBER_ID,
            PLAN_VERSION_STARTER_ID,
            TrialRequest::Default,
        ))
        .await
        .expect("a session");
    let completed = env
        .services()
        .complete_checkout_without_payment_method(TENANT_ID, open.id, "EUR".to_string(), None)
        .await
        .expect("started without a card");
    let CheckoutCompletionResult::Completed {
        subscription_id,
        transaction,
    } = completed
    else {
        panic!("completed: {completed:?}");
    };
    assert!(transaction.is_none(), "nothing charged");
    assert_eq!(
        env.get_subscription(subscription_id).await.status,
        SubscriptionStatusEnum::TrialActive
    );
    let (trial, _) = env
        .services()
        .subscription_trial(TENANT_ID, subscription_id)
        .await
        .expect("read")
        .expect("a trial");
    assert_eq!(trial.days(), 14);

    let none = env
        .store()
        .create_checkout_session(session(
            CUST_SPOTIFY_ID,
            PLAN_VERSION_STARTER_ID,
            TrialRequest::None,
        ))
        .await
        .expect("a session");
    let _ = env
        .services()
        .complete_checkout_without_payment_method(TENANT_ID, none.id, "EUR".to_string(), None)
        .await
        .expect_err("no trial: a payment method");

    starter_trial(&env, true).await;
    let card = env
        .store()
        .create_checkout_session(session(
            CUST_SPOTIFY_ID,
            PLAN_VERSION_STARTER_ID,
            TrialRequest::Default,
        ))
        .await
        .expect("a session");
    let _ = env
        .services()
        .complete_checkout_without_payment_method(TENANT_ID, card.id, "EUR".to_string(), None)
        .await
        .expect_err("the trial asks a card");
}

/// A quote deciding at the end converts to a trial whose end waits for an operator.
/// Rules: R-155, R-167
#[rstest]
#[tokio::test]
async fn a_quote_deciding_at_the_end_converts_to_a_manual_trial(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    use meteroid_store::repositories::quotes::QuotesInterface;
    let env = test_env.await;
    let quote_id = super::trial_offers::draft_quote(
        &env,
        Some(14),
        meteroid_store::domain::enums::SubscriptionFeeBillingPeriod::Monthly,
    )
    .await;
    diesel::sql_query("UPDATE quote SET trial_end_action = 'MANUAL' WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(quote_id)
        .execute(&mut env.conn().await)
        .await
        .expect("decided at the end");
    let quote = env
        .store()
        .get_quote_by_id(TENANT_ID, quote_id)
        .await
        .expect("read");
    assert_eq!(
        quote.trial_end_action,
        meteroid_store::domain::quotes::QuoteTrialEnd::Manual
    );
    env.store()
        .publish_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("published");
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
    let (trial, _) = env
        .services()
        .subscription_trial(TENANT_ID, sub_id)
        .await
        .expect("read")
        .expect("a trial");
    assert_eq!(trial.end_action, TrialEnd::Manual);
    assert_eq!(trial.days(), 14);
}

/// A trial asking no card, charged automatically through the mock card provider, the customer
/// holding no payment method: listed as ending within 7 days from January 10 (not 3), its end,
/// status and missing method read beside it; then awaiting payment from January 15, its opening
/// invoice the one the portal pays.
/// Rules: R-179, R-167
#[rstest]
#[tokio::test]
async fn trials_ending_and_awaiting_payment_narrow_the_list(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    env.set_now(at(1, 1, 12));
    starter_trial(&env, false).await;
    let trialing = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .auto_charge()
        .create(env.services())
        .await;
    let plain = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_until(day(1, 10)).await;

    let listed = |trial: TrialListFilter| {
        let env = &env;
        async move {
            env.store()
                .list_subscriptions_by_trial(
                    TENANT_ID,
                    None,
                    None,
                    None,
                    Some(trial),
                    None,
                    vec![],
                    None,
                    page(),
                )
                .await
                .expect("listed")
                .items
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<SubscriptionId>>()
        }
    };
    let ending = listed(TrialListFilter::EndingWithinDays(7)).await;
    assert!(ending.contains(&trialing), "{ending:?}");
    assert!(!ending.contains(&plain));
    assert!(
        !listed(TrialListFilter::EndingWithinDays(3))
            .await
            .contains(&trialing)
    );
    assert!(
        !listed(TrialListFilter::AwaitingPayment)
            .await
            .contains(&trialing)
    );
    let summaries = env
        .store()
        .list_trial_summaries(TENANT_ID, &[trialing, plain])
        .await
        .expect("summaries");
    let summary = summaries.get(&trialing).expect("its trial");
    assert_eq!(
        (summary.ends_on, summary.status, summary.has_payment_method),
        (day(1, 15), TrialStatus::Active, Some(false))
    );
    assert!(!summaries.contains_key(&plain));

    let details = env
        .store()
        .get_subscription_details(TENANT_ID, trialing)
        .await
        .expect("details");
    let running = env
        .services()
        .customer_trial(TENANT_ID, &details)
        .await
        .expect("read")
        .expect("a trial");
    assert_eq!(running.status, TrialStatus::Active);
    assert!(!running.has_payment_method);
    let after = running.after_trial.expect("what follows");
    assert_eq!(after.starts_on, day(1, 15));
    assert_eq!(
        after
            .recurring
            .iter()
            .map(|entry| (entry.amount, entry.period))
            .collect::<Vec<_>>(),
        vec![(
            3900,
            meteroid_store::domain::enums::BillingPeriodEnum::Monthly
        )]
    );
    assert!(running.invoice_to_pay.is_none());

    env.run_until(day(1, 18)).await;
    let awaiting = listed(TrialListFilter::AwaitingPayment).await;
    assert!(awaiting.contains(&trialing), "{awaiting:?}");
    assert!(
        !listed(TrialListFilter::EndingWithinDays(7))
            .await
            .contains(&trialing)
    );
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, trialing)
        .await
        .expect("details");
    let to_pay = env
        .services()
        .customer_trial(TENANT_ID, &details)
        .await
        .expect("read")
        .expect("a trial");
    assert_eq!(to_pay.status, TrialStatus::AwaitingPayment);
    assert!(to_pay.after_trial.is_none());
    let invoice = to_pay.invoice_to_pay.expect("its opening invoice");
    assert!(invoice.amount_due >= 3900, "{invoice:?}");
}

/// A sign-up whose plan's trial asks a card, its entity taking cards, goes through checkout; once
/// the trial asks none, the next sign-up starts on it.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_signup_on_a_trial_asking_a_card_goes_through_checkout(#[future] test_env: TestEnv) {
    use meteroid_store::domain::SubscriptionActivationCondition;
    use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
    use meteroid_store::repositories::ProductFamilyJourneysInterface;
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    starter_trial(&env, true).await;
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                product_family_id: PRODUCT_FAMILY_ID,
                signup_plan_id: Some(PLAN_STARTER_ID),
                signup_option: None,
                period_switch: true,
                self_serve_downgrade: false,
            },
        )
        .await
        .expect("the sign-up plan");
    let signed_up = |alias: &'static str| {
        let env = &env;
        async move {
            let offer = env
                .services()
                .signup_offer(TENANT_ID, None, "EUR")
                .await
                .expect("the sign-up offer");
            let customer = env
                .services()
                .create_customer_on_signup(
                    Actor::System,
                    TENANT_ID,
                    super::signup_eligibility::signing_up(alias, "PT"),
                    offer,
                )
                .await
                .expect("signed up");
            env.store()
                .list_subscriptions(
                    TENANT_ID,
                    Some(customer.id),
                    None,
                    None,
                    None,
                    vec![],
                    None,
                    page(),
                )
                .await
                .expect("listed")
                .items
                .pop()
                .expect("its subscription")
        }
    };
    let by_checkout = signed_up("signup-card").await;
    assert!(
        matches!(
            by_checkout.activation_condition,
            SubscriptionActivationCondition::OnCheckout
        ),
        "{by_checkout:?}"
    );

    starter_trial(&env, false).await;
    let started = signed_up("signup-no-card").await;
    assert!(
        matches!(
            started.activation_condition,
            SubscriptionActivationCondition::OnStart
        ),
        "{started:?}"
    );
}

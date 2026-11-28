//! A plan's trial switching plans in the open-source edition: to a free plan only; a paid plan
//! needs the Enterprise `subscription_phases` gate.
use meteroid::module::Modules;
use meteroid_grpc::meteroid::api::plans::v1::{
    SetPlanTrialRequest, TrialAtEnd, TrialTerms, TrialWithoutCard, trial_at_end, trial_without_card,
};
use tonic::Code;

use crate::data::ids::{PLAN_STARTER_ID, PLAN_VERSION_FREE_ID, PLAN_VERSION_NOTION_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;

fn terms(at_end: TrialAtEnd, without_card: TrialWithoutCard) -> SetPlanTrialRequest {
    SetPlanTrialRequest {
        plan_id: PLAN_STARTER_ID.as_proto(),
        trial: Some(TrialTerms {
            duration_days: 14,
            trial_plan_id: None,
            card_required: true,
            at_end: Some(at_end),
            without_card: Some(without_card),
        }),
    }
}

fn switch_at_end(to: common_domain::ids::PlanVersionId) -> TrialAtEnd {
    TrialAtEnd {
        kind: trial_at_end::Kind::Switch as i32,
        plan_version_id: Some(to.as_proto()),
    }
}

fn convert() -> TrialAtEnd {
    TrialAtEnd {
        kind: trial_at_end::Kind::Convert as i32,
        plan_version_id: None,
    }
}

fn switch_without_card(to: common_domain::ids::PlanVersionId) -> TrialWithoutCard {
    TrialWithoutCard {
        kind: trial_without_card::Kind::Switch as i32,
        plan_version_id: Some(to.as_proto()),
    }
}

fn await_payment() -> TrialWithoutCard {
    TrialWithoutCard {
        kind: trial_without_card::Kind::AwaitPayment as i32,
        plan_version_id: None,
    }
}

/// Switching to Free at the trial's end or without a card is Core; to Notion (paid), at the end
/// or without a card, is refused as not in the plan, and the trial is left as it was.
#[tokio::test]
async fn a_trial_switches_to_a_paid_plan_only_with_the_phases_gate() {
    helpers::init::logging();
    let modules = Modules::core();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let instance = modules.edition.instance();
    let setup = meteroid_it::container::start_meteroid_with_modules(
        database,
        SeedLevel::PLANS,
        instance,
        modules,
    )
    .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let mut plans = clients.plans.clone();

    let free = plans
        .set_plan_trial(terms(
            switch_at_end(PLAN_VERSION_FREE_ID),
            switch_without_card(PLAN_VERSION_FREE_ID),
        ))
        .await
        .expect("a free plan is Core")
        .into_inner()
        .trial
        .expect("its trial");

    let paid_at_end = plans
        .set_plan_trial(terms(
            switch_at_end(PLAN_VERSION_NOTION_ID),
            await_payment(),
        ))
        .await
        .expect_err("a paid plan at the end");
    assert_eq!(
        paid_at_end.code(),
        Code::PermissionDenied,
        "{paid_at_end:?}"
    );
    let paid_without_card = plans
        .set_plan_trial(terms(
            convert(),
            switch_without_card(PLAN_VERSION_NOTION_ID),
        ))
        .await
        .expect_err("a paid plan without a card");
    assert_eq!(
        paid_without_card.code(),
        Code::PermissionDenied,
        "{paid_without_card:?}"
    );

    // Its days edited, the switches to Free kept.
    let kept = plans
        .set_plan_trial(SetPlanTrialRequest {
            trial: Some(TrialTerms {
                duration_days: 21,
                ..free.terms.clone().expect("its terms")
            }),
            ..terms(convert(), await_payment())
        })
        .await
        .expect("an edit keeping its switches")
        .into_inner()
        .trial
        .expect("its trial");
    assert_eq!(kept.terms.expect("its terms").duration_days, 21);
}

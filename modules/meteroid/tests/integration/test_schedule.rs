use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_grpc::meteroid::common::v1 as common;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_schedules_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let plan_version_id = "018c344a-78a9-7e2b-af90-5748672711f9";

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    let schedules = clients
        .schedules
        .clone()
        .list_schedules(api::schedules::v1::ListSchedulesRequests {
            plan_version_id: plan_version_id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .schedules;

    assert_eq!(schedules.len(), 0);

    let ramps = api::schedules::v1::PlanRamps {
        ramps: vec![api::schedules::v1::plan_ramps::PlanRamp {
            index: 0,
            duration_in_months: None,
            ramp_adjustment: Some(
                api::schedules::v1::plan_ramps::plan_ramp::PlanRampAdjustment {
                    minimum: Some(api::adjustments::v1::discount::Amount {
                        value_in_cents: 157,
                    }),
                    discount: Some(api::adjustments::v1::StandardDiscount {
                        discount_type: Some(
                            api::adjustments::v1::standard_discount::DiscountType::Percent(
                                api::adjustments::v1::discount::Percent {
                                    percentage: Some(common::Decimal {
                                        value: "25.18".into(),
                                    }),
                                },
                            ),
                        ),
                    }),
                },
            ),
        }],
    };

    // create schedule
    let created = clients
        .schedules
        .clone()
        .create_schedule(api::schedules::v1::CreateScheduleRequest {
            plan_version_id: plan_version_id.to_string(),
            billing_period: api::shared::v1::BillingPeriod::Monthly as i32,
            ramps: Some(ramps.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .schedule
        .unwrap();

    assert_eq!(created.term, api::shared::v1::BillingPeriod::Monthly as i32);
    assert_eq!(created.name, "".to_string());
    assert_eq!(created.ramps.as_ref(), Some(&ramps));

    // list schedules
    let schedules = clients
        .schedules
        .clone()
        .list_schedules(api::schedules::v1::ListSchedulesRequests {
            plan_version_id: plan_version_id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .schedules;

    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules.first(), Some(&created));

    // edit schedule
    let updated_schedule = api::schedules::v1::Schedule {
        ramps: Some(api::schedules::v1::PlanRamps {
            ramps: vec![api::schedules::v1::plan_ramps::PlanRamp {
                index: 1,
                duration_in_months: Some(3),
                ramp_adjustment: Some(
                    api::schedules::v1::plan_ramps::plan_ramp::PlanRampAdjustment {
                        minimum: Some(api::adjustments::v1::discount::Amount {
                            value_in_cents: 197,
                        }),
                        discount: Some(api::adjustments::v1::StandardDiscount {
                            discount_type: Some(
                                api::adjustments::v1::standard_discount::DiscountType::Amount(
                                    api::adjustments::v1::discount::Amount {
                                        value_in_cents: 300,
                                    },
                                ),
                            ),
                        }),
                    },
                ),
            }],
        }),
        ..created.clone()
    };

    let edited = clients
        .schedules
        .clone()
        .edit_schedule(api::schedules::v1::EditScheduleRequest {
            plan_version_id: plan_version_id.to_string(),
            schedule: Some(updated_schedule.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .schedule
        .unwrap();

    assert_eq!(&edited, &updated_schedule);

    // remove schedule
    let resp = clients
        .schedules
        .clone()
        .remove_schedule(api::schedules::v1::RemoveScheduleRequest {
            schedule_id: edited.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(&resp, &api::schedules::v1::EmptyResponse {});

    let schedules = clients
        .schedules
        .clone()
        .list_schedules(api::schedules::v1::ListSchedulesRequests {
            plan_version_id: plan_version_id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .schedules;

    assert_eq!(schedules.len(), 0);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

use crate::helpers;
use chrono::{Datelike, Months, NaiveDate, NaiveDateTime};
use meteroid::api::shared::conversions::ProtoConv;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::error::Error;
use std::str::FromStr;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tonic::Code;

use meteroid_grpc::meteroid::api;

use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use meteroid_grpc::meteroid::api::subscriptions::v1::cancel_subscription_request::EffectiveAt;
use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionStatus;

use meteroid_store::domain::{CursorPaginationRequest, LineItem};
use meteroid_store::repositories::subscriptions::SubscriptionSlotsInterface;
use meteroid_store::repositories::InvoiceInterface;

struct TestContext {
    setup: MeteroidSetup,
    clients: AllClients,
    _container: ContainerAsync<Postgres>,
}

async fn setup_test(seed_level: SeedLevel) -> Result<TestContext, Box<dyn Error>> {
    helpers::init::logging();
    let (_container, postgres_connection_string) = meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, seed_level).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    Ok(TestContext {
        setup,
        clients,
        _container,
    })
}

#[tokio::test]
#[ignore] // subscription seed is broken
async fn test_subscription_create() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    let tenant_id = "018c2c82-3df1-7e84-9e05-6e141d0e751a".to_string();
    let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
    let plan_version_id = "018c344b-da87-7392-bbae-c5c8780adb1b".to_string();
    let component_id = "018c344c-9ec9-7608-b115-1537b6985e73".to_string();

    let now = chrono::offset::Local::now().date_naive();

    let subscription = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: now.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                initial_slot_count: Some(10),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    // it should fail if a parameter is missing // TODO actually maybe it'll default to minimal slots no ?
    let res = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: now.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                initial_slot_count: None,
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await;

    let err = res.err().unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);

    let res = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: now.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: None,
                                initial_slot_count: Some(10),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await;

    let err = res.err().unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);

    let result_subscription = clients
        .subscriptions
        .clone()
        .get_subscription_details(tonic::Request::new(
            api::subscriptions::v1::GetSubscriptionDetailsRequest {
                subscription_id: subscription.subscription.clone().unwrap().id.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // check DB state
    assert_eq!(
        result_subscription.customer_id.clone().to_string(),
        customer_id.clone()
    );
    assert_eq!(
        result_subscription.plan_version_id.to_string(),
        plan_version_id
    );

    let db_invoices = setup
        .store
        .list_invoices_to_issue(
            1,
            CursorPaginationRequest {
                limit: Some(1000),
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(db_invoices.len(), 1);

    let db_invoice = db_invoices.get(0).unwrap();

    assert_eq!(db_invoice.tenant_id.to_string(), tenant_id);
    assert_eq!(db_invoice.customer_id.clone().to_string(), customer_id);
    assert_eq!(
        db_invoice.subscription_id.map(|x| x.to_string()),
        subscription.subscription.clone().map(|x| x.id)
    );

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

#[tokio::test]
#[ignore] // subscription seed is broken
async fn test_subscription_cancel() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();
    let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
    let plan_version_id = "018c344b-da87-7392-bbae-c5c8780adb1b".to_string();
    let component_id = "018c344c-9ec9-7608-b115-1537b6985e73".to_string();

    let now = chrono::offset::Local::now().date_naive();

    let subscription = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: now.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                initial_slot_count: Some(10),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                committed_capacity: None,
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let result_subscription = clients
        .subscriptions
        .clone()
        .cancel_subscription(tonic::Request::new(
            api::subscriptions::v1::CancelSubscriptionRequest {
                subscription_id: subscription.subscription.clone().unwrap().id.clone(),
                reason: Some("test".to_string()),
                effective_at: EffectiveAt::BillingPeriodEnd as i32,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .subscription
        .unwrap();

    // check DB state
    assert_eq!(result_subscription.status(), SubscriptionStatus::Pending);
    assert!(result_subscription.canceled_at.is_some());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

// TODO Commenting this test while we complete the slot flow (cf sequence diagram in the store impl)
// #[tokio::test]
// async fn test_slot_subscription_upgrade_downgrade() {
//     let docker = Cli::default();
//     let TestContext {
//         setup,
//         clients,
//         _container,
//     } = setup_test(&docker, SeedLevel::PLANS).await.unwrap();
//
//     let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
//     let plan_version_id = "018c344b-da87-7392-bbae-c5c8780adb1b".to_string();
//     let component_id = "018c344c-9ec9-7608-b115-1537b6985e73".to_string();
//
//     fn now() -> NaiveDateTime {
//         chrono::offset::Local::now().naive_utc()
//     }
//
//     let start = now().date();
//
//     let seats_quantity = 15;
//
//     let subscription = clients
//         .subscriptions
//         .clone()
//         .create_subscription(tonic::Request::new(
//             api::subscriptions::v1::CreateSubscriptionRequest {
//                 subscription: Some(api::subscriptions::v1::CreateSubscription {
//                     plan_version_id: plan_version_id.clone(),
//                     billing_start_date: start.as_proto(),
//                     billing_day: start.day(),
//                     customer_id: customer_id.clone(),
//                     currency: "USD".to_string(),
//                     components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
//                         parameterized_components: vec![
//                             api::subscriptions::v1::create_subscription_components::ComponentParameterization {
//                                 component_id: component_id.clone(),
//                                 initial_slot_count: Some(seats_quantity),
//                                 billing_period: Some(BillingPeriod::Monthly.into()),
//                                 committed_capacity: None,
//                             }
//                         ],
//                         ..Default::default()
//                     }),
//                     ..Default::default()
//                 })
//             },
//         ))
//         .await
//         .unwrap()
//         .into_inner();
//
//     let subscription_id =
//         uuid::Uuid::parse_str(subscription.subscription.map(|s| s.id).unwrap().as_str()).unwrap();
//     let price_component_id = uuid::Uuid::parse_str(component_id.as_str()).unwrap();
//
//     let db_invoices = meteroid_it::db::invoice::all(&setup.pool).await;
//
//     let sub_invoice_id = db_invoices.get(0).unwrap().id;
//
//     let current_active_seats = meteroid_it::db::slot_transaction::get_active_slots(
//         &setup.pool,
//         subscription_id.clone(),
//         price_component_id.clone(),
//         chrono_to_datetime(now()).unwrap(),
//     )
//         .await;
//
//     assert_eq!(current_active_seats, seats_quantity as i32);
//
//     // downgrade -6
//     let slots = clients
//         .subscriptions
//         .clone()
//         .apply_slots_delta(tonic::Request::new(
//             api::subscriptions::v1::ApplySlotsDeltaRequest {
//                 subscription_id: subscription_id.to_string(),
//                 price_component_id: price_component_id.to_string(),
//                 delta: -6,
//             },
//         ))
//         .await
//         .unwrap()
//         .into_inner()
//         .active_slots;
//
//     assert_eq!(slots as i32, seats_quantity as i32);
//
//     let current_active_seats = meteroid_it::db::slot_transaction::get_active_slots(
//         &setup.pool,
//         subscription_id.clone(),
//         price_component_id.clone(),
//         chrono_to_datetime(now()).unwrap(),
//     )
//         .await;
//
//     assert_eq!(current_active_seats, seats_quantity as i32);
//
//     // downgrade -10 should fail
//     let slots = clients
//         .subscriptions
//         .clone()
//         .apply_slots_delta(tonic::Request::new(
//             api::subscriptions::v1::ApplySlotsDeltaRequest {
//                 subscription_id: subscription_id.to_string(),
//                 price_component_id: price_component_id.to_string(),
//                 delta: -10,
//             },
//         ))
//         .await;
//
//     assert!(slots.is_err());
//
//     // upgrade 5
//     let slots = clients
//         .subscriptions
//         .clone()
//         .apply_slots_delta(tonic::Request::new(
//             api::subscriptions::v1::ApplySlotsDeltaRequest {
//                 subscription_id: subscription_id.to_string(),
//                 price_component_id: price_component_id.to_string(),
//                 delta: 5,
//             },
//         ))
//         .await
//         .unwrap()
//         .into_inner()
//         .active_slots;
//
//     assert_eq!(slots as i32, seats_quantity as i32 + 5);
//
//     let current_active_seats = meteroid_it::db::slot_transaction::get_active_slots(
//         &setup.pool,
//         subscription_id.clone(),
//         price_component_id.clone(),
//         chrono_to_datetime(now()).unwrap(),
//     )
//         .await;
//
//     assert_eq!(current_active_seats, seats_quantity as i32 + 5);
//
//     let db_invoices = meteroid_it::db::invoice::all(&setup.pool)
//         .await
//         .into_iter()
//         .filter(|i| i.id != sub_invoice_id)
//         .collect::<Vec<_>>();
//
//     assert_eq!(db_invoices.len(), 1);
//
//     let db_invoice = db_invoices.get(0).unwrap();
//
//     assert_eq!(db_invoice.invoice_date, chrono_to_date(start).unwrap());
//
//     let invoice_lines: Vec<InvoiceLine> =
//         serde_json::from_value(db_invoice.line_items.clone()).unwrap();
//     assert_eq!(invoice_lines.len(), 1);
//
//     let invoice_line = invoice_lines.get(0).unwrap();
//     assert_eq!(invoice_line.name, "Seats");
//     assert_eq!(invoice_line.quantity, Some(5));
//
//     assert_eq!(invoice_line.unit_price, Some(1000f64));
//     assert_eq!(invoice_line.total, 1000 * 5);
//
//     let period = invoice_line.period.as_ref().unwrap();
//     assert_eq!(period.from, start);
//     assert_eq!(period.to, start.checked_add_months(Months::new(1)).unwrap());
// }

#[tokio::test]
#[ignore] // subscription seed is broken
async fn test_subscription_create_invoice_seats() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();
    let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
    let plan_version_id = "018c344b-da87-7392-bbae-c5c8780adb1b".to_string();
    let component_id = "018c344c-9ec9-7608-b115-1537b6985e73".to_string();

    let start = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();

    let seats_quantity = 15;

    let subscription = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 10,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                initial_slot_count: Some(seats_quantity),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                committed_capacity: None,
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let subscription_id =
        uuid::Uuid::parse_str(subscription.subscription.map(|s| s.id).unwrap().as_str()).unwrap();
    let price_component_id = uuid::Uuid::parse_str(component_id.as_str()).unwrap();

    let db_invoices = setup
        .store
        .list_invoices_to_issue(
            1,
            CursorPaginationRequest {
                limit: Some(1000),
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(db_invoices.len(), 1);

    let db_invoice = db_invoices.get(0).unwrap();

    assert_eq!(db_invoice.invoice_date, start);

    let invoice_lines: Vec<LineItem> = db_invoice.line_items.clone();
    assert_eq!(invoice_lines.len(), 1);

    let invoice_line = invoice_lines.get(0).unwrap();
    assert_eq!(invoice_line.name, "Seats");
    assert_eq!(invoice_line.quantity, Some(Decimal::from(seats_quantity)));

    // Monthly unit price (1000) * num_days (10 - 1) / total_days_in_month (31)
    let prorated_unit_price = (1000.0 * (10 - 1) as f64 / 31.0).round();
    assert_eq!(
        invoice_line.unit_price,
        Decimal::from_f64(prorated_unit_price)
    );
    assert_eq!(
        invoice_line.total,
        (prorated_unit_price * seats_quantity as f64) as i64
    );

    assert_eq!(invoice_line.start_date, start);
    assert_eq!(invoice_line.end_date, start.with_day(10).unwrap());

    let current_active_seats = setup
        .store
        .get_current_slots_value(
            db_invoice.tenant_id.clone(),
            subscription_id.clone(),
            price_component_id.clone(),
            Some(NaiveDateTime::from_str("2023-01-01T02:00:00").unwrap()),
        )
        .await
        .unwrap();

    assert_eq!(current_active_seats, seats_quantity);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

#[tokio::test]
#[ignore] // subscription seed is broken
async fn test_subscription_create_invoice_rate() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
    let plan_version_id = "018c344a-78a9-7e2b-af90-5748672711f8".to_string();
    let component_id = "018c344b-6050-7ec8-bd8c-d2e9c41ab711".to_string();

    let start = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();

    // should fail with invalid billing period
    let res = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Quarterly.into()),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await;

    assert!(res.is_err());

    // not prorated
    let sub_annual = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Annual.into()),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    // not prorated
    let sub_monthly = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 1,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let sub_monthly_prorated = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 30,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: component_id.clone(),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let db_invoices = setup
        .store
        .list_invoices_to_issue(
            1,
            CursorPaginationRequest {
                limit: Some(1000),
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(db_invoices.len(), 3);

    let db_invoice_monthly = db_invoices
        .iter()
        .find(|i| {
            i.subscription_id.unwrap().to_string() == sub_monthly.subscription.clone().unwrap().id
        })
        .unwrap();

    let invoice_lines_monthly: Vec<LineItem> = db_invoice_monthly.line_items.clone();
    assert_eq!(invoice_lines_monthly.len(), 1);
    let invoice_line_monthly = invoice_lines_monthly.get(0).unwrap();
    assert_eq!(invoice_line_monthly.name, "Subscription Rate");
    assert_eq!(invoice_line_monthly.quantity, Some(dec!(1)));
    assert_eq!(invoice_line_monthly.unit_price, Some(dec!(3500.0)));
    assert_eq!(invoice_line_monthly.total, 3500);

    assert_eq!(invoice_line_monthly.start_date, start);
    assert_eq!(
        invoice_line_monthly.end_date,
        start.checked_add_months(Months::new(1)).unwrap()
    );

    let db_invoice_annual = db_invoices
        .iter()
        .find(|i| {
            i.subscription_id.unwrap().to_string() == sub_annual.subscription.clone().unwrap().id
        })
        .unwrap();

    let invoice_lines_annual = db_invoice_annual.line_items.clone();
    assert_eq!(invoice_lines_annual.len(), 1);
    let invoice_line_annual = invoice_lines_annual.get(0).unwrap();
    assert_eq!(invoice_line_annual.name, "Subscription Rate");
    assert_eq!(invoice_line_annual.quantity, Some(dec!(1)));
    assert_eq!(invoice_line_annual.unit_price, Some(dec!(15900.0)));
    assert_eq!(invoice_line_annual.total, 15900);

    assert_eq!(invoice_line_annual.start_date, start);
    assert_eq!(
        invoice_line_annual.end_date,
        start.checked_add_months(Months::new(12)).unwrap()
    );

    // prorated
    let db_invoice_monthly = db_invoices
        .iter()
        .find(|i| {
            i.subscription_id.unwrap().to_string()
                == sub_monthly_prorated.subscription.clone().unwrap().id
        })
        .unwrap();

    let invoice_lines_monthly = db_invoice_monthly.line_items.clone();
    assert_eq!(invoice_lines_monthly.len(), 1);
    let invoice_line_monthly = invoice_lines_monthly.get(0).unwrap();
    assert_eq!(invoice_line_monthly.name, "Subscription Rate");
    assert_eq!(invoice_line_monthly.quantity, Some(dec!(1)));

    let prorated_unit_price: i64 = (3500.0 * (30 - 1) as f64 / 31.0).round() as i64;

    assert_eq!(
        invoice_line_monthly.unit_price,
        Some(Decimal::from(prorated_unit_price))
    );
    assert_eq!(invoice_line_monthly.total, prorated_unit_price);

    assert_eq!(invoice_line_monthly.start_date, start);
    assert_eq!(invoice_line_monthly.end_date, start.with_day(30).unwrap());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

#[tokio::test]
#[ignore] // subscription seed is broken
async fn test_subscription_create_invoice_usage() {
    let TestContext {
        setup,
        clients,
        _container,
    } = setup_test(SeedLevel::PLANS).await.unwrap();

    let customer_id = "018c345f-7324-7cd2-a692-78e5ab9158e0".to_string();
    let plan_version_id = "018c35cc-3f41-7551-b7b6-f8bbcd62b784".to_string();
    let slots_component_id = "3b083801-c77c-4488-848e-a185f0f0a8be".to_string();

    let start = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();

    let slots_quantity = 3;

    let _subscription = clients
        .subscriptions
        .clone()
        .create_subscription(tonic::Request::new(
            api::subscriptions::v1::CreateSubscriptionRequest {
                subscription: Some(api::subscriptions::v1::CreateSubscription {
                    plan_version_id: plan_version_id.clone(),
                    billing_start_date: start.as_proto(),
                    billing_day: 10,
                    customer_id: customer_id.clone(),
                    currency: "USD".to_string(),
                    components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
                        parameterized_components: vec![
                            api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                                component_id: slots_component_id.clone(),
                                billing_period: Some(BillingPeriod::Monthly.into()),
                                initial_slot_count: Some(slots_quantity),
                                ..Default::default()
                            }
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let db_invoices = setup
        .store
        .list_invoices_to_issue(
            1,
            CursorPaginationRequest {
                limit: Some(1000),
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(db_invoices.len(), 1);

    let db_invoice = db_invoices.get(0).unwrap();

    assert_eq!(db_invoice.invoice_date, start);

    let invoice_lines = db_invoice.line_items.clone();

    assert_eq!(
        invoice_lines.len(),
        1,
        "Usage lines are not created in initial invoice."
    );

    let invoice_line = invoice_lines
        .iter()
        .find(|l| l.name == "Organization Slots")
        .unwrap();
    assert_eq!(invoice_line.name, "Organization Slots");
    assert_eq!(invoice_line.quantity, Some(Decimal::from(slots_quantity)));

    // Monthly unit price (1000) * num_days (10 - 1) / total_days_in_month (31)
    let prorated_unit_price: i64 = (2500.0 * (10 - 1) as f64 / 31.0).round() as i64;
    assert_eq!(
        invoice_line.unit_price,
        Some(Decimal::from(prorated_unit_price))
    );
    assert_eq!(
        invoice_line.total,
        prorated_unit_price * slots_quantity as i64
    );

    assert_eq!(invoice_line.start_date, start);
    assert_eq!(invoice_line.end_date, start.with_day(10).unwrap());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

// TDOO capacity, onetime, recurring

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::stats::v1::general_stats_response;

#[tokio::test]
async fn test_stats_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // general stats
    let res = clients
        .stats
        .clone()
        .general_stats(api::stats::v1::GeneralStatsRequest {})
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        &res.total_mrr,
        &Some(general_stats_response::TotalMrr { value_cents: 0 })
    );

    // total_mrr_chart
    let res = clients
        .stats
        .clone()
        .total_mrr_chart(api::stats::v1::MrrChartRequest {
            start_date: None,
            end_date: None,
            plans_id: vec!["ae35bbb9-65da-477d-b856-7dbd87546441".into()],
        })
        .await
        .unwrap()
        .into_inner()
        .series;

    assert_eq!(res.len(), 1);

    // mrr_breakdown
    let res = clients
        .stats
        .clone()
        .mrr_breakdown(api::stats::v1::MrrBreakdownRequest {
            scope: api::stats::v1::MrrBreakdownScope::LastMonth.into(),
        })
        .await
        .unwrap()
        .into_inner()
        .mmr_breakdown
        .unwrap();

    assert_eq!(res.total_net_mrr, 0);

    // mrr_log
    let res = clients
        .stats
        .clone()
        .mrr_log(api::stats::v1::MrrLogRequest {
            before: None,
            after: None,
        })
        .await
        .unwrap()
        .into_inner()
        .entries;

    assert!(res.is_empty());

    // signup_sparkline
    let res = clients
        .stats
        .clone()
        .signup_sparkline(api::stats::v1::SignupSparklineRequest {})
        .await
        .unwrap()
        .into_inner()
        .series
        .unwrap();

    assert!(!res.data.is_empty());

    // trial_conversion_rate_sparkline
    let res = clients
        .stats
        .clone()
        .trial_conversion_rate_sparkline(api::stats::v1::TrialConversionRateSparklineRequest {})
        .await
        .unwrap()
        .into_inner()
        .series;

    assert!(!res.is_empty());

    // top_revenue_by_customer
    let res = clients
        .stats
        .clone()
        .top_revenue_by_customer(api::stats::v1::TopRevenueByCustomerRequest { count: 1 })
        .await
        .unwrap()
        .into_inner()
        .revenue_by_customer;

    assert!(res.is_empty());

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

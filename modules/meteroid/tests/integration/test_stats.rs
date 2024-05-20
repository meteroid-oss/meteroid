use testcontainers::clients::Cli;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

#[tokio::test]
async fn test_stats_basic() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
    );

    // // general stats
    // let res = clients
    //     .stats
    //     .clone()
    //     .general_stats(api::stats::v1::GeneralStatsRequest {})
    //     .await
    //     .unwrap()
    //     .into_inner();
    //
    // assert_eq!(&res.total_mrr, &None);

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
        .into_inner();

    assert_eq!(res.series.is_empty(), true);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

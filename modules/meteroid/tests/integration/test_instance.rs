use testcontainers::clients::Cli;

use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_customers_basic() {
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

    let instance = clients
        .instance
        .clone()
        .get_instance(api::instance::v1::GetInstanceRequest {})
        .await
        .unwrap()
        .into_inner()
        .instance
        .unwrap();

    assert_eq!(instance.company_name, "Local Org".to_string());

    let invite = clients
        .instance
        .clone()
        .get_invite(api::instance::v1::GetInviteRequest {})
        .await
        .unwrap()
        .into_inner();

    assert_eq!(invite.invite_hash, "fake-invite-link".to_string());

    // creating second organization
    // this is a bit of a hack, but we need to check
    // 1) org init
    // 2) org get should fail because it's expected only 1

    let new_org = clients
        .instance
        .clone()
        .init_instance(api::instance::v1::InitInstanceRequest {
            company_name: "new org".to_string(),
            currency: "USD".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        new_org.instance.unwrap().company_name,
        "new org".to_string()
    );

    let invite_res = clients
        .instance
        .clone()
        .get_invite(api::instance::v1::GetInviteRequest {})
        .await;

    log::error!("{:?}", invite_res);

    assert_eq!(invite_res.is_err(), true);

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await
}

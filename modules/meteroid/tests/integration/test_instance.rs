use meteroid_grpc::meteroid::api;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_instance() {
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

    let instance = clients
        .instance
        .clone()
        .get_instance(api::instance::v1::GetInstanceRequest {})
        .await
        .unwrap()
        .into_inner();

    assert!(instance.instance_initiated);
    assert!(!instance.multi_organization_enabled);

    // creating second organization
    //  should fail because it's expected only 1

    let new_org_res = clients
        .organizations
        .clone()
        .create_organization(api::organizations::v1::CreateOrganizationRequest {
            trade_name: "new org".to_string(),
            country: "US".to_string(),
            legal_name: None,
            vat_number: None,
            address_line1: None,
            address_line2: None,
            zip_code: None,
            state: None,
            city: None,
        })
        .await;

    log::error!("{:?}", new_org_res);

    assert!(new_org_res.is_err());

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await
}

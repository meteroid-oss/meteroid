use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::SEED_USERNAME;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_users_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // login
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // me
    let me = clients
        .users
        .clone()
        .me(api::users::v1::MeRequest {})
        .await
        .unwrap()
        .into_inner()
        .user
        .unwrap();

    // TODO check if /me should have role
    // assert_eq!(me.role, UserRole::Admin as i32);
    assert_eq!(me.email, SEED_USERNAME);

    // get by id
    let user = clients
        .users
        .clone()
        .get_user_by_id(api::users::v1::GetUserByIdRequest { id: me.id.clone() })
        .await
        .unwrap()
        .into_inner()
        .user
        .unwrap();

    assert_eq!(user.email, me.email);

    // list
    let users = clients
        .users
        .clone()
        .list_users(api::users::v1::ListUsersRequest {})
        .await
        .unwrap()
        .into_inner()
        .users;

    assert_eq!(users.len(), 1);

    let user = users.first().unwrap().clone();
    assert_eq!(user.email, me.email);

    // register
    let new_email: String = "meteroid-abcd@def.com".into();
    let new_pass: String = "super-secret".into();
    let invite_key: String = "fake-invite-link".into();
    let resp = clients
        .users
        .clone()
        .register(api::users::v1::RegisterRequest {
            email: new_email.clone(),
            password: new_pass.clone(),
            invite_key: Some(invite_key),
        })
        .await
        .unwrap()
        .into_inner();

    let user = resp.user.unwrap();
    assert_eq!(user.email, new_email.clone());
}

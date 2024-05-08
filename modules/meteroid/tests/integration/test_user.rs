use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::SEED_USERNAME;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::users::v1::UserRole;
use testcontainers::clients::Cli;

#[tokio::test]
async fn test_users_basic() {
    // Generic setup
    helpers::init::logging();
    let docker = Cli::default();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres(&docker);
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // login
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    assert_eq!(auth.user.unwrap().role, UserRole::Admin as i32);

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "a712afi5lzhk",
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

    assert_eq!(me.role, UserRole::Admin as i32);
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

    assert_eq!(user, me);

    // find by email
    let user = clients
        .users
        .clone()
        .find_user_by_email(api::users::v1::FindUserByEmailRequest {
            email: SEED_USERNAME.into(),
        })
        .await
        .unwrap()
        .into_inner()
        .user
        .unwrap();

    assert_eq!(user, me);

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
    assert_eq!(user, me);

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
    assert_eq!(user.role, UserRole::Member as i32);
    assert_eq!(user.email, new_email.clone());
}

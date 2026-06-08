use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::SEED_USERNAME;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_users_basic() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: new_email.clone(),
            password: new_pass.clone(),
            invite_key: Some(invite_key),
            validation_token: None,
        })
        .await
        .unwrap()
        .into_inner();

    let user = resp.user.unwrap();
    assert_eq!(user.email, new_email.clone());
}

#[tokio::test]
async fn test_leave_and_remove_member() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
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

    let me = clients
        .users
        .clone()
        .me(api::users::v1::MeRequest {})
        .await
        .unwrap()
        .into_inner()
        .user
        .unwrap();

    // Last admin cannot leave
    let leave_err = clients
        .users
        .clone()
        .leave_organization(api::users::v1::LeaveOrganizationRequest {})
        .await;
    assert!(leave_err.is_err());
    assert_eq!(leave_err.unwrap_err().code(), tonic::Code::InvalidArgument);

    // Register a second user via the seed invite link
    let second_email = "second@meteroid.dev".to_string();
    clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: second_email.clone(),
            password: "super-secret".to_string(),
            invite_key: Some("fake-invite-link".to_string()),
            validation_token: None,
        })
        .await
        .unwrap();

    // List users — expect 2
    let users = clients
        .users
        .clone()
        .list_users(api::users::v1::ListUsersRequest {})
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(users.len(), 2);

    let second_user = users
        .iter()
        .find(|u| u.email == second_email)
        .unwrap()
        .clone();

    // Admin cannot remove themselves
    let self_remove_err = clients
        .users
        .clone()
        .remove_member(api::users::v1::RemoveMemberRequest {
            user_id: me.id.clone(),
        })
        .await;
    assert!(self_remove_err.is_err());
    assert_eq!(
        self_remove_err.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );

    // Admin removes the second user
    clients
        .users
        .clone()
        .remove_member(api::users::v1::RemoveMemberRequest {
            user_id: second_user.id.clone(),
        })
        .await
        .unwrap();

    // List users — back to 1
    let users = clients
        .users
        .clone()
        .list_users(api::users::v1::ListUsersRequest {})
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, me.id);
}

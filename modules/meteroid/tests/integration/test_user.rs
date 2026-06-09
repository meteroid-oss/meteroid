use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::SEED_USERNAME;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api;

async fn invite_and_get_token(clients: &meteroid_it::clients::AllClients, email: &str) -> String {
    clients
        .users
        .clone()
        .invite_member(api::users::v1::InviteMemberRequest {
            email: email.to_string(),
            role: api::users::v1::OrganizationUserRole::Member as i32,
        })
        .await
        .unwrap()
        .into_inner()
        .invite_id
}

#[tokio::test]
async fn test_users_basic() {
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
    assert_eq!(users[0].email, me.email);

    // invite + register via invite token
    let new_email = "meteroid-abcd@def.com";
    let invite_id = invite_and_get_token(&clients, new_email).await;

    let resp = clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: new_email.to_string(),
            password: "super-secret".to_string(),
            invite_key: Some(invite_id),
            validation_token: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.user.unwrap().email, new_email);
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

    // Invite + register a second user
    let second_email = "second@meteroid.dev";
    let invite_id = invite_and_get_token(&clients, second_email).await;

    clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: second_email.to_string(),
            password: "super-secret".to_string(),
            invite_key: Some(invite_id),
            validation_token: None,
        })
        .await
        .unwrap();

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

#[tokio::test]
async fn test_org_invite_flow() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let admin_clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // Anonymous client (no bearer token) for GetInviteDetails
    let anon_clients =
        meteroid_it::clients::AllClients::from_channel(setup.channel.clone(), "", "", "");

    let invitee_email = "invitee@example.com";

    let me_email = SEED_USERNAME;
    let self_invite_err = admin_clients
        .users
        .clone()
        .invite_member(api::users::v1::InviteMemberRequest {
            email: me_email.to_string(),
            role: api::users::v1::OrganizationUserRole::Member as i32,
        })
        .await;
    assert!(self_invite_err.is_err());
    assert_eq!(
        self_invite_err.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );

    admin_clients
        .users
        .clone()
        .invite_member(api::users::v1::InviteMemberRequest {
            email: invitee_email.to_string(),
            role: api::users::v1::OrganizationUserRole::Member as i32,
        })
        .await
        .unwrap();

    let dup_err = admin_clients
        .users
        .clone()
        .invite_member(api::users::v1::InviteMemberRequest {
            email: invitee_email.to_string(),
            role: api::users::v1::OrganizationUserRole::Member as i32,
        })
        .await;
    assert!(dup_err.is_err());
    assert_eq!(dup_err.unwrap_err().code(), tonic::Code::InvalidArgument);

    let invites = admin_clients
        .users
        .clone()
        .list_pending_invites(api::users::v1::ListPendingInvitesRequest {})
        .await
        .unwrap()
        .into_inner()
        .invites;

    assert_eq!(invites.len(), 1);
    let invite = invites.into_iter().next().unwrap();
    assert_eq!(invite.invited_email, invitee_email);
    assert_eq!(
        invite.role,
        api::users::v1::OrganizationUserRole::Member as i32
    );
    assert!(!invite.is_expired);

    let invite_id = invite.id.clone();

    let details = anon_clients
        .instance
        .clone()
        .get_invite_details(api::instance::v1::GetInviteDetailsRequest {
            invite_id: invite_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(!details.organization_name.is_empty());
    assert_eq!(
        details.role,
        api::users::v1::OrganizationUserRole::Member as i32
    );
    assert_eq!(details.invited_email, invitee_email);

    let bad_token_err = anon_clients
        .instance
        .clone()
        .get_invite_details(api::instance::v1::GetInviteDetailsRequest {
            invite_id: "orginv_notreal".to_string(),
        })
        .await;
    assert!(bad_token_err.is_err());

    admin_clients
        .users
        .clone()
        .resend_invite(api::users::v1::ResendInviteRequest {
            invite_id: invite_id.clone(),
        })
        .await
        .unwrap();

    let wrong_email_err = admin_clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: "wrong@example.com".to_string(),
            password: "pass1234".to_string(),
            invite_key: Some(invite_id.clone()),
            validation_token: None,
        })
        .await;
    assert!(wrong_email_err.is_err());

    admin_clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: invitee_email.to_string(),
            password: "pass1234".to_string(),
            invite_key: Some(invite_id.clone()),
            validation_token: None,
        })
        .await
        .unwrap();

    let used_token_err = anon_clients
        .instance
        .clone()
        .get_invite_details(api::instance::v1::GetInviteDetailsRequest {
            invite_id: invite_id.clone(),
        })
        .await;
    assert!(used_token_err.is_err());

    let reuse_err = admin_clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: invitee_email.to_string(),
            password: "pass1234".to_string(),
            invite_key: Some(invite_id.clone()),
            validation_token: None,
        })
        .await;
    assert!(reuse_err.is_err());

    let third_email = "third@example.com";
    let third_invite_id = invite_and_get_token(&admin_clients, third_email).await;

    admin_clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: third_email.to_string(),
            password: "pass1234".to_string(),
            invite_key: Some(third_invite_id.clone()),
            validation_token: None,
        })
        .await
        .unwrap();

    let fourth_email = "fourth@example.com";
    let fourth_invite_id = invite_and_get_token(&admin_clients, fourth_email).await;

    admin_clients
        .users
        .clone()
        .revoke_invite(api::users::v1::RevokeInviteRequest {
            invite_id: fourth_invite_id.clone(),
        })
        .await
        .unwrap();

    let revoked_err = anon_clients
        .instance
        .clone()
        .get_invite_details(api::instance::v1::GetInviteDetailsRequest {
            invite_id: fourth_invite_id.clone(),
        })
        .await;
    assert!(revoked_err.is_err());

    let reg_err = admin_clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: fourth_email.to_string(),
            password: "pass1234".to_string(),
            invite_key: Some(fourth_invite_id.clone()),
            validation_token: None,
        })
        .await;
    assert!(reg_err.is_err());

    let double_revoke_err = admin_clients
        .users
        .clone()
        .revoke_invite(api::users::v1::RevokeInviteRequest {
            invite_id: fourth_invite_id.clone(),
        })
        .await;
    assert!(double_revoke_err.is_err());

    let users = admin_clients
        .users
        .clone()
        .list_users(api::users::v1::ListUsersRequest {})
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(users.len(), 3);
}

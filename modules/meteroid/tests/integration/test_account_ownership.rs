//! Account ownership through the customers and users gRPC services.

use common_domain::ids::{BaseId, CustomerId, UserId};
use common_grpc::meteroid::common::v1::{Filter, FilterOperator};
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::users::UserRowNew;
use meteroid_grpc::meteroid::api::customers::v1::{
    AssignCustomerOwnerRequest, CustomerBrief, GetCustomerByIdRequest, ListCustomerRequest,
};
use meteroid_grpc::meteroid::api::users::v1::RemoveMemberRequest;
use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use meteroid_store::repositories::OrganizationsInterface;
use rstest::rstest;
use tonic::Code;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;

async fn clients(env: &TestEnv) -> AllClients {
    let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    )
}

async fn insert_user(env: &TestEnv, email: &str) -> UserId {
    let id = UserId::new();
    UserRowNew {
        id,
        email: email.to_string(),
        password_hash: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("insert user");
    id
}

pub(crate) async fn add_member(env: &TestEnv, email: &str) -> UserId {
    let id = insert_user(env, email).await;
    OrganizationMemberRow {
        user_id: id,
        organization_id: ORGANIZATION_ID,
        role: OrganizationUserRole::Member,
        role_id: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("insert member");
    id
}

async fn assign(
    clients: &AllClients,
    customers: &[CustomerId],
    owner: Option<UserId>,
) -> Result<u32, tonic::Status> {
    clients
        .customers
        .clone()
        .assign_customer_owner(AssignCustomerOwnerRequest {
            customer_ids: customers.iter().map(|c| c.as_proto()).collect(),
            owner_user_id: owner.map(|o| o.as_proto()),
        })
        .await
        .map(|r| r.into_inner().updated)
}

async fn owner_of(clients: &AllClients, customer: CustomerId) -> Option<String> {
    clients
        .customers
        .clone()
        .get_customer_by_id(GetCustomerByIdRequest {
            id: customer.as_proto(),
        })
        .await
        .unwrap()
        .into_inner()
        .customer
        .unwrap()
        .owner
        .map(|o| o.user_id)
}

async fn list_filtered(
    clients: &AllClients,
    filters: Vec<Filter>,
) -> Result<Vec<CustomerBrief>, tonic::Status> {
    let mut customers = clients
        .customers
        .clone()
        .list_customers(ListCustomerRequest {
            search: None,
            order_by: None,
            pagination: None,
            archived: None,
            lifecycle_state: vec![],
            country: vec![],
            filters,
        })
        .await?
        .into_inner()
        .customers;
    customers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(customers)
}

fn owner_filter(op: FilterOperator, values: Vec<String>) -> Filter {
    Filter {
        field: "owner".to_string(),
        op: op as i32,
        values,
    }
}

/// The customers list's `owner` filter; no owners = no filter.
async fn list_owned_by(clients: &AllClients, owners: &[UserId]) -> Vec<CustomerBrief> {
    let filters = if owners.is_empty() {
        vec![]
    } else {
        vec![owner_filter(
            FilterOperator::In,
            owners.iter().map(|o| o.as_proto()).collect(),
        )]
    };
    list_filtered(clients, filters).await.unwrap()
}

#[rstest]
#[tokio::test]
async fn assign_and_unassign_owner(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let clients = clients(&env).await;

    let updated = assign(
        &clients,
        &[CUST_SPOTIFY_ID, CUST_UBER_ID, CustomerId::new()],
        Some(USER_ID),
    )
    .await
    .unwrap();
    assert_eq!(updated, 2, "ids outside the tenant are ignored");
    assert_eq!(
        owner_of(&clients, CUST_SPOTIFY_ID).await,
        Some(USER_ID.as_proto())
    );
    assert_eq!(
        owner_of(&clients, CUST_UBER_ID).await,
        Some(USER_ID.as_proto())
    );

    assert_eq!(assign(&clients, &[CUST_UBER_ID], None).await.unwrap(), 1);
    assert_eq!(owner_of(&clients, CUST_UBER_ID).await, None);
    assert_eq!(
        owner_of(&clients, CUST_SPOTIFY_ID).await,
        Some(USER_ID.as_proto())
    );
    assert_eq!(owner_of(&clients, CUST_COMODO_ID).await, None);
}

#[rstest]
#[tokio::test]
async fn assign_owner_rejects_non_member(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let clients = clients(&env).await;

    let outsider = insert_user(&env, "outsider@meteroid.dev").await;
    for candidate in [outsider, UserId::new()] {
        let err = assign(&clients, &[CUST_SPOTIFY_ID], Some(candidate))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument, "{err:?}");
    }
    assert_eq!(owner_of(&clients, CUST_SPOTIFY_ID).await, None);

    let err = clients
        .customers
        .clone()
        .assign_customer_owner(AssignCustomerOwnerRequest {
            customer_ids: vec![CUST_SPOTIFY_ID.as_proto()],
            owner_user_id: Some("not-a-user-id".to_string()),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[rstest]
#[tokio::test]
async fn remove_member_clears_owned_accounts(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let clients = clients(&env).await;

    let removed = add_member(&env, "removed@meteroid.dev").await;
    let leaving = add_member(&env, "leaving@meteroid.dev").await;
    assign(&clients, &[CUST_SPOTIFY_ID], Some(removed))
        .await
        .unwrap();
    assign(&clients, &[CUST_UBER_ID], Some(leaving))
        .await
        .unwrap();
    assign(&clients, &[CUST_COMODO_ID], Some(USER_ID))
        .await
        .unwrap();

    clients
        .users
        .clone()
        .remove_member(RemoveMemberRequest {
            user_id: removed.as_proto(),
        })
        .await
        .unwrap();
    assert_eq!(owner_of(&clients, CUST_SPOTIFY_ID).await, None);
    assert_eq!(
        owner_of(&clients, CUST_UBER_ID).await,
        Some(leaving.as_proto())
    );

    env.store()
        .leave_organization(leaving, ORGANIZATION_ID, SecurityRequestContext::default())
        .await
        .unwrap();
    assert_eq!(owner_of(&clients, CUST_UBER_ID).await, None);
    assert_eq!(
        owner_of(&clients, CUST_COMODO_ID).await,
        Some(USER_ID.as_proto()),
        "other members keep their accounts"
    );

    let err = assign(&clients, &[CUST_SPOTIFY_ID], Some(removed))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[rstest]
#[tokio::test]
async fn list_customers_filters_by_owner(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let clients = clients(&env).await;

    let colleague = add_member(&env, "colleague@meteroid.dev").await;
    assign(&clients, &[CUST_SPOTIFY_ID], Some(USER_ID))
        .await
        .unwrap();
    assign(&clients, &[CUST_UBER_ID], Some(colleague))
        .await
        .unwrap();

    let mine = list_owned_by(&clients, &[USER_ID]).await;
    assert_eq!(
        mine.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        vec![CUST_SPOTIFY_ID.as_proto()]
    );
    assert_eq!(
        mine[0].owner.as_ref().map(|o| o.user_id.clone()),
        Some(USER_ID.as_proto())
    );

    let both = list_owned_by(&clients, &[USER_ID, colleague]).await;
    assert_eq!(
        both.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        vec![CUST_SPOTIFY_ID.as_proto(), CUST_UBER_ID.as_proto()]
    );

    let all = list_owned_by(&clients, &[]).await;
    assert!(all.len() > both.len());
    assert!(
        all.iter()
            .any(|c| c.id == CUST_COMODO_ID.as_proto() && c.owner.is_none())
    );
    assert!(list_owned_by(&clients, &[UserId::new()]).await.is_empty());

    let ids =
        |customers: Vec<CustomerBrief>| customers.into_iter().map(|c| c.id).collect::<Vec<_>>();
    let unowned = ids(list_filtered(
        &clients,
        vec![owner_filter(FilterOperator::IsEmpty, vec![])],
    )
    .await
    .unwrap());
    assert!(unowned.contains(&CUST_COMODO_ID.as_proto()));
    assert!(!unowned.contains(&CUST_SPOTIFY_ID.as_proto()));
    assert!(!unowned.contains(&CUST_UBER_ID.as_proto()));
    let owned = ids(
        list_filtered(&clients, vec![owner_filter(FilterOperator::IsSet, vec![])])
            .await
            .unwrap(),
    );
    assert_eq!(
        owned,
        vec![CUST_SPOTIFY_ID.as_proto(), CUST_UBER_ID.as_proto()]
    );
    let not_mine = ids(list_filtered(
        &clients,
        vec![owner_filter(
            FilterOperator::NotEquals,
            vec![USER_ID.as_proto()],
        )],
    )
    .await
    .unwrap());
    assert_eq!(
        not_mine,
        vec![CUST_UBER_ID.as_proto()],
        "is not: owned by someone else, unowned accounts never match"
    );

    let err = list_filtered(
        &clients,
        vec![owner_filter(
            FilterOperator::In,
            vec!["not-a-user".to_string()],
        )],
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[rstest]
#[tokio::test]
async fn member_removal_waits_for_an_assignment_holding_the_membership(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::customer::dsl as c;

    let env = test_env.await;
    let clients = clients(&env).await;
    let leaving = add_member(&env, "leaving@meteroid.dev").await;

    let mut assignment = env.conn().await;
    diesel::sql_query("BEGIN")
        .execute(&mut assignment)
        .await
        .unwrap();
    assert!(
        OrganizationMemberRow::lock_membership(&mut assignment, leaving.as_uuid(), ORGANIZATION_ID)
            .await
            .unwrap()
    );
    diesel::update(c::customer.filter(c::id.eq(CUST_UBER_ID)))
        .set(c::owner_user_id.eq(Some(leaving)))
        .execute(&mut assignment)
        .await
        .unwrap();

    let store = env.store().clone();
    let removal = tokio::spawn(async move {
        store
            .leave_organization(leaving, ORGANIZATION_ID, SecurityRequestContext::default())
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !removal.is_finished(),
        "the removal waits for the assignment holding the membership"
    );

    diesel::sql_query("COMMIT")
        .execute(&mut assignment)
        .await
        .unwrap();
    removal.await.unwrap().unwrap();
    assert_eq!(
        owner_of(&clients, CUST_UBER_ID).await,
        None,
        "the removal releases what the assignment wrote"
    );
}

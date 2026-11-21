//! Member role changes and removals, and platform-admin support access.

use common_domain::ids::{BaseId, UserId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::enums::OrganizationUserRole as DbOrganizationUserRole;
use diesel_models::org_security_audit_log::OrgSecurityAuditLogRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::schema::org_security_audit_log::dsl as sa;
use diesel_models::users::UserRowNew;
use rstest::rstest;

use meteroid_store::domain::enums::OrganizationUserRole;
use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use meteroid_store::repositories::RolesInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;

async fn role_change_events(env: &TestEnv) -> Vec<OrgSecurityAuditLogRow> {
    let mut conn = env.conn().await;
    sa::org_security_audit_log
        .filter(sa::organization_id.eq(ORGANIZATION_ID))
        .filter(sa::event_type.eq("member.role.changed"))
        .order((sa::occurred_at.desc(), sa::id.desc()))
        .select(OrgSecurityAuditLogRow::as_select())
        .load(&mut conn)
        .await
        .expect("list security audit")
}

/// Changing a member's role records a `member.role.changed` security-audit event
/// naming the actor and target; re-applying the same role records nothing.
#[rstest]
#[tokio::test]
async fn test_assign_member_role_records_security_event(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // A second member to promote (USER_ID is the seeded admin actor).
    let target = UserId::new();
    let mut conn = env.conn().await;
    UserRowNew {
        id: target,
        email: "member2@meteroid.dev".to_string(),
        password_hash: None,
    }
    .insert(&mut conn)
    .await
    .expect("insert user");
    OrganizationMemberRow {
        user_id: target,
        organization_id: ORGANIZATION_ID,
        role: DbOrganizationUserRole::Member,
        role_id: None,
    }
    .insert(&mut conn)
    .await
    .expect("insert member");

    env.store()
        .assign_member_role(
            ORGANIZATION_ID,
            USER_ID,
            target,
            OrganizationUserRole::Admin,
            None,
            SecurityRequestContext::default(),
        )
        .await
        .expect("assign role");

    let events = role_change_events(&env).await;
    assert_eq!(events.len(), 1, "expected one member.role.changed event");
    assert_eq!(events[0].target_user_id, Some(target));
    assert_eq!(events[0].actor_id, Some(USER_ID.as_uuid()));

    // Re-assigning the same role changes nothing → no new event.
    env.store()
        .assign_member_role(
            ORGANIZATION_ID,
            USER_ID,
            target,
            OrganizationUserRole::Admin,
            None,
            SecurityRequestContext::default(),
        )
        .await
        .expect("re-assign same role");

    assert_eq!(
        role_change_events(&env).await.len(),
        1,
        "re-assigning the same role must not record another event"
    );
}

const SEED_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$dawIX5+sybNHqfFoNvHFhw$uhtWJd50wiFDV8nR10RNZI4OCrOAJ1kiNZQF0OUSoGE";

async fn list_customers(clients: &AllClients) -> Result<(), tonic::Status> {
    clients
        .customers
        .clone()
        .list_customers(
            meteroid_grpc::meteroid::api::customers::v1::ListCustomerRequest {
                search: None,
                archived: None,
                order_by: None,
                pagination: None,
                lifecycle_state: vec![],
                country: vec![],
                filters: vec![],
            },
        )
        .await
        .map(|_| ())
}

/// A new member of the seeded organization, signed in with the seed user's password. Tests use
/// fresh users rather than the seeded one: member access is cached per process across tests.
async fn signed_in_user(
    setup: &meteroid_it::container::MeteroidSetup,
    email: &str,
    role: DbOrganizationUserRole,
) -> (UserId, String) {
    let id = UserId::new();
    let mut conn = setup.store.pool.get().await.expect("conn");
    UserRowNew {
        id,
        email: email.to_string(),
        password_hash: Some(SEED_PASSWORD_HASH.to_string()),
    }
    .insert(&mut conn)
    .await
    .expect("insert user");
    OrganizationMemberRow {
        user_id: id,
        organization_id: ORGANIZATION_ID,
        role,
        role_id: None,
    }
    .insert(&mut conn)
    .await
    .expect("insert member");
    let token = AllClients::from_channel(setup.channel.clone(), "", "", "")
        .users
        .clone()
        .login(meteroid_grpc::meteroid::api::users::v1::LoginRequest {
            email: email.to_string(),
            password: meteroid_it::svc_auth::SEED_PASSWORD.to_string(),
        })
        .await
        .expect("login")
        .into_inner()
        .token;
    (id, token)
}

async fn member_clients(env: &TestEnv, email: &str) -> (UserId, AllClients) {
    let (id, token) = signed_in_user(&env.setup, email, DbOrganizationUserRole::Member).await;
    let clients =
        AllClients::from_channel(env.setup.channel.clone(), &token, "TESTORG", "testslug");
    (id, clients)
}

/// A removed member, or one who left, loses access on their next request rather than once
/// their cached access expires.
#[rstest]
#[tokio::test]
async fn test_removed_and_leaving_members_lose_access_at_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let admin = {
        let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
        AllClients::from_channel(
            env.setup.channel.clone(),
            &auth.token,
            "TESTORG",
            "testslug",
        )
    };

    let (removed, removed_clients) = member_clients(&env, "removed@meteroid.dev").await;
    let (_, leaving_clients) = member_clients(&env, "leaving@meteroid.dev").await;
    list_customers(&removed_clients)
        .await
        .expect("member reads");
    list_customers(&leaving_clients)
        .await
        .expect("member reads");

    admin
        .users
        .clone()
        .remove_member(
            meteroid_grpc::meteroid::api::users::v1::RemoveMemberRequest {
                user_id: removed.as_proto(),
            },
        )
        .await
        .expect("remove member");
    leaving_clients
        .users
        .clone()
        .leave_organization(meteroid_grpc::meteroid::api::users::v1::LeaveOrganizationRequest {})
        .await
        .expect("leave organization");

    for clients in [&removed_clients, &leaving_clients] {
        let denied = list_customers(clients).await.unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied, "{denied:?}");
    }
}

/// Grants Admin support access to any organization, as a live grant would.
struct AdminGrants;

#[async_trait::async_trait]
impl meteroid_middleware::server::auth::SupportGrantReader for AdminGrants {
    async fn grant_mode(
        &self,
        _user_id: UserId,
        _org_id: common_domain::ids::OrganizationId,
    ) -> Option<meteroid_middleware::server::auth::SupportGrantMode> {
        Some(meteroid_middleware::server::auth::SupportGrantMode::Admin)
    }
}

struct SupportAccess;

impl meteroid::module::ApiModule for SupportAccess {
    fn id(&self) -> &'static str {
        "support-access"
    }

    fn support_grants(
        &self,
        _ctx: &meteroid::module::ApiContext,
    ) -> Option<meteroid_middleware::server::auth::SharedSupportGrantReader> {
        Some(std::sync::Arc::new(AdminGrants))
    }
}

/// A platform admin demoted in the admin organization can no longer use their live grants.
#[tokio::test]
async fn test_demoted_platform_admin_loses_support_access() {
    use diesel_async::SimpleAsyncConnection;

    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(
        meteroid_it::container::SeedLevel::MINIMAL,
    )
    .await;
    let setup = meteroid_it::container::start_meteroid_as_platform_admin(
        database,
        meteroid_it::container::SeedLevel::MINIMAL,
        meteroid::module::Modules {
            api: vec![std::sync::Arc::new(SupportAccess)],
            ..meteroid::module::Modules::core()
        },
    )
    .await;
    let mut conn = setup.store.pool.get().await.expect("conn");
    conn.batch_execute(
        r#"
INSERT INTO organization(id,trade_name,slug,default_country) VALUES
  ('30000000-0000-0000-0000-00000000000b','B','org-b','US');
INSERT INTO tenant(id,name,slug,organization_id,reporting_currency,environment) VALUES
  ('30000000-0000-0000-0000-0000000000b1','B sand','b-sand','30000000-0000-0000-0000-00000000000b','USD','SANDBOX');
"#,
    )
    .await
    .expect("fixture");

    let (platform_admin, token) = signed_in_user(
        &setup,
        "platform-admin@meteroid.dev",
        DbOrganizationUserRole::Admin,
    )
    .await;
    let in_org_b = AllClients::from_channel(setup.channel.clone(), &token, "org-b", "b-sand");
    let current = |clients: &AllClients| {
        let mut organizations = clients.organizations.clone();
        async move {
            organizations
                .get_current_organizations(
                    meteroid_grpc::meteroid::api::organizations::v1::GetCurrentOrganizationRequest {},
                )
                .await
                .map(|r| r.into_inner().support_session)
        }
    };

    list_customers(&in_org_b)
        .await
        .expect("platform admin with a grant");
    assert!(current(&in_org_b).await.expect("support session"));

    diesel::update(
        diesel_models::schema::organization_member::table
            .filter(diesel_models::schema::organization_member::user_id.eq(platform_admin))
            .filter(
                diesel_models::schema::organization_member::organization_id.eq(ORGANIZATION_ID),
            ),
    )
    .set(diesel_models::schema::organization_member::role.eq(DbOrganizationUserRole::Member))
    .execute(&mut conn)
    .await
    .expect("demote");
    // What AssignRole does after changing a role.
    meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_member_access_cache(
        platform_admin,
        ORGANIZATION_ID,
    )
    .await;

    let denied = list_customers(&in_org_b).await.unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied, "{denied:?}");
    assert!(
        current(&in_org_b).await.is_err(),
        "no support session either"
    );
}

//! Security-audit coverage for member role changes.

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
    assert_eq!(events[0].target_user_id, Some(target.as_uuid()));
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

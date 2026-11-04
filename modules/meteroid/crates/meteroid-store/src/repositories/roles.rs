use async_trait::async_trait;
use error_stack::Report;

use common_domain::ids::{BaseId, OrganizationId, RoleId, UserId};
use common_domain::rbac::RolePermissions;

use crate::StoreResult;
use crate::domain::enums::OrganizationUserRole;
use crate::domain::org_security_audit_log::{
    Actor, RecordSecurityEvent, SecurityEventType, SecurityRequestContext,
};
use crate::domain::roles::{MemberAccess, Role};
use crate::errors::StoreError;
use crate::repositories::org_security_audit_log::SecurityAuditInterface;
use crate::store::Store;

use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::roles::RoleRow;
use diesel_models::users::UserRow;
use scoped_futures::ScopedFutureExt;

#[async_trait]
pub trait RolesInterface {
    async fn list_roles(&self, org_id: OrganizationId) -> StoreResult<Vec<Role>>;

    async fn get_role(&self, org_id: OrganizationId, id: RoleId) -> StoreResult<Role>;

    /// Assign a member's tier and optional custom role. `custom_role_id` is
    /// validated to belong to `org_id`. Records a `member.role.changed` security
    /// event when the assignment actually changes the member's role.
    async fn assign_member_role(
        &self,
        org_id: OrganizationId,
        actor_id: UserId,
        target_user: UserId,
        role: OrganizationUserRole,
        custom_role_id: Option<RoleId>,
        request_context: SecurityRequestContext,
    ) -> StoreResult<()>;

    /// Resolve the effective access (tier + permission set) for a member.
    async fn resolve_member_access(
        &self,
        user_id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<MemberAccess>;
}

#[async_trait]
impl RolesInterface for Store {
    async fn list_roles(&self, org_id: OrganizationId) -> StoreResult<Vec<Role>> {
        let mut conn = self.get_conn().await?;
        let rows = RoleRow::list_by_org(&mut conn, org_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_role(&self, org_id: OrganizationId, id: RoleId) -> StoreResult<Role> {
        let mut conn = self.get_conn().await?;
        let row = RoleRow::find_by_id_and_org(&mut conn, id, org_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(row.into())
    }

    async fn assign_member_role(
        &self,
        org_id: OrganizationId,
        actor_id: UserId,
        target_user: UserId,
        role: OrganizationUserRole,
        custom_role_id: Option<RoleId>,
        request_context: SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let (previous_role, previous_custom_role_id) = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    if let Some(role_id) = custom_role_id {
                        RoleRow::find_by_id_and_org(conn, role_id, org_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    // Preserve the "at least one owner" invariant: demoting the last
                    // Admin would leave the organization with nobody able to manage it.
                    let current = OrganizationMemberRow::find_by_user_and_org(
                        conn,
                        target_user.as_uuid(),
                        org_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let current_role: OrganizationUserRole = current.role.into();
                    let demoting_last_owner = matches!(current_role, OrganizationUserRole::Admin)
                        && !matches!(role, OrganizationUserRole::Admin);
                    if demoting_last_owner {
                        let admin_count = OrganizationMemberRow::count_admins(conn, org_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        if admin_count <= 1 {
                            return Err(StoreError::InvalidArgument(
                                "This is the last owner. Assign another owner before changing this member's role."
                                    .to_string(),
                            )
                            .into());
                        }
                    }

                    OrganizationMemberRow::update_role(
                        conn,
                        target_user.as_uuid(),
                        org_id,
                        role.into(),
                        custom_role_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    Ok((current_role, current.role_id))
                }
                .scope_boxed()
            })
            .await?;

        // Only audit a real change (tier or custom-role reassignment); re-saving the
        // same role must not pollute the security log. Recorded after commit
        // (best-effort) so an audit write never rolls back the change.
        if previous_role != role || previous_custom_role_id != custom_role_id {
            let actor_email = UserRow::find_by_id(&mut conn, actor_id.as_uuid())
                .await
                .ok()
                .map(|u| u.email);
            let target_email = UserRow::find_by_id(&mut conn, target_user.as_uuid())
                .await
                .ok()
                .map(|u| u.email);

            self.record_security_event_best_effort(RecordSecurityEvent {
                organization_id: org_id,
                event_type: SecurityEventType::MemberRoleChanged,
                actor: Actor::User { id: actor_id },
                actor_email,
                target_user_id: Some(target_user.as_uuid()),
                target_email,
                metadata: Some(serde_json::json!({
                    "from_role": format!("{previous_role:?}"),
                    "to_role": format!("{role:?}"),
                    "from_custom_role_id": previous_custom_role_id.map(|r| r.as_base62()),
                    "to_custom_role_id": custom_role_id.map(|r| r.as_base62()),
                })),
                context: request_context,
            })
            .await;
        }

        Ok(())
    }

    async fn resolve_member_access(
        &self,
        user_id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<MemberAccess> {
        let mut conn = self.get_conn().await?;
        let member =
            OrganizationMemberRow::find_by_user_and_org(&mut conn, user_id.as_uuid(), org_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let role: OrganizationUserRole = member.role.into();
        let permissions = match (role, member.role_id) {
            (OrganizationUserRole::Admin, _) => RolePermissions::All,
            (_, Some(role_id)) => {
                let role_row = RoleRow::find_by_id_and_org(&mut conn, role_id, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                RolePermissions::from_codes(role_row.permissions.into_iter().flatten())
            }
            (_, None) => RolePermissions::member_default(),
        };

        Ok(MemberAccess { role, permissions })
    }
}

use crate::StoreResult;
use crate::domain::org_security_audit_log::{
    Actor, RecordSecurityEvent, SecurityEventType, SecurityRequestContext,
};
use crate::errors::StoreError;
use crate::repositories::org_security_audit_log::SecurityAuditInterface;
use crate::repositories::organizations::{mark_organization_for_deletion, release_member_accounts};
use crate::store::Store;
use common_domain::ids::{BaseId, OrganizationId, TenantId, UserId};
use diesel_models::entity_list_views::EntityListViewDefaultRow;
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_invites::OrganizationInviteRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::users::UserRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

#[derive(Debug, Clone)]
pub struct DeletedUser {
    pub organizations_left: Vec<OrganizationId>,
    /// Organizations the user was the only member of, scheduled for deletion with them.
    pub deleted_organizations: Vec<OrganizationId>,
}

#[async_trait::async_trait]
pub trait UserDeletionInterface {
    /// Erases a user's personal data and blocks the account for good. The row is anonymised
    /// rather than deleted: ledgers, activity and the security audit trail keep referencing its
    /// id. The email is freed, so the person can sign up again as a new user. Organizations the
    /// user is the only member of are scheduled for deletion, unless one has a production tenant.
    async fn delete_user(
        &self,
        actor: UserId,
        user_id: UserId,
        request_context: SecurityRequestContext,
    ) -> StoreResult<DeletedUser>;

    async fn is_user_active(&self, user_id: UserId) -> StoreResult<bool>;
}

#[async_trait::async_trait]
impl UserDeletionInterface for Store {
    async fn delete_user(
        &self,
        actor: UserId,
        user_id: UserId,
        request_context: SecurityRequestContext,
    ) -> StoreResult<DeletedUser> {
        let now = chrono::Utc::now().naive_utc();

        let member_hooks = &*self.hooks.members;
        // The tenants it marks for deletion leave the evaluator's scope: bumped last.
        let (deleted, marked_tenants) = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let user = UserRow::find_by_id(conn, *user_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if UserRow::anonymize(conn, user_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        == 0
                    {
                        return Err(Report::new(StoreError::FailedPrecondition(
                            "User is already deleted".to_string(),
                        )));
                    }

                    let memberships = OrganizationMemberRow::list_by_user_for_update(conn, *user_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let mut deleted = DeletedUser {
                        organizations_left: Vec::with_capacity(memberships.len()),
                        deleted_organizations: vec![],
                    };
                    let mut marked_tenants: Vec<TenantId> = vec![];

                    for membership in memberships {
                        let org_id = membership.organization_id;
                        let members = OrganizationMemberRow::count_members(conn, org_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        if members <= 1 {
                            marked_tenants
                                .extend(mark_organization_for_deletion(conn, org_id, now).await?);
                            deleted.deleted_organizations.push(org_id);
                        } else if matches!(membership.role, OrganizationUserRole::Admin)
                            && OrganizationMemberRow::count_admins(conn, org_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?
                                <= 1
                        {
                            return Err(Report::new(StoreError::FailedPrecondition(format!(
                                "User is the last admin of organization {org_id}, which has other members. Transfer the admin role first."
                            ))));
                        }

                        OrganizationMemberRow::delete_member(conn, *user_id, org_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        release_member_accounts(conn, member_hooks, org_id, user_id).await?;
                        deleted.organizations_left.push(org_id);
                    }

                    OrganizationInviteRow::delete_all_for_email(conn, &user.email)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    EntityListViewDefaultRow::delete_all_private_for_user(conn, user_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(((deleted, marked_tenants.clone()), marked_tenants))
                }
                .scope_boxed()
            })
            .await?;

        for tenant_id in &marked_tenants {
            crate::repositories::tenants::invalidate_tenant_cache(tenant_id).await;
        }

        for org_id in &deleted.organizations_left {
            self.record_security_event_best_effort(RecordSecurityEvent {
                organization_id: *org_id,
                event_type: SecurityEventType::UserDeleted,
                actor: Actor::User { id: actor },
                actor_email: None,
                target_user_id: Some(user_id.as_uuid()),
                target_email: None,
                metadata: None,
                context: request_context.clone(),
            })
            .await;
        }
        for org_id in &deleted.deleted_organizations {
            self.record_security_event_best_effort(RecordSecurityEvent {
                organization_id: *org_id,
                event_type: SecurityEventType::OrganizationDeletionScheduled,
                actor: Actor::User { id: actor },
                actor_email: None,
                target_user_id: Some(user_id.as_uuid()),
                target_email: None,
                metadata: None,
                context: request_context.clone(),
            })
            .await;
        }

        Ok(deleted)
    }

    async fn is_user_active(&self, user_id: UserId) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        UserRow::is_active(&mut conn, user_id)
            .await
            .map_err(Into::into)
    }
}

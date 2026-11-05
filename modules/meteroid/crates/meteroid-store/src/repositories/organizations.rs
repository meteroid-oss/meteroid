use crate::StoreResult;
use crate::domain::enums::TenantEnvironmentEnum;
use crate::domain::organizations::{InviteDetails, OrganizationInvite};
use crate::domain::{
    InstanceFlags, Organization, OrganizationNew, OrganizationWithTenants, TenantNew,
};
use crate::errors::StoreError;
use crate::hooks::MemberHooks;
use crate::repositories::org_security_audit_log::SecurityAuditInterface;
use crate::store::{PgConn, Store};
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, OrganizationId, OrganizationInviteId, RoleId, TenantId, UserId};
use common_eventbus::Event;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_invites::{OrganizationInviteRow, OrganizationInviteRowNew};
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::{OrganizationRow, OrganizationRowNew};
use diesel_models::roles::RoleRow;
use diesel_models::tenants::TenantRow;
use diesel_models::users::UserRow;
use error_stack::Report;
use meteroid_oauth::model::OauthProvider;
use scoped_futures::ScopedFutureExt;
use tracing::info;
use tracing_log::log;

#[async_trait::async_trait]
pub trait OrganizationsInterface {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        actor: UserId,
    ) -> StoreResult<OrganizationWithTenants>;

    async fn get_instance(&self) -> StoreResult<InstanceFlags>;

    async fn count_organizations(&self) -> StoreResult<i64>;

    async fn invite_member(
        &self,
        org_id: OrganizationId,
        actor_id: UserId,
        invited_email: String,
        role: OrganizationUserRole,
        custom_role_id: Option<RoleId>,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<OrganizationInvite>;

    async fn resend_invite(
        &self,
        invite_id: OrganizationInviteId,
        actor_id: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn revoke_invite(
        &self,
        invite_id: OrganizationInviteId,
        org_id: OrganizationId,
        actor_id: UserId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn list_pending_invites(
        &self,
        org_id: OrganizationId,
    ) -> StoreResult<Vec<OrganizationInvite>>;

    async fn get_invite_details(
        &self,
        invite_id: OrganizationInviteId,
    ) -> StoreResult<InviteDetails>;

    async fn accept_invite(
        &self,
        user_id: UserId,
        invite_id: OrganizationInviteId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<Organization>;

    async fn leave_organization(
        &self,
        actor: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn remove_member(
        &self,
        actor: UserId,
        target_user_id: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    async fn list_organizations_for_user(&self, user_id: UserId) -> StoreResult<Vec<Organization>>;
    async fn get_organization_by_id(&self, id: OrganizationId) -> StoreResult<Organization>;
    async fn get_organization_by_tenant_id(&self, id: &TenantId) -> StoreResult<Organization>;
    async fn get_organizations_with_tenants_by_id(
        &self,
        id: OrganizationId,
    ) -> StoreResult<OrganizationWithTenants>;
    async fn get_organizations_by_slug(&self, slug: String) -> StoreResult<Organization>;

    async fn get_organizations_by_ids(
        &self,
        ids: &[OrganizationId],
    ) -> StoreResult<Vec<Organization>>;

    async fn insert_express_organization(
        &self,
        organization: OrganizationNew,
        actor: UserId,
        tenant_environment: TenantEnvironmentEnum,
    ) -> StoreResult<OrganizationWithTenants>;

    /// Request deletion of an organization: archive it and mark all its tenants
    /// `PendingDeletion`. Refuses if it owns any Production tenant. The weekly
    /// cleanup worker then purges the tenants and finally the organization.
    async fn schedule_organization_deletion(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()>;

    /// Hard-delete archived organizations that no longer have any tenant — the
    /// final step of the cleanup worker, once all their tenants are purged.
    async fn delete_empty_archived_organizations(&self) -> StoreResult<()>;

    /// Toggle the org-wide MFA enforcement. Requires the actor to be an Admin
    /// member of the organization.
    async fn set_organization_mfa_required(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        required: bool,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<Organization>;
}

/// Checked inside the caller's write transaction under a share lock on the membership, so a
/// concurrent removal waits for the write and then releases it.
pub(crate) async fn ensure_org_member(
    conn: &mut PgConn,
    tenant_id: TenantId,
    user_id: UserId,
) -> StoreResult<()> {
    let tenant = TenantRow::find_by_id(conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let is_member =
        OrganizationMemberRow::lock_membership(conn, user_id.as_uuid(), tenant.organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if !is_member {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "User {user_id} is not a member of this organization"
        ))));
    }
    Ok(())
}

/// Archives the organization and marks all its tenants `PendingDeletion`; the weekly cleanup
/// worker then purges the tenants' data and removes the empty organization. Refuses an
/// organization that still has a production tenant.
pub(crate) async fn mark_organization_for_deletion(
    conn: &mut PgConn,
    organization_id: OrganizationId,
    now: chrono::NaiveDateTime,
) -> StoreResult<Vec<TenantId>> {
    let tenants = TenantRow::list_by_organization_id(conn, organization_id).await?;
    if tenants
        .iter()
        .any(|t| t.environment == diesel_models::enums::TenantEnvironmentEnum::Production)
    {
        return Err(Report::new(StoreError::FailedPrecondition(format!(
            "cannot delete organization {organization_id}: it still has a production tenant"
        ))));
    }

    OrganizationRow::set_archived_at(conn, organization_id, now)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let tenant_ids: Vec<TenantId> = tenants.into_iter().map(|t| t.id).collect();
    for tid in &tenant_ids {
        TenantRow::set_status(
            conn,
            *tid,
            diesel_models::enums::TenantStatusEnum::PendingDeletion,
            now,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(tenant_ids)
}

/// A departing member's accounts, and what the edition lets them own, become
/// unowned, in the removal transaction.
pub(crate) async fn release_member_accounts(
    conn: &mut PgConn,
    members: &dyn MemberHooks,
    org_id: OrganizationId,
    user_id: UserId,
) -> StoreResult<()> {
    CustomerRow::clear_owner_for_user_in_org(conn, org_id, user_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    members.member_released(conn, org_id, user_id).await
}

#[async_trait::async_trait]
impl OrganizationsInterface for Store {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        user_id: UserId,
    ) -> StoreResult<OrganizationWithTenants> {
        let mut conn = self.get_conn().await?;

        if !self.multi_organization_enabled() {
            let exists = OrganizationRow::exists(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if exists {
                return Err(StoreError::InvalidArgument(
                    "This instance does not allow multiple organizations".to_string(),
                )
                .into());
            }
        }

        let org = OrganizationRowNew {
            id: OrganizationId::new(),
            slug: Organization::new_slug(),
            trade_name: organization.trade_name.clone(),
            default_country: organization.country.clone(),
            is_express: false,
        };

        // TODO trigger sandbox init ?

        let org_member = OrganizationMemberRow {
            user_id,
            organization_id: org.id,
            role: OrganizationUserRole::Admin,
            role_id: None,
        };

        let dev_tenant_new = TenantNew {
            name: "Development".to_string(),
            environment: TenantEnvironmentEnum::Development,
            email_delivery_mode: None,
            invoicing_entity: None,
        };

        let (org_created, tenant_created) = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let org_created = OrganizationRowNew::insert(&org, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    OrganizationMemberRow::insert(&org_member, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let tenant_created = self
                        .internal
                        .insert_tenant_with_default_entities(
                            conn,
                            dev_tenant_new,
                            org.id,
                            org.trade_name.clone(),
                            org.default_country.clone(),
                            organization.invoicing_entity.unwrap_or_default(),
                        )
                        .await?;

                    Ok((org_created, tenant_created))
                }
                .scope_boxed()
            })
            .await?;
        drop(conn);

        let organization: Organization = org_created.into();
        self.organization_hooks
            .organization_created(&organization)
            .await?;

        let _ = self
            .eventbus
            .publish(Event::organization_created(
                Actor::User { id: user_id },
                organization.id,
            ))
            .await;

        Ok(OrganizationWithTenants {
            organization,
            tenants: vec![tenant_created],
        })
    }

    async fn count_organizations(&self) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;
        OrganizationRow::count(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn get_instance(&self) -> StoreResult<InstanceFlags> {
        let mut conn = self.get_conn().await?;

        let (multi_organization_enabled, instance_initiated) = if self.multi_organization_enabled()
        {
            (true, true)
        } else {
            // single organization
            let exists = OrganizationRow::exists(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            (false, exists)
        };

        Ok(InstanceFlags {
            multi_organization_enabled,
            instance_initiated,
            mailer_enabled: self.settings.mailer_enabled,
            google_oauth_client_id: self.oauth.client_id(OauthProvider::Google),
            hubspot_oauth_client_id: self.oauth.client_id(OauthProvider::Hubspot),
            pennylane_oauth_client_id: self.oauth.client_id(OauthProvider::Pennylane),
        })
    }

    async fn invite_member(
        &self,
        org_id: OrganizationId,
        actor_id: UserId,
        invited_email: String,
        role: OrganizationUserRole,
        custom_role_id: Option<RoleId>,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<OrganizationInvite> {
        let mut conn = self.get_conn().await?;

        // A custom role must belong to this org; a null role_id keeps the tier.
        if let Some(role_id) = custom_role_id {
            RoleRow::find_by_id_and_org(&mut conn, role_id, org_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        let actor_user = UserRow::find_by_id(&mut conn, *actor_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if actor_user.email.to_lowercase() == invited_email.to_lowercase() {
            return Err(
                StoreError::InvalidArgument("You cannot invite yourself".to_string()).into(),
            );
        }

        let invited_email = invited_email.to_lowercase();

        let already_member = match UserRow::find_by_email(&mut conn, invited_email.clone()).await {
            Ok(Some(existing_user)) => {
                match OrganizationMemberRow::find_by_user_and_org(
                    &mut conn,
                    *existing_user.id,
                    org_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)
                {
                    Ok(_) => true,
                    Err(e) if matches!(e.current_context(), StoreError::ValueNotFound(_)) => false,
                    Err(e) => return Err(e),
                }
            }
            Ok(None) => false,
            Err(e) => return Err(e.into()),
        };
        if already_member {
            return Err(StoreError::InvalidArgument("User is already a member".to_string()).into());
        }

        if OrganizationInviteRow::find_pending_by_email_and_org(&mut conn, org_id, &invited_email)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .is_some()
        {
            return Err(StoreError::InvalidArgument(
                "An active invite already exists for this email".to_string(),
            )
            .into());
        }

        OrganizationInviteRow::revoke_expired_for_email_and_org(&mut conn, org_id, &invited_email)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let ttl_days = self.settings.invite_ttl_days;
        let expires_at = Utc::now() + chrono::Duration::days(ttl_days as i64);

        let row_new = OrganizationInviteRowNew {
            id: OrganizationInviteId::new(),
            organization_id: org_id,
            invited_email: invited_email.clone(),
            invited_by: *actor_id,
            role,
            expires_at,
            role_id: custom_role_id,
        };

        let row = row_new.insert(&mut conn).await.map_err(|e| {
            let store_err = Into::<Report<StoreError>>::into(e);
            if matches!(
                store_err.current_context(),
                StoreError::DuplicateValue { .. }
            ) {
                Report::new(StoreError::InvalidArgument(
                    "An active invite already exists for this email".to_string(),
                ))
            } else {
                store_err
            }
        })?;

        let org = OrganizationRow::get_by_id(&mut conn, org_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);

        if self.settings.mailer_enabled {
            self.send_invite_email(
                row.id,
                &org.trade_name,
                &actor_user.email,
                &role,
                row.invited_email.clone(),
            )
            .await;
        }

        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id: org_id,
                event_type: crate::domain::org_security_audit_log::SecurityEventType::MemberInvited,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor_id },
                actor_email: Some(actor_user.email.clone()),
                target_user_id: None,
                target_email: Some(row.invited_email.clone()),
                metadata: Some(serde_json::json!({"role": format!("{:?}", role)})),
                context: request_context,
            },
        )
        .await;

        Ok(OrganizationInvite::from_row_and_inviter(
            row,
            actor_user.email,
        ))
    }

    async fn resend_invite(
        &self,
        invite_id: OrganizationInviteId,
        actor_id: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let invite = OrganizationInviteRow::find_by_id(&mut conn, invite_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if invite.organization_id != org_id {
            return Err(StoreError::Forbidden(
                "Invite does not belong to this organization".to_string(),
            )
            .into());
        }
        if invite.accepted_at.is_some() {
            return Err(StoreError::InvalidArgument("Invite already accepted".to_string()).into());
        }
        if invite.revoked_at.is_some() {
            return Err(StoreError::InvalidArgument("Invite is revoked".to_string()).into());
        }
        if invite.expires_at < Utc::now() {
            return Err(StoreError::InvalidArgument(
                "Invite is expired. Revoke it and create a new one.".to_string(),
            )
            .into());
        }

        let ttl_days = self.settings.invite_ttl_days;
        let new_expires_at = Utc::now() + chrono::Duration::days(ttl_days as i64);

        OrganizationInviteRow::update_expires_at(&mut conn, invite_id, new_expires_at)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let org = OrganizationRow::get_by_id(&mut conn, org_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let actor_user = UserRow::find_by_id(&mut conn, *actor_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);

        let invited_email = invite.invited_email.clone();
        if self.settings.mailer_enabled {
            self.send_invite_email(
                invite.id,
                &org.trade_name,
                &actor_user.email,
                &invite.role,
                invited_email.clone(),
            )
            .await;
        }

        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id: org_id,
                event_type:
                    crate::domain::org_security_audit_log::SecurityEventType::MemberInviteResent,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor_id },
                actor_email: Some(actor_user.email),
                target_user_id: None,
                target_email: Some(invited_email),
                metadata: None,
                context: request_context,
            },
        )
        .await;

        Ok(())
    }

    async fn revoke_invite(
        &self,
        invite_id: OrganizationInviteId,
        org_id: OrganizationId,
        actor_id: UserId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let invite = OrganizationInviteRow::find_by_id(&mut conn, invite_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if invite.organization_id != org_id {
            return Err(StoreError::Forbidden(
                "Invite does not belong to this organization".to_string(),
            )
            .into());
        }
        if invite.accepted_at.is_some() {
            return Err(StoreError::InvalidArgument("Invite already accepted".to_string()).into());
        }
        if invite.revoked_at.is_some() {
            return Err(StoreError::InvalidArgument("Invite already revoked".to_string()).into());
        }

        OrganizationInviteRow::set_revoked_at(&mut conn, invite_id, Utc::now())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let actor_user = UserRow::find_by_id(&mut conn, *actor_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id: org_id,
                event_type:
                    crate::domain::org_security_audit_log::SecurityEventType::MemberInviteRevoked,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor_id },
                actor_email: Some(actor_user.email),
                target_user_id: None,
                target_email: Some(invite.invited_email),
                metadata: None,
                context: request_context,
            },
        )
        .await;

        Ok(())
    }

    async fn list_pending_invites(
        &self,
        org_id: OrganizationId,
    ) -> StoreResult<Vec<OrganizationInvite>> {
        let mut conn = self.get_conn().await?;

        let rows = OrganizationInviteRow::list_pending_with_inviter_email(&mut conn, org_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|(row, inviter_email)| {
                OrganizationInvite::from_row_and_inviter(row, inviter_email)
            })
            .collect())
    }

    async fn get_invite_details(
        &self,
        invite_id: OrganizationInviteId,
    ) -> StoreResult<InviteDetails> {
        let mut conn = self.get_conn().await?;

        let invite = OrganizationInviteRow::find_by_id(&mut conn, invite_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if invite.revoked_at.is_some()
            || invite.accepted_at.is_some()
            || invite.expires_at < Utc::now()
        {
            return Err(
                StoreError::InvalidArgument("Invalid or expired invite link".to_string()).into(),
            );
        }

        let org = OrganizationRow::get_by_id(&mut conn, invite.organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let custom_role_name = match invite.role_id {
            Some(role_id) => Some(
                RoleRow::find_by_id_and_org(&mut conn, role_id, invite.organization_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .name,
            ),
            None => None,
        };

        Ok(InviteDetails {
            organization_name: org.trade_name,
            role: invite.role,
            invited_email: invite.invited_email,
            custom_role_name,
        })
    }

    async fn accept_invite(
        &self,
        user_id: UserId,
        invite_id: OrganizationInviteId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let (organization, user_email): (Organization, String) = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let invite = OrganizationInviteRow::find_by_id(conn, invite_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if invite.accepted_at.is_some() {
                        return Err(StoreError::InvalidArgument(
                            "This invite has already been used".to_string(),
                        )
                        .into());
                    }
                    if invite.revoked_at.is_some() || invite.expires_at < Utc::now() {
                        return Err(StoreError::InvalidArgument(
                            "Invalid or expired invite link".to_string(),
                        )
                        .into());
                    }

                    let user = UserRow::find_by_id(conn, *user_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if invite.invited_email.to_lowercase() != user.email.to_lowercase() {
                        return Err(StoreError::Forbidden(
                            "This invite is not valid for your account".to_string(),
                        )
                        .into());
                    }

                    match OrganizationMemberRow::find_by_user_and_org(
                        conn,
                        *user_id,
                        invite.organization_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    {
                        Ok(_) => {
                            return Err(StoreError::InvalidArgument(
                                "You are already a member of this organization".to_string(),
                            )
                            .into());
                        }
                        Err(e) if matches!(e.current_context(), StoreError::ValueNotFound(_)) => {}
                        Err(e) => return Err(e),
                    }

                    let org_member = OrganizationMemberRow {
                        user_id,
                        organization_id: invite.organization_id,
                        role: invite.role,
                        role_id: invite.role_id,
                    };
                    OrganizationMemberRow::insert(&org_member, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    OrganizationInviteRow::set_accepted_at(conn, invite_id, Utc::now())
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let org = OrganizationRow::get_by_id(conn, invite.organization_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok::<_, error_stack::Report<StoreError>>((org.into(), user.email))
                }
                .scope_boxed()
            })
            .await?;

        let organization_id = organization.id;
        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id,
                event_type:
                    crate::domain::org_security_audit_log::SecurityEventType::MemberInviteAccepted,
                actor: crate::domain::org_security_audit_log::Actor::User { id: user_id },
                actor_email: Some(user_email.clone()),
                target_user_id: Some(user_id.as_uuid()),
                target_email: Some(user_email),
                metadata: None,
                context: request_context,
            },
        )
        .await;

        Ok(organization)
    }

    async fn leave_organization(
        &self,
        actor: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let members = &*self.hooks.members;

        self.transaction_with(&mut conn, |conn| {
            async move {
                let member = OrganizationMemberRow::find_by_user_and_org(conn, *actor, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if matches!(member.role, OrganizationUserRole::Admin) {
                    let admin_count = OrganizationMemberRow::count_admins(conn, org_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if admin_count <= 1 {
                        return Err(StoreError::InvalidArgument(
                            "Last admin cannot leave the organization. Transfer the admin role first.".to_string(),
                        )
                        .into());
                    }
                }

                OrganizationMemberRow::delete_member(conn, *actor, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                release_member_accounts(conn, members, org_id, actor).await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        // Best-effort: at this point the row is already gone, so fetch the
        // user's email separately for the audit trail.
        let mut conn = self.get_conn().await?;
        let actor_user = UserRow::find_by_id(&mut conn, *actor)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id: org_id,
                event_type: crate::domain::org_security_audit_log::SecurityEventType::MemberLeft,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor },
                actor_email: Some(actor_user.email.clone()),
                target_user_id: Some(actor.as_uuid()),
                target_email: Some(actor_user.email),
                metadata: None,
                context: request_context,
            },
        )
        .await;

        Ok(())
    }

    async fn remove_member(
        &self,
        actor: UserId,
        target_user_id: UserId,
        org_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        // Look up emails up front while both users are guaranteed to exist —
        // the delete below races with anything that might purge the user row.
        let actor_user = UserRow::find_by_id(&mut conn, *actor)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let target_user = UserRow::find_by_id(&mut conn, *target_user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let members = &*self.hooks.members;
        self.transaction_with(&mut conn, |conn| {
            async move {
                if actor == target_user_id {
                    return Err(StoreError::InvalidArgument(
                        "Cannot remove yourself. Use LeaveOrganization instead.".to_string(),
                    )
                    .into());
                }

                OrganizationMemberRow::delete_member(conn, *target_user_id, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                release_member_accounts(conn, members, org_id, target_user_id).await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id: org_id,
                event_type: crate::domain::org_security_audit_log::SecurityEventType::MemberRemoved,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor },
                actor_email: Some(actor_user.email),
                target_user_id: Some(target_user_id.as_uuid()),
                target_email: Some(target_user.email),
                metadata: None,
                context: request_context,
            },
        )
        .await;

        Ok(())
    }

    async fn list_organizations_for_user(&self, user_id: UserId) -> StoreResult<Vec<Organization>> {
        let mut conn = self.get_conn().await?;

        let orgs = OrganizationRow::list_by_user_id(&mut conn, *user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(orgs.into_iter().map(Into::into).collect())
    }

    async fn get_organization_by_id(&self, id: OrganizationId) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::get_by_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(org.into())
    }

    async fn get_organization_by_tenant_id(&self, id: &TenantId) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::get_by_tenant_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(org.into())
    }

    async fn get_organizations_with_tenants_by_id(
        &self,
        id: OrganizationId,
    ) -> StoreResult<OrganizationWithTenants> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::get_by_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let tenants = TenantRow::list_by_organization_id(&mut conn, id).await?;

        Ok(OrganizationWithTenants {
            organization: org.into(),
            tenants: tenants.into_iter().map(Into::into).collect(),
        })
    }

    async fn get_organizations_by_slug(&self, slug: String) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::find_by_slug(&mut conn, slug)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(org.into())
    }

    async fn get_organizations_by_ids(
        &self,
        ids: &[OrganizationId],
    ) -> StoreResult<Vec<Organization>> {
        let mut conn = self.get_conn().await?;

        let orgs = OrganizationRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(orgs.into_iter().map(Into::into).collect())
    }

    async fn insert_express_organization(
        &self,
        organization: OrganizationNew,
        user_id: UserId,
        tenant_environment: TenantEnvironmentEnum,
    ) -> StoreResult<OrganizationWithTenants> {
        let mut conn = self.get_conn().await?;

        let tenant_name = match tenant_environment {
            TenantEnvironmentEnum::Production => "Production",
            TenantEnvironmentEnum::Staging => "Staging",
            TenantEnvironmentEnum::Qa => "QA",
            TenantEnvironmentEnum::Development => "Development",
            TenantEnvironmentEnum::Sandbox => "Sandbox",
            TenantEnvironmentEnum::Demo => "Demo",
        }
        .to_string();

        let org = OrganizationRowNew {
            id: OrganizationId::new(),
            slug: Organization::new_slug(),
            trade_name: organization.trade_name.clone(),
            default_country: organization.country.clone(),
            is_express: true,
        };

        let org_member = OrganizationMemberRow {
            user_id,
            organization_id: org.id,
            role: OrganizationUserRole::Admin,
            role_id: None,
        };

        let tenant_new = TenantNew {
            name: tenant_name,
            environment: tenant_environment,
            email_delivery_mode: None,
            invoicing_entity: None,
        };

        let (org_created, tenant_created) = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let org_created = OrganizationRowNew::insert(&org, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    OrganizationMemberRow::insert(&org_member, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let tenant_created = self
                        .internal
                        .insert_tenant_with_default_entities(
                            conn,
                            tenant_new,
                            org.id,
                            org.trade_name.clone(),
                            org.default_country.clone(),
                            organization.invoicing_entity.unwrap_or_default(),
                        )
                        .await?;

                    Ok((org_created, tenant_created))
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::organization_created(
                Actor::User { id: user_id },
                org_created.id,
            ))
            .await;

        Ok(OrganizationWithTenants {
            organization: org_created.into(),
            tenants: vec![tenant_created],
        })
    }

    async fn schedule_organization_deletion(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<()> {
        // Admin authorization is enforced at the gRPC layer (require_admin).

        let now = Utc::now().naive_utc();
        let tenant_ids = self
            .transaction_bumping_tenants(|conn| {
                async move {
                    let tenant_ids =
                        mark_organization_for_deletion(conn, organization_id, now).await?;
                    Ok((tenant_ids.clone(), tenant_ids))
                }
                .scope_boxed()
            })
            .await?;

        for tid in &tenant_ids {
            crate::repositories::tenants::invalidate_tenant_cache(tid).await;
        }

        let mut conn = self.get_conn().await?;
        let actor_email = UserRow::find_by_id(&mut conn, actor.as_uuid())
            .await
            .ok()
            .map(|u| u.email);
        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id,
                event_type:
                    crate::domain::org_security_audit_log::SecurityEventType::OrganizationDeletionScheduled,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor },
                actor_email,
                target_user_id: None,
                target_email: None,
                metadata: Some(serde_json::json!({ "tenant_count": tenant_ids.len() })),
                context: request_context,
            },
        )
        .await;

        info!(
            organization_id = %organization_id,
            actor = %actor,
            "organization scheduled for deletion"
        );

        Ok(())
    }

    async fn delete_empty_archived_organizations(&self) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        OrganizationRow::delete_empty_archived(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn set_organization_mfa_required(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        required: bool,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let member = OrganizationMemberRow::find_by_user_and_org(
            &mut conn,
            actor.as_uuid(),
            organization_id,
        )
        .await
        .map_err(|_| {
            StoreError::InvalidArgument("user is not a member of this organization".into())
        })?;

        if member.role != OrganizationUserRole::Admin {
            return Err(StoreError::InvalidArgument(
                "only organization admins can change MFA enforcement".into(),
            )
            .into());
        }

        let previous = OrganizationRow::get_by_id(&mut conn, organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if previous.mfa_required == required {
            // Idempotent no-op: don't pollute the audit log with a redundant entry.
            return Ok(previous.into());
        }

        OrganizationRow::update_mfa_required(&mut conn, organization_id, required)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let actor_user = UserRow::find_by_id(&mut conn, actor.as_uuid())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let event_type = if required {
            crate::domain::org_security_audit_log::SecurityEventType::OrganizationMfaRequiredEnabled
        } else {
            crate::domain::org_security_audit_log::SecurityEventType::OrganizationMfaRequiredDisabled
        };

        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id,
                event_type,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor },
                actor_email: Some(actor_user.email),
                target_user_id: None,
                target_email: None,
                metadata: None,
                context: request_context,
            },
        )
        .await;

        let updated = OrganizationRow::get_by_id(&mut conn, organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.into())
    }
}

impl Store {
    async fn send_invite_email(
        &self,
        invite_id: OrganizationInviteId,
        org_name: &str,
        inviter_email: &str,
        role: &OrganizationUserRole,
        recipient_email: String,
    ) {
        let ttl_days = self.settings.invite_ttl_days;
        let invite_url = format!(
            "{}/invite?token={}",
            self.settings.public_url.trim_end_matches('/'),
            invite_id
        );
        let _ = self
            .mailer
            .send_org_invite(meteroid_mailer::model::OrgInvite {
                org_name: org_name.to_string(),
                inviter_name: inviter_email.to_string(),
                role: format!("{}", role),
                invite_url,
                expires_in: format!("{} days", ttl_days),
                recipient: meteroid_mailer::model::EmailRecipient {
                    email: recipient_email,
                    first_name: None,
                    last_name: None,
                },
            })
            .await
            .map_err(|e| log::error!("Failed to send invite email: {e:?}"));
    }
}

/// `manages` names the guarded area in the rejection message.
pub async fn require_org_admin(
    conn: &mut PgConn,
    actor: UserId,
    organization_id: OrganizationId,
    manages: &str,
) -> StoreResult<()> {
    let member =
        OrganizationMemberRow::find_by_user_and_org(conn, actor.as_uuid(), organization_id)
            .await
            .map_err(|_| {
                StoreError::InvalidArgument("user is not a member of this organization".into())
            })?;
    if member.role != OrganizationUserRole::Admin {
        error_stack::bail!(StoreError::InvalidArgument(format!(
            "only organization admins can manage {manages}"
        )));
    }
    Ok(())
}

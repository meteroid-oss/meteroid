use crate::StoreResult;
use crate::domain::enums::TenantEnvironmentEnum;
use crate::domain::organizations::{InviteDetails, OrganizationInvite};
use crate::domain::{
    InstanceFlags, Organization, OrganizationNew, OrganizationWithTenants, TenantNew,
};
use crate::errors::StoreError;
use crate::store::Store;
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, OrganizationId, OrganizationInviteId, TenantId, UserId};
use common_eventbus::Event;
use diesel_models::enums::OrganizationUserRole;
use diesel_models::organization_invites::{OrganizationInviteRow, OrganizationInviteRowNew};
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::{OrganizationRow, OrganizationRowNew};
use diesel_models::tenants::TenantRow;
use diesel_models::users::UserRow;
use error_stack::Report;
use meteroid_oauth::model::OauthProvider;
use scoped_futures::ScopedFutureExt;
use tracing_log::log;

#[async_trait::async_trait]
pub trait OrganizationsInterface {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        actor: UserId,
    ) -> StoreResult<OrganizationWithTenants>;

    async fn get_instance(&self) -> StoreResult<InstanceFlags>;

    async fn invite_member(
        &self,
        org_id: OrganizationId,
        actor_id: UserId,
        invited_email: String,
        role: OrganizationUserRole,
    ) -> StoreResult<OrganizationInvite>;

    async fn resend_invite(
        &self,
        invite_id: OrganizationInviteId,
        actor_id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<()>;

    async fn revoke_invite(
        &self,
        invite_id: OrganizationInviteId,
        org_id: OrganizationId,
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
    ) -> StoreResult<Organization>;

    async fn leave_organization(&self, actor: UserId, org_id: OrganizationId) -> StoreResult<()>;

    async fn remove_member(
        &self,
        actor: UserId,
        target_user_id: UserId,
        org_id: OrganizationId,
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
}

#[async_trait::async_trait]
impl OrganizationsInterface for Store {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        user_id: UserId,
    ) -> StoreResult<OrganizationWithTenants> {
        let mut conn = self.get_conn().await?;

        if !self.settings.multi_organization_enabled {
            let exists = OrganizationRow::exists(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if exists {
                return Err(StoreError::InvalidArgument(
                    "This instance does not allow mutiple organizations".to_string(),
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
        };

        let dev_tenant_new = TenantNew {
            name: "Development".to_string(),
            environment: TenantEnvironmentEnum::Development,
            disable_emails: None,
            invoicing_entity: None,
        };

        let _country = &org.default_country.clone();

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
                            vec![],
                            organization.invoicing_entity.unwrap_or_default(),
                        )
                        .await?;

                    Ok((org_created, tenant_created))
                }
                .scope_boxed()
            })
            .await?;

        if let Some(_billing) = &self.billing {
            // enterprise placeholder
        }

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

    async fn get_instance(&self) -> StoreResult<InstanceFlags> {
        let mut conn = self.get_conn().await?;

        let (multi_organization_enabled, instance_initiated) =
            if self.settings.multi_organization_enabled {
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
    ) -> StoreResult<OrganizationInvite> {
        let mut conn = self.get_conn().await?;

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

        if self.settings.mailer_enabled {
            self.send_invite_email(
                row.id,
                &org.trade_name,
                &actor_user.email,
                &role,
                invited_email,
            )
            .await;
        }

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

        if self.settings.mailer_enabled {
            self.send_invite_email(
                invite.id,
                &org.trade_name,
                &actor_user.email,
                &invite.role,
                invite.invited_email,
            )
            .await;
        }

        Ok(())
    }

    async fn revoke_invite(
        &self,
        invite_id: OrganizationInviteId,
        org_id: OrganizationId,
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

        Ok(InviteDetails {
            organization_name: org.trade_name,
            role: invite.role,
            invited_email: invite.invited_email,
        })
    }

    async fn accept_invite(
        &self,
        user_id: UserId,
        invite_id: OrganizationInviteId,
    ) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, |conn| {
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

                Ok(org.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn leave_organization(&self, actor: UserId, org_id: OrganizationId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

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

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn remove_member(
        &self,
        actor: UserId,
        target_user_id: UserId,
        org_id: OrganizationId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, |conn| {
            async move {
                let actor_member =
                    OrganizationMemberRow::find_by_user_and_org(conn, *actor, org_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                if !matches!(actor_member.role, OrganizationUserRole::Admin) {
                    return Err(StoreError::Forbidden(
                        "Only admins can remove members".to_string(),
                    )
                    .into());
                }

                if actor == target_user_id {
                    return Err(StoreError::InvalidArgument(
                        "Cannot remove yourself. Use LeaveOrganization instead.".to_string(),
                    )
                    .into());
                }

                OrganizationMemberRow::delete_member(conn, *target_user_id, org_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
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
        };

        let tenant_new = TenantNew {
            name: tenant_name,
            environment: tenant_environment,
            disable_emails: None,
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
                            vec![],
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

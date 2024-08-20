use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use tracing_log::log;
use uuid::Uuid;

use common_eventbus::Event;
use common_utils::rng::{BASE62_ALPHABET, UPPER_ALPHANUMERIC};
use diesel_models::enums::{OrganizationUserRole, TenantEnvironmentEnum};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::organization_members::OrganizationMemberRow;
use diesel_models::organizations::{OrganizationRow, OrganizationRowNew};
use diesel_models::tenants::TenantRowNew;

use crate::domain::{InstanceFlags, Organization, OrganizationNew};
use crate::errors::StoreError;
use crate::repositories::customer_balance::CustomerBalance;
use crate::store::Store;
use crate::StoreResult;
use crate::utils::local_id::{IdType, LocalId};

#[async_trait::async_trait]
pub trait OrganizationsInterface {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        actor: Uuid,
    ) -> StoreResult<Organization>;

    async fn get_instance(&self) -> StoreResult<InstanceFlags>;
    async fn organization_get_or_create_invite_link(&self, organization_id: Uuid) -> StoreResult<String>;

    async fn list_organizations_for_user(&self, user_id: Uuid) -> StoreResult<Vec<Organization>>;
    async fn get_organizations_by_slug(&self, slug: String) -> StoreResult<Organization>;
}

#[async_trait::async_trait]
impl OrganizationsInterface for Store {
    async fn get_organizations_by_slug(&self, slug: String) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::find_by_slug(&mut conn, slug)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(org.into())
    }


    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        user_id: Uuid,
    ) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        if !self.settings.multi_organization_enabled {
            let count = OrganizationRow::count_all(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if count > 0 { return Err(StoreError::InitializationError.into()); }
        }

        let org = OrganizationRowNew {
            id: Uuid::now_v7(),
            slug: Organization::new_slug(),
            default_trade_name: organization.default_trade_name.clone(),
            default_country: organization.default_country.clone(),
        };

        let currency = crate::constants::COUNTRIES.iter().find(|x| x.code == &org.default_country).map(|x| x.currency)
            .ok_or(StoreError::ValueNotFound(format!("No currency found for country code {}", &organization.default_country)))?;

        // when we crate a tenant, we also insert an accounting entity
        let production_tenant = TenantRowNew {
            id: Uuid::now_v7(),
            organization_id: org.id,
            currency: currency.to_string(),
            name: "Production".to_string(),
            slug: "prod".to_string(),
            environment: TenantEnvironmentEnum::Production,
        };

        let invoicing_entity = InvoicingEntityRow {
            id: Uuid::now_v7(),
            local_id: LocalId::generate_for(IdType::InvoicingEntity),
            is_default: true,
            legal_name: organization.default_trade_name.clone(),
            invoice_number_pattern: "INV-{number}".to_string(),
            next_invoice_number: 1,
            next_credit_note_number: 1,
            grace_period_hours: 24,
            net_terms: 30,
            country: organization.default_country.clone(),
            currency: currency.to_string(),
            tenant_id: production_tenant.id,
            //
            invoice_footer_info: None,
            invoice_footer_legal: None,
            logo_attachment_id: None,
            brand_color: None,
            address_line1: None,
            address_line2: None,
            zip_code: None,
            state: None,
            city: None,
            tax_id: None, // TODO rename to vat_number
        };


        // TODO trigger sandbox init ?

        //
        // let sandbox_tenant = TenantRowNew {
        //     id: Uuid::now_v7(),
        //     organization_id: org.id,
        //     currency: currency.to_string(),
        //     name: "Sandbox".to_string(),
        //     slug: "sandbox".to_string(),
        //     environment: TenantEnvironmentEnum::Sandbox,
        // };

        let org_member = OrganizationMemberRow {
            user_id,
            organization_id: org.id,
            role: OrganizationUserRole::Admin,
        };


        let org_created = self.transaction_with(&mut conn, |conn| {
            async move {
                let org_created = OrganizationRowNew::insert(&org, conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                TenantRowNew::insert(&production_tenant, conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                InvoicingEntityRow::insert(&invoicing_entity, conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                OrganizationMemberRow::insert(&org_member, conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(org_created)
            }
                .scope_boxed()
        })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::organization_created(user_id, org_created.id.clone()))
            .await;

        Ok(org_created.into())
    }

    async fn get_instance(&self) -> StoreResult<InstanceFlags> {
        let mut conn = self.get_conn().await?;

        if self.settings.multi_organization_enabled {
            Ok(InstanceFlags {
                multi_organization_enabled: true,
                instance_initiated: true,
            })
        } else {
            // single organization
            let count = OrganizationRow::count_all(&mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            Ok(InstanceFlags {
                multi_organization_enabled: false,
                instance_initiated: count > 0,
            })
        }
    }

    async fn organization_get_or_create_invite_link(&self, organization_id: Uuid) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;

        let org = OrganizationRow::get_by_id(&mut conn, organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        match org.invite_link_hash {
            Some(hash) => Ok(hash),
            None => {
                log::warn!("Organization invite link is not set - creating new one");

                let invite_hash = nanoid::nanoid!(32, &BASE62_ALPHABET);

                // we could add some expiry (configurable enum 1 hour/day/week/month/forever) default week, via a signature to avoid invalidating old links on view (unless that is requested)
                // ex: keep the hash to allow invalidation, drop the find_by_invite_link, encode org id + expiry + hash + signature in the resulting data provided to user
                let _ = OrganizationRow::update_invite_link(&mut conn, org.id, &invite_hash)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(invite_hash)
            }
        }
    }

    async fn list_organizations_for_user(&self, user_id: Uuid) -> StoreResult<Vec<Organization>> {
        let mut conn = self.get_conn().await?;

        let orgs = OrganizationRow::list_by_user_id(&mut conn, user_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(orgs.into_iter().map(Into::into).collect())
    }
}

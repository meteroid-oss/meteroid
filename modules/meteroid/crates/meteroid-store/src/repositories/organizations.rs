use error_stack::Report;
use tracing_log::log;
use uuid::Uuid;

use common_eventbus::Event;
use diesel_models::organizations::{OrganizationRow, OrganizationRowNew};

use crate::domain::{Organization, OrganizationNew};
use crate::errors::StoreError;
use crate::store::Store;
use crate::StoreResult;

#[async_trait::async_trait]
pub trait OrganizationsInterface {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        actor: Uuid,
    ) -> StoreResult<Organization>;

    async fn find_organization_as_instance(&self) -> StoreResult<Option<Organization>>;
    async fn organization_get_or_create_invite_link(&self) -> StoreResult<String>;
}

#[async_trait::async_trait]
impl OrganizationsInterface for Store {
    async fn insert_organization(
        &self,
        organization: OrganizationNew,
        actor: Uuid,
    ) -> StoreResult<Organization> {
        let mut conn = self.get_conn().await?;

        let insertable = OrganizationRowNew {
            id: Uuid::now_v7(),
            name: organization.name,
            slug: organization.slug,
        };

        let res: Organization = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let _ = self
            .eventbus
            .publish(Event::instance_inited(actor, res.id))
            .await;

        Ok(res)
    }

    async fn find_organization_as_instance(&self) -> StoreResult<Option<Organization>> {
        let mut conn = self.get_conn().await?;

        OrganizationRow::find_all(&mut conn)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
            .and_then(|v: Vec<Organization>| {
                if v.len() == 0 {
                    Ok(None)
                } else if v.len() == 1 {
                    Ok(Some(v.into_iter().next().unwrap()))
                } else {
                    v.iter()
                        .find(|x| x.slug == "instance")
                        .map(|x| Some(x.clone()))
                        .ok_or_else(|| Report::from(StoreError::InitializationError))
                }
            })
    }

    async fn organization_get_or_create_invite_link(&self) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;

        let (org_id, maybe_hash) =
            self.find_organization_as_instance()
                .await
                .and_then(|v| match v {
                    Some(org) => Ok((org.id, org.invite_link_hash)),
                    None => Err(Report::from(StoreError::InitializationError)),
                })?;

        match maybe_hash {
            Some(hash) => Ok(hash),
            None => {
                log::warn!("Organization invite link is not set - creating new one");

                let id = Uuid::new_v4();
                let hash = base62::encode_alternative(id.as_u128());

                let _ = OrganizationRow::update_invite_link(&mut conn, org_id, &hash)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(hash)
            }
        }
    }
}

use crate::domain::{InvoicingEntityNew, OrganizationWithTenants, Tenant, TenantNew, TenantUpdate};
use cached::proc_macro::cached;
use cached::Cached;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;

use crate::constants::{Currencies, Currency};
use crate::errors::StoreError;
use crate::repositories::OrganizationsInterface;
use crate::store::{PgConn, Store, StoreInternal};
use crate::{domain, StoreResult};
use diesel_models::organizations::OrganizationRow;
use diesel_models::tenants::{TenantRow, TenantRowNew, TenantRowPatch};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait TenantInterface {
    async fn insert_tenant(&self, tenant: TenantNew, organization_id: Uuid) -> StoreResult<Tenant>;
    async fn update_tenant(
        &self,
        tenant: TenantUpdate,
        organization_id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<Tenant>;
    async fn find_tenant_by_id_and_organization(
        &self,
        tenant_id: Uuid,
        organization_id: Uuid,
    ) -> StoreResult<Tenant>;
    async fn find_tenant_by_slug_and_organization_slug(
        &self,
        slug: String,
        organization_slug: String,
    ) -> StoreResult<Tenant>;
    async fn list_tenants_by_organization_id(
        &self,
        organization_id: Uuid,
    ) -> StoreResult<Vec<Tenant>>;

    async fn get_reporting_currency_by_tenant_id(&self, tenant_id: Uuid) -> StoreResult<Currency>;

    async fn list_tenant_currencies_with_customer_count(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<(String, u64)>>;

    async fn list_tenant_currencies(&self, tenant_id: Uuid) -> StoreResult<Vec<String>>;

    async fn add_tenant_currency(&self, tenant_id: Uuid, currency: String) -> StoreResult<()>;

    async fn remove_tenant_currency(&self, tenant_id: Uuid, currency: String) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(&self, tenant: TenantNew, organization_id: Uuid) -> StoreResult<Tenant> {
        let OrganizationWithTenants {
            organization,
            tenants,
        } = self
            .get_organizations_with_tenants_by_id(organization_id)
            .await?;

        self.transaction(|conn| {
            async move {
                self.internal
                    .insert_tenant_with_default_entities(
                        conn,
                        tenant,
                        organization_id,
                        organization.trade_name.clone(),
                        organization.default_country.clone(),
                        tenants.iter().map(|x| x.slug.clone()).collect(),
                        InvoicingEntityNew::default(),
                    )
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_tenant(
        &self,
        tenant: TenantUpdate,
        organization_id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<Tenant> {
        let res = self
            .transaction(|conn| {
                async move {
                    // we update org trade name

                    if let Some(trade_name) = &tenant.trade_name {
                        OrganizationRow::update_trade_name(conn, organization_id, trade_name)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    let patch: TenantRowPatch = tenant.into();

                    let updated_tenant = patch
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(updated_tenant.into())
                }
                .scope_boxed()
            })
            .await?;

        Ok(res)
    }

    async fn find_tenant_by_id_and_organization(
        &self,
        tenant_id: Uuid,
        organization_id: Uuid,
    ) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_id_and_organization_id(&mut conn, tenant_id, organization_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_tenant_by_slug_and_organization_slug(
        &self,
        slug: String,
        organization_slug: String,
    ) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_slug_and_organization_slug(&mut conn, slug, organization_slug)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_tenants_by_organization_id(
        &self,
        organization_id: Uuid,
    ) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_by_organization_id(&mut conn, organization_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn get_reporting_currency_by_tenant_id(&self, tenant_id: Uuid) -> StoreResult<Currency> {
        let mut conn = self.get_conn().await?;

        self.internal
            .get_reporting_currency_by_tenant_id(&mut conn, tenant_id)
            .await
    }

    async fn list_tenant_currencies_with_customer_count(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<(String, u64)>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_tenant_currencies_with_customer_count(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn list_tenant_currencies(&self, tenant_id: Uuid) -> StoreResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        TenantRow::list_tenant_currencies(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn add_tenant_currency(&self, tenant_id: Uuid, currency: String) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        TenantRow::add_available_currency(&mut conn, tenant_id, currency)
            .await
            .map_err(Into::into)
    }

    async fn remove_tenant_currency(&self, tenant_id: Uuid, currency: String) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        TenantRow::remove_available_currency(&mut conn, tenant_id, currency)
            .await
            .map_err(Into::into)
    }
}

impl StoreInternal {
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_tenant_with_default_entities(
        &self,
        conn: &mut PgConn,
        tenant: TenantNew,
        organization_id: Uuid,
        trade_name: String,
        country: String,
        existing_tenant_slugs: Vec<String>,
        invoicing_entity: InvoicingEntityNew,
    ) -> StoreResult<Tenant> {
        let currency = self.get_currency_from_country(&country)?;

        let base_slug = match tenant.environment {
            domain::enums::TenantEnvironmentEnum::Production => "prod",
            domain::enums::TenantEnvironmentEnum::Staging => "staging",
            domain::enums::TenantEnvironmentEnum::Qa => "qa",
            domain::enums::TenantEnvironmentEnum::Development => "dev",
            domain::enums::TenantEnvironmentEnum::Sandbox => "sandbox",
            domain::enums::TenantEnvironmentEnum::Demo => "demo",
        };

        let mut slug = base_slug.to_string();
        let mut i = 1;
        while existing_tenant_slugs.contains(&slug) {
            slug = format!("{}-{}", base_slug, i);
            i += 1;
        }

        let insertable_tenant: TenantRowNew = TenantRowNew {
            id: Uuid::now_v7(),
            environment: tenant.environment.into(),
            reporting_currency: currency.clone(),
            name: tenant.name,
            slug,
            organization_id,
            available_currencies: vec![Some(currency)],
        };

        let inserted: Tenant = insertable_tenant
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let _ = self
            .create_invoicing_entity(conn, invoicing_entity, inserted.id, country, trade_name)
            .await?;

        // TODO think about making it easier in the api (default with optional)
        let _ = self
            .insert_product_family(
                conn,
                domain::ProductFamilyNew {
                    name: "Default".to_string(),
                    local_id: "default".to_string(),
                    tenant_id: inserted.id,
                },
            )
            .await?;

        Ok(inserted)
    }

    pub async fn get_reporting_currency_by_tenant_id(
        &self,
        conn: &mut PgConn,
        tenant_id: Uuid,
    ) -> StoreResult<Currency> {
        get_reporting_currency_by_tenant_id_cached(conn, tenant_id).await
    }
}

#[cached(
    result = true,
    size = 100,
    time = 3600, // 1h
    key = "Uuid",
    convert = r#"{ tenant_id }"#
)]
async fn get_reporting_currency_by_tenant_id_cached(
    conn: &mut PgConn,
    tenant_id: Uuid,
) -> StoreResult<Currency> {
    let currency = TenantRow::get_reporting_currency_by_id(conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let res = Currencies::resolve_currency(&currency)
        .ok_or_else(|| {
            StoreError::ValueNotFound(format!("Currency not found for code {}", currency))
        })
        .cloned()?;

    Ok(res)
}

pub async fn invalidate_reporting_currency_cache(tenant_id: &Uuid) {
    GET_REPORTING_CURRENCY_BY_TENANT_ID_CACHED
        .lock()
        .await
        .cache_remove(tenant_id);
}

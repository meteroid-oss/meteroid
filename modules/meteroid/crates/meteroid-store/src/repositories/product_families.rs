use crate::domain::{PaginatedVec, PaginationRequest, ProductFamily};
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use crate::{StoreResult, domain};
use common_domain::actor::Actor;
use common_domain::ids::{ProductFamilyId, TenantId};
use common_eventbus::Event;
use diesel_models::product_families::{ProductFamilyRow, ProductFamilyRowNew};
use error_stack::{IntoReport, Report};

#[async_trait::async_trait]
pub trait ProductFamilyInterface {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
        actor: Option<Actor>,
    ) -> StoreResult<domain::ProductFamily>;

    /// `insert_product_family` under `code`, unique per tenant (ADR-0013); none: its name's slug.
    async fn insert_product_family_with_code(
        &self,
        product_family: domain::ProductFamilyNew,
        code: Option<String>,
        actor: Option<Actor>,
    ) -> StoreResult<domain::ProductFamily>;

    async fn list_product_families(
        &self,
        auth_tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<domain::ProductFamily>>;

    /// The tenant's families after `after` in id order, at most `limit`, the archived ones only
    /// when asked: a keyset page of API v2.
    async fn list_product_families_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<ProductFamilyId>,
        limit: usize,
    ) -> StoreResult<Vec<domain::ProductFamily>>;

    async fn find_product_family_by_id(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily>;

    async fn rename_product_family(
        &self,
        id: ProductFamilyId,
        name: String,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily>;

    /// The tenant's first live family: where a product or a plan naming none goes.
    async fn find_default_product_family(
        &self,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily>;

    /// Off the catalog for new products, plans and metrics; what is in it keeps selling and
    /// billing. Archiving an archived family changes nothing.
    async fn archive_product_family(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily>;

    async fn unarchive_product_family(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily>;
}

/// Refuses a new use of an archived family: a product, a plan or a metric put in it.
pub(crate) fn ensure_family_live(family: &ProductFamilyRow) -> StoreResult<()> {
    if family.archived_at.is_some() {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "product family {} is archived",
            family.code
        ))));
    }
    Ok(())
}

impl StoreInternal {
    pub async fn insert_product_family(
        &self,
        conn: &mut PgConn,
        product_family: domain::ProductFamilyNew,
    ) -> StoreResult<domain::ProductFamily> {
        let insertable_product_family: ProductFamilyRowNew = product_family.into();

        insertable_product_family
            .insert(conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}

#[async_trait::async_trait]
impl ProductFamilyInterface for Store {
    async fn insert_product_family(
        &self,
        product_family: domain::ProductFamilyNew,
        actor: Option<Actor>,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        let res = self
            .internal
            .insert_product_family(&mut conn, product_family)
            .await?;

        let _ = self
            .eventbus
            .publish(Event::product_family_created(actor, res.id, res.tenant_id))
            .await;

        Ok(res)
    }

    async fn insert_product_family_with_code(
        &self,
        product_family: domain::ProductFamilyNew,
        code: Option<String>,
        actor: Option<Actor>,
    ) -> StoreResult<domain::ProductFamily> {
        use common_domain::ids::BaseId;
        use scoped_futures::ScopedFutureExt;

        let Some(code) = code else {
            return self.insert_product_family(product_family, actor).await;
        };
        crate::domain::catalog_codes::validate_code(&code, ProductFamilyId::PREFIX)
            .map_err(Report::new)?;
        let res = self
            .transaction(|conn| {
                async move {
                    let mut family = self
                        .internal
                        .insert_product_family(conn, product_family)
                        .await?;
                    diesel_models::query::catalog_codes::set_product_family_code(
                        conn,
                        family.tenant_id,
                        family.id,
                        &code,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    family.code = code;
                    Ok(family)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::product_family_created(actor, res.id, res.tenant_id))
            .await;

        Ok(res)
    }

    async fn list_product_families(
        &self,
        auth_tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<domain::ProductFamily>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductFamilyRow::list(
            &mut conn,
            auth_tenant_id,
            pagination.into(),
            order_by.as_deref(),
            query,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<domain::ProductFamily> = PaginatedVec {
            items: rows.items.into_iter().map(Into::into).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn list_product_families_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<ProductFamilyId>,
        limit: usize,
    ) -> StoreResult<Vec<domain::ProductFamily>> {
        let mut conn = self.get_conn().await?;
        Ok(ProductFamilyRow::list_page(
            &mut conn,
            tenant_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    async fn find_product_family_by_id(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily> {
        let mut conn = self.get_conn().await?;

        ProductFamilyRow::find_by_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn rename_product_family(
        &self,
        id: ProductFamilyId,
        name: String,
        auth_tenant_id: TenantId,
    ) -> StoreResult<domain::ProductFamily> {
        let name = name.trim();
        if name.is_empty() {
            return Err(error_stack::Report::new(
                crate::errors::StoreError::InvalidArgument("a product line has a name".to_owned()),
            ));
        }
        let mut conn = self.get_conn().await?;
        ProductFamilyRow::rename(&mut conn, id, auth_tenant_id, name)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_default_product_family(
        &self,
        auth_tenant_id: TenantId,
    ) -> StoreResult<ProductFamily> {
        let mut conn = self.get_conn().await?;
        ProductFamilyRow::find_default(&mut conn, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .map(Into::into)
            .ok_or(StoreError::ValueNotFound("Default product family".to_string()).into_report())
    }

    async fn archive_product_family(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<ProductFamily> {
        let mut conn = self.get_conn().await?;
        let family = ProductFamilyRow::find_by_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if family.archived_at.is_some() {
            return Ok(family.into());
        }
        ProductFamilyRow::set_archived(
            &mut conn,
            id,
            auth_tenant_id,
            Some(common_clock::now().naive_utc()),
        )
        .await
        .map_err(Into::into)
        .map(Into::into)
    }

    async fn unarchive_product_family(
        &self,
        id: ProductFamilyId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<ProductFamily> {
        let mut conn = self.get_conn().await?;
        let family = ProductFamilyRow::find_by_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if family.archived_at.is_none() {
            return Ok(family.into());
        }
        ProductFamilyRow::set_archived(&mut conn, id, auth_tenant_id, None)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}

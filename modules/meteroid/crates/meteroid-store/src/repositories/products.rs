use std::collections::HashMap;

use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::FeeTypeEnum;
use crate::domain::outbox_event::{OutboxEvent, ProductEvent};
use crate::domain::plan_options::PlanOption;
use crate::domain::price_cells;
use crate::domain::price_components::PriceComponent;
use crate::domain::prices::FeeStructure;
use crate::domain::{
    PaginatedVec, PaginationRequest, Price, Product, ProductListFacts, ProductNew,
    ProductPlanUsage, ProductUsage, ProductWithLatestPrice, validate_product_appearance,
};
use crate::errors::StoreError;
use crate::repositories::add_ons::enrich_add_ons;
use crate::repositories::price_components::attach_prices;
use crate::store::Store;
use common_domain::ids::{
    BaseId, PlanVersionId, ProductFamilyId, ProductId, TaxCategoryId, TenantId,
};
use diesel_models::add_ons::AddOnRow;
use diesel_models::entitlements::FeatureRow;
use diesel_models::plan_version_options::PlanVersionOptionRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::{ProductRow, ProductRowNew, ProductRowPatch};
use diesel_models::tax_categories::TaxCategoryRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// A product may only reference a category the tenant can see (built-in, or its own).
pub(crate) async fn validate_tax_category(
    conn: &mut diesel_models::PgConn,
    tax_category_id: Option<TaxCategoryId>,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let Some(id) = tax_category_id else {
        return Ok(());
    };

    let available = TaxCategoryRow::is_available_for_tenant(conn, id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if available {
        Ok(())
    } else {
        Err(Report::new(StoreError::InvalidArgument(format!(
            "Unknown tax category: {id}"
        ))))
    }
}

/// A product's edit. How it is billed is its charges', never the product's.
#[derive(Clone, Debug)]
pub struct ProductUpdate {
    pub id: ProductId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    /// Patch: None leaves unchanged, Some(None) clears, Some(Some(id)) sets.
    pub tax_category_id: Option<Option<TaxCategoryId>>,
    /// Patch, as `tax_category_id`.
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
}

#[async_trait::async_trait]
pub trait ProductInterface {
    async fn create_product(&self, actor: Actor, product: ProductNew) -> StoreResult<Product>;
    async fn update_product(&self, actor: Actor, update: ProductUpdate) -> StoreResult<Product>;
    async fn find_product_by_id(
        &self,
        id: ProductId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Product>;
    async fn find_products_by_ids(
        &self,
        ids: &[ProductId],
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<Product>>;
    async fn list_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<Product>>;
    async fn search_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        catalog_only: bool,
        archived: bool,
        pagination: PaginationRequest,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<Product>>;

    async fn archive_product(
        &self,
        actor: Actor,
        id: ProductId,
        tenant_id: TenantId,
    ) -> StoreResult<Product>;

    async fn unarchive_product(&self, id: ProductId, tenant_id: TenantId) -> StoreResult<Product>;

    async fn list_products_with_latest_price(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        currency: &str,
        query: Option<&str>,
        catalog_only: bool,
        fee_types: Vec<FeeTypeEnum>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<ProductWithLatestPrice>>;

    /// The catalog list's facts of each product: latest prices, features and plans.
    async fn product_list_facts(
        &self,
        auth_tenant_id: TenantId,
        ids: &[ProductId],
    ) -> StoreResult<HashMap<ProductId, ProductListFacts>>;

    /// Where the product is sold: each live plan version with a charge of it, and its live
    /// add-ons.
    async fn product_usage(
        &self,
        auth_tenant_id: TenantId,
        product_id: ProductId,
    ) -> StoreResult<ProductUsage>;
}

#[async_trait::async_trait]
impl ProductInterface for Store {
    async fn create_product(&self, actor: Actor, product: ProductNew) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;
        let tenant_id = product.tenant_id;

        let family = ProductFamilyRow::find_by_id(&mut conn, product.family_id, product.tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        validate_tax_category(&mut conn, product.tax_category_id, product.tenant_id).await?;
        validate_product_appearance(product.icon.as_deref(), product.color.as_deref())?;

        let insertable: ProductRowNew = ProductRowNew {
            id: ProductId::new(),
            name: product.name,
            description: product.description,
            tenant_id: product.tenant_id,
            product_family_id: family.id,
            tax_category_id: product.tax_category_id,
            catalog: product.catalog,
            icon: product.icon,
            color: product.color,
        };

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                let product: Product = insertable
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(|row| row.try_into())?;

                let event = ProductEvent::new(
                    product.id,
                    product.tenant_id,
                    product.name.clone(),
                    product.description.clone(),
                    None,
                    product.product_family_id,
                    product.created_at,
                );
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::product_created(event)],
                    )
                    .await?;

                Ok(product)
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_product(&self, actor: Actor, update: ProductUpdate) -> StoreResult<Product> {
        let tenant_id = update.tenant_id;

        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                ProductRow::find_by_id_and_tenant_id(conn, update.id, update.tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                validate_tax_category(conn, update.tax_category_id.flatten(), update.tenant_id)
                    .await?;
                validate_product_appearance(
                    update.icon.as_ref().and_then(|i| i.as_deref()),
                    update.color.as_ref().and_then(|c| c.as_deref()),
                )?;

                let product: Product = ProductRowPatch {
                    id: update.id,
                    tenant_id: update.tenant_id,
                    name: update.name,
                    description: update.description,
                    tax_category_id: update.tax_category_id,
                    icon: update.icon,
                    color: update.color,
                }
                .patch(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(|row| row.try_into())?;

                let event = ProductEvent::new(
                    product.id,
                    product.tenant_id,
                    product.name.clone(),
                    product.description.clone(),
                    None,
                    product.product_family_id,
                    product.created_at,
                );
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::product_updated(event)],
                    )
                    .await?;

                Ok(product)
            }
            .scope_boxed()
        })
        .await
    }

    async fn find_product_by_id(
        &self,
        id: ProductId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        ProductRow::find_by_id_and_tenant_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn find_products_by_ids(
        &self,
        ids: &[ProductId],
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::list_by_ids(&mut conn, ids, auth_tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        catalog_only: bool,
        pagination: PaginationRequest,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::list(
            &mut conn,
            auth_tenant_id,
            family_id,
            catalog_only,
            &[],
            pagination.into(),
            order_by.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Result<Vec<Product>, _> =
            rows.items.into_iter().map(Product::try_from).collect();

        Ok(PaginatedVec {
            items: items?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn archive_product(
        &self,
        actor: Actor,
        id: ProductId,
        tenant_id: TenantId,
    ) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                let product: Product = ProductRow::archive(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(|row| row.try_into())?;

                let event = ProductEvent::new(
                    product.id,
                    product.tenant_id,
                    product.name.clone(),
                    product.description.clone(),
                    None,
                    product.product_family_id,
                    product.created_at,
                );
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::product_archived(event)],
                    )
                    .await?;

                Ok(product)
            }
            .scope_boxed()
        })
        .await
    }

    async fn unarchive_product(&self, id: ProductId, tenant_id: TenantId) -> StoreResult<Product> {
        let mut conn = self.get_conn().await?;

        ProductRow::unarchive(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|row| row.try_into())
    }

    async fn search_products(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        query: &str,
        catalog_only: bool,
        archived: bool,
        pagination: PaginationRequest,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<Product>> {
        let mut conn = self.get_conn().await?;

        let rows = ProductRow::search(
            &mut conn,
            auth_tenant_id,
            family_id,
            query,
            catalog_only,
            archived,
            &[],
            pagination.into(),
            order_by.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Result<Vec<Product>, _> =
            rows.items.into_iter().map(Product::try_from).collect();

        Ok(PaginatedVec {
            items: items?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_products_with_latest_price(
        &self,
        auth_tenant_id: TenantId,
        family_id: Option<ProductFamilyId>,
        currency: &str,
        query: Option<&str>,
        catalog_only: bool,
        fee_types: Vec<FeeTypeEnum>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<ProductWithLatestPrice>> {
        let mut conn = self.get_conn().await?;

        let db_fee_types: Vec<diesel_models::enums::FeeTypeEnum> =
            fee_types.into_iter().map(Into::into).collect();

        // Step 1: Fetch products (search or list)
        let rows = match query {
            Some(q) if !q.is_empty() => {
                ProductRow::search(
                    &mut conn,
                    auth_tenant_id,
                    family_id,
                    q,
                    catalog_only,
                    false,
                    &db_fee_types,
                    pagination.into(),
                    Some("name.asc"),
                )
                .await
            }
            _ => {
                ProductRow::list(
                    &mut conn,
                    auth_tenant_id,
                    family_id,
                    catalog_only,
                    &db_fee_types,
                    pagination.into(),
                    Some("name.asc"),
                )
                .await
            }
        }
        .map_err(Into::<Report<StoreError>>::into)?;

        let products: Vec<Product> = rows
            .items
            .into_iter()
            .map(Product::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        // Step 2: Batch-fetch latest price per product for the given currency
        let product_ids: Vec<ProductId> = products.iter().map(|p| p.id).collect();
        let price_rows = PriceRow::latest_by_product_ids_and_currency(
            &mut conn,
            &product_ids,
            currency,
            auth_tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut price_map: HashMap<ProductId, Price> = HashMap::new();
        for row in price_rows {
            let product_id = row.product_id;
            let price = Price::try_from(row)?;
            price_map.insert(product_id, price);
        }

        // Step 3: Zip products with their latest price
        let items: Vec<ProductWithLatestPrice> = products
            .into_iter()
            .map(|product| {
                let latest_price = price_map.remove(&product.id);
                ProductWithLatestPrice {
                    product,
                    latest_price,
                }
            })
            .collect();

        Ok(PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn product_list_facts(
        &self,
        auth_tenant_id: TenantId,
        ids: &[ProductId],
    ) -> StoreResult<HashMap<ProductId, ProductListFacts>> {
        let mut conn = self.get_conn().await?;

        let price_rows =
            PriceRow::latest_per_currency_by_product_ids(&mut conn, ids, auth_tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let features = FeatureRow::count_by_products(&mut conn, auth_tenant_id, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let plans = ProductRow::plan_counts(&mut conn, auth_tenant_id, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mut structures = charge_structures_of_products(&mut conn, auth_tenant_id, ids).await?;

        let mut facts: HashMap<ProductId, ProductListFacts> = ids
            .iter()
            .map(|id| {
                let fact = ProductListFacts {
                    latest_prices: vec![],
                    feature_count: features.get(id).copied().unwrap_or(0),
                    plan_count: plans.get(id).copied().unwrap_or(0),
                    charge_structures: structures.remove(id).unwrap_or_default(),
                };
                (*id, fact)
            })
            .collect();
        for row in price_rows {
            let product_id = row.product_id;
            let price = Price::try_from(row)?;
            if let Some(fact) = facts.get_mut(&product_id) {
                fact.latest_prices.push(price);
            }
        }

        Ok(facts)
    }

    async fn product_usage(
        &self,
        auth_tenant_id: TenantId,
        product_id: ProductId,
    ) -> StoreResult<ProductUsage> {
        let mut conn = self.get_conn().await?;

        let versions = ProductRow::live_plan_versions(&mut conn, auth_tenant_id, product_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let version_ids: Vec<PlanVersionId> = versions.iter().map(|v| v.plan_version_id).collect();

        // Every charge of the versions, so a currency only another charge has counts as a gap.
        let rows =
            PriceComponentRow::get_by_plan_version_ids(&mut conn, &version_ids, auth_tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let mut version_of = Vec::new();
        let mut components: Vec<PriceComponent> = Vec::new();
        for (version_id, rows) in rows {
            for row in rows {
                version_of.push(version_id);
                components.push(row.try_into()?);
            }
        }
        attach_prices(&mut conn, auth_tenant_id, &mut components).await?;
        let mut by_version: HashMap<PlanVersionId, Vec<PriceComponent>> = HashMap::new();
        for (version_id, component) in version_of.into_iter().zip(components) {
            by_version.entry(version_id).or_default().push(component);
        }

        let mut options: HashMap<PlanVersionId, Vec<PlanOption>> = HashMap::new();
        for row in
            PlanVersionOptionRow::list_by_plan_version_ids(&mut conn, auth_tenant_id, &version_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            options
                .entry(row.plan_version_id)
                .or_default()
                .push(row.into());
        }

        let plans = versions
            .into_iter()
            .map(|v| {
                let all = by_version.remove(&v.plan_version_id).unwrap_or_default();
                let currencies = price_cells::currencies(&all);
                let mut own: Vec<PriceComponent> = all
                    .into_iter()
                    .filter(|c| c.product_id == Some(product_id))
                    .collect();
                own.sort_by(|a, b| a.name.cmp(&b.name));
                ProductPlanUsage {
                    plan_id: v.plan_id,
                    plan_name: v.plan_name,
                    plan_version_id: v.plan_version_id,
                    version: v.version,
                    is_draft: v.is_draft,
                    options: options.remove(&v.plan_version_id).unwrap_or_default(),
                    currencies,
                    components: own,
                }
            })
            .collect();

        let add_on_rows = AddOnRow::list_live_by_product(&mut conn, auth_tenant_id, product_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let add_ons = enrich_add_ons(&mut conn, add_on_rows, auth_tenant_id).await?;

        Ok(ProductUsage { plans, add_ons })
    }
}

/// How the live charges of each product bill it, each way once.
pub(crate) async fn charge_structures_of_products(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    ids: &[ProductId],
) -> StoreResult<HashMap<ProductId, Vec<FeeStructure>>> {
    let mut by_product: HashMap<ProductId, Vec<FeeStructure>> = HashMap::new();
    for (product_id, json) in
        diesel_models::query::price_components::charge_structures_of_products(conn, tenant_id, ids)
            .await?
    {
        let structure: FeeStructure = serde_json::from_value(json).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize FeeStructure".to_string(),
                e,
            ))
        })?;
        let known = by_product.entry(product_id).or_default();
        if !known.iter().any(|other| other.same_shape(&structure)) {
            known.push(structure);
        }
    }
    Ok(by_product)
}

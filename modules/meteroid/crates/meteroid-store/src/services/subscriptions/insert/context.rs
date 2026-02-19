use crate::domain::add_ons::AddOn;
use crate::domain::coupons::Coupon;
use crate::domain::prices::{self, LegacyPricingData, extract_legacy_pricing};
use crate::domain::{
    CreateSubscription, CreateSubscriptionFromQuote, Customer, InvoicingEntityProviderSensitive,
    PlanForSubscription, Price, PriceComponent, Product,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{StoreResult, services::Services};
use common_domain::ids::{CouponId, CustomerId, PlanVersionId, PriceComponentId, PriceId, ProductId, TenantId};
use diesel_models::add_ons::AddOnRow;
use diesel_models::coupons::CouponRow;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plans::PlanRowForSubscription;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use error_stack::Report;
use itertools::Itertools;
use secrecy::SecretString;
use std::collections::HashMap;

/// A resolved custom component: fee computed read-only, source data preserved for deferred creation.
#[derive(Debug, Clone)]
pub struct ResolvedCustomComponent {
    pub name: String,
    pub fee: crate::domain::SubscriptionFee,
    pub period: crate::domain::enums::SubscriptionFeeBillingPeriod,
    pub price_component_id: Option<PriceComponentId>,
    /// Existing → already has an ID; New → needs creation inside the persist transaction.
    pub product_ref: crate::domain::price_components::ProductRef,
    /// Same as above for the price.
    pub price_entry: crate::domain::price_components::PriceEntry,
}

impl ResolvedCustomComponent {
    /// Product ID if the product already exists, None if it needs creation.
    pub fn existing_product_id(&self) -> Option<ProductId> {
        match &self.product_ref {
            crate::domain::price_components::ProductRef::Existing(id) => Some(*id),
            crate::domain::price_components::ProductRef::New { .. } => None,
        }
    }

    /// Price ID if the price already exists, None if it needs creation.
    pub fn existing_price_id(&self) -> Option<common_domain::ids::PriceId> {
        match &self.price_entry {
            crate::domain::price_components::PriceEntry::Existing(id) => Some(*id),
            crate::domain::price_components::PriceEntry::New(_) => None,
        }
    }

    /// True if this component requires Product or Price row creation in the transaction.
    pub fn needs_materialization(&self) -> bool {
        matches!(
            self.product_ref,
            crate::domain::price_components::ProductRef::New { .. }
        ) || matches!(
            self.price_entry,
            crate::domain::price_components::PriceEntry::New(_)
        )
    }
}

#[derive(Debug)]
pub struct ResolvedCustomComponents {
    pub overrides: HashMap<PriceComponentId, ResolvedCustomComponent>,
    pub extras: Vec<ResolvedCustomComponent>,
}

#[derive(Debug)]
pub struct SubscriptionCreationContext {
    pub customers: Vec<Customer>,
    pub plans: Vec<PlanForSubscription>,
    pub price_components_by_plan_version: HashMap<PlanVersionId, Vec<PriceComponent>>,
    pub products_by_id: HashMap<ProductId, Product>,
    pub addon_prices_by_id: HashMap<PriceId, Price>,
    pub all_add_ons: Vec<AddOn>,
    pub all_coupons: Vec<Coupon>,
    pub invoicing_entity_providers: Vec<InvoicingEntityProviderSensitive>,
    /// Pre-resolved extras and overrides, indexed by batch position.
    pub resolved_custom_components: Vec<ResolvedCustomComponents>,
}

impl SubscriptionCreationContext {
    pub(crate) fn get_invoicing_entity_providers_for_customer(
        &self,
        customer: &Customer,
    ) -> Option<&InvoicingEntityProviderSensitive> {
        self.invoicing_entity_providers
            .iter()
            .find(|e| e.id == customer.invoicing_entity_id)
    }
}

impl Services {
    pub(crate) async fn gather_subscription_context(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<SubscriptionCreationContext> {
        let plan_version_ids: Vec<_> = batch
            .iter()
            .map(|c| c.subscription.plan_version_id)
            .collect();

        let plans = self.get_plans(conn, &plan_version_ids).await?;

        // Load components with prices and legacy data pre-attached
        let price_components = self
            .load_price_components_with_prices(conn, &plan_version_ids, tenant_id)
            .await?;

        let add_ons = self.get_add_ons(conn, batch, &tenant_id).await?;

        // Load real products for v2 components and add-ons
        let product_ids: Vec<ProductId> = price_components
            .values()
            .flat_map(|comps| comps.iter().filter_map(|c| c.product_id))
            .chain(add_ons.iter().filter_map(|a| a.product_id))
            .unique()
            .collect();
        let products_by_id: HashMap<ProductId, Product> = if product_ids.is_empty() {
            HashMap::new()
        } else {
            ProductRow::list_by_ids(conn, &product_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Product::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?
        };

        // Load prices referenced by add-ons
        let addon_price_ids: Vec<PriceId> = add_ons.iter().filter_map(|a| a.price_id).unique().collect();
        let addon_prices_by_id = if addon_price_ids.is_empty() {
            HashMap::new()
        } else {
            PriceRow::list_by_ids(conn, &addon_price_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Price::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?
        };
        let coupons = self.get_coupons(conn, batch, &tenant_id).await?;
        let customers = self.get_customers(conn, batch, &tenant_id).await?;
        let invoicing_entities = self
            .list_invoicing_entities(conn, &tenant_id, secret_decoding_key)
            .await?;

        // Resolve extras and overrides for each subscription in batch (read-only fee computation)
        let resolved_custom_components = self
            .resolve_custom_components(
                conn,
                batch,
                &price_components,
                &products_by_id,
                tenant_id,
            )
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: price_components,
            products_by_id,
            addon_prices_by_id,
            all_add_ons: add_ons,
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
            resolved_custom_components,
        })
    }

    async fn list_invoicing_entities(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<Vec<InvoicingEntityProviderSensitive>> {
        InvoicingEntityProvidersRow::list_by_tenant_id(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| InvoicingEntityProviderSensitive::from_row(s, secret_decoding_key))
            .collect::<Result<Vec<_>, _>>()
    }
    async fn get_plans(
        &self,
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
    ) -> StoreResult<Vec<PlanForSubscription>> {
        PlanRowForSubscription::get_plans_for_subscription_by_version_ids(
            conn,
            plan_version_ids.to_vec(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|x| x.into_iter().map(Into::into).collect())
    }

    /// Load price components with v2 prices and legacy pricing data pre-attached.
    /// No fake IDs are created — v1 components carry `legacy_pricing` instead.
    async fn load_price_components_with_prices(
        &self,
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
        tenant_id: TenantId,
    ) -> StoreResult<HashMap<PlanVersionId, Vec<PriceComponent>>> {
        let rows_by_version =
            PriceComponentRow::get_by_plan_version_ids(conn, plan_version_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Collect all component IDs
        let all_component_ids: Vec<PriceComponentId> = rows_by_version
            .values()
            .flat_map(|rows| rows.iter().map(|r| r.id))
            .collect();

        // Load v2 prices via plan_component_price join
        let mut prices_by_component: HashMap<PriceComponentId, Vec<Price>> = HashMap::new();
        if !all_component_ids.is_empty() {
            let pcp_rows =
                PlanComponentPriceRow::list_by_component_ids(conn, &all_component_ids)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

            if !pcp_rows.is_empty() {
                let price_ids: Vec<PriceId> = pcp_rows.iter().map(|pcp| pcp.price_id).collect();
                let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let prices_by_id: HashMap<PriceId, Price> = price_rows
                    .into_iter()
                    .map(|row| {
                        let id = row.id;
                        Price::try_from(row).map(|p| (id, p))
                    })
                    .collect::<Result<HashMap<_, _>, _>>()?;

                for pcp in &pcp_rows {
                    if let Some(price) = prices_by_id.get(&pcp.price_id) {
                        prices_by_component
                            .entry(pcp.plan_component_id)
                            .or_default()
                            .push(price.clone());
                    }
                }
            }
        }

        // Extract legacy pricing data for v1 components (no fake IDs)
        let mut legacy_by_component: HashMap<PriceComponentId, LegacyPricingData> = HashMap::new();
        let has_v1 = rows_by_version
            .values()
            .flat_map(|rows| rows.iter())
            .any(|r| !prices_by_component.contains_key(&r.id) && r.legacy_fee.is_some());

        if has_v1 {
            let mut plan_versions: HashMap<
                PlanVersionId,
                diesel_models::plan_versions::PlanVersionRow,
            > = HashMap::new();
            for pv_id in plan_version_ids {
                let pv =
                    diesel_models::plan_versions::PlanVersionRow::find_by_id_and_tenant_id(
                        conn, *pv_id, tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                plan_versions.insert(*pv_id, pv);
            }

            for (pv_id, rows) in &rows_by_version {
                for row in rows {
                    if !prices_by_component.contains_key(&row.id) {
                        if let Some(pv) = plan_versions.get(pv_id) {
                            if let Some(legacy_json) = &row.legacy_fee {
                                let legacy =
                                    extract_legacy_pricing(legacy_json, pv.currency.clone())?;
                                legacy_by_component.insert(row.id, legacy);
                            }
                        }
                    }
                }
            }
        }

        // Convert to domain with prices and legacy data attached
        let mut result: HashMap<PlanVersionId, Vec<PriceComponent>> = HashMap::new();
        for (pv_id, rows) in rows_by_version {
            let mut components = Vec::new();
            for row in rows {
                let id = row.id;
                let mut comp: PriceComponent = row.try_into()?;
                if let Some(prices) = prices_by_component.remove(&id) {
                    comp.prices = prices;
                }
                if let Some(legacy) = legacy_by_component.remove(&id) {
                    comp.legacy_pricing = Some(legacy);
                }
                components.push(comp);
            }
            result.insert(pv_id, components);
        }

        Ok(result)
    }

    async fn get_add_ons(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<AddOn>> {
        let add_on_ids: Vec<_> = batch
            .iter()
            .filter_map(|x| x.add_ons.as_ref())
            .flat_map(|x| &x.add_ons)
            .map(|x| x.add_on_id)
            .unique()
            .collect();

        AddOnRow::list_by_ids(conn, &add_on_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn get_coupons(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Coupon>> {
        let coupon_ids: Vec<_> = batch
            .iter()
            .filter_map(|x| x.coupons.as_ref())
            .flat_map(|x| &x.coupons)
            .map(|x| x.coupon_id)
            .unique()
            .collect();

        CouponRow::list_by_ids(conn, &coupon_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())
    }

    async fn get_customers(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let customer_ids: Vec<_> = batch
            .iter()
            .map(|c| c.subscription.customer_id)
            .unique()
            .collect();

        if let Some((id, name)) =
            CustomerRow::find_archived_customer_in_batch(conn, *tenant_id, customer_ids.clone())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create subscription for archived customer: {} ({})",
                name, id
            ))
            .into());
        }

        CustomerRow::list_by_ids(conn, tenant_id, customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Vec<StoreResult<Customer>>>()
            .into_iter()
            .collect::<StoreResult<Vec<Customer>>>()
    }

    pub(crate) async fn gather_subscription_context_from_quote(
        &self,
        conn: &mut PgConn,
        params: &CreateSubscriptionFromQuote,
        tenant_id: TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<SubscriptionCreationContext> {
        let plan_version_ids = vec![params.subscription.plan_version_id];

        let plans = self.get_plans(conn, &plan_version_ids).await?;

        // Get coupons by IDs
        let coupons = self
            .get_coupons_by_ids(conn, &params.coupon_ids, &tenant_id)
            .await?;

        // Get customer
        let customers = self
            .get_customers_by_ids(conn, &[params.subscription.customer_id], &tenant_id)
            .await?;

        let invoicing_entities = self
            .list_invoicing_entities(conn, &tenant_id, secret_decoding_key)
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: HashMap::new(), // Not needed for quote conversion
            products_by_id: HashMap::new(),                   // Not needed for quote conversion
            addon_prices_by_id: HashMap::new(),               // Not needed for quote conversion
            all_add_ons: vec![],                              // Not needed for quote conversion
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
            resolved_custom_components: vec![ResolvedCustomComponents {
                overrides: HashMap::new(),
                extras: vec![],
            }],
        })
    }

    /// Resolve fees for overrides and extras
    /// FeeStructure comes from existing Product or ProductRef::New payload.
    /// Pricing comes from existing Price or PriceEntry::New payload.
    async fn resolve_custom_components(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        price_components: &HashMap<PlanVersionId, Vec<PriceComponent>>,
        products_by_id: &HashMap<ProductId, Product>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ResolvedCustomComponents>> {
        use crate::domain::price_components::{PriceEntry, ProductRef};

        let mut results = Vec::with_capacity(batch.len());

        for params in batch {
            let sub = &params.subscription;

            let mut overrides = HashMap::new();
            let mut extras = Vec::new();

            if let Some(components) = &params.price_components {
                let binding = vec![];
                let plan_comps = price_components
                    .get(&sub.plan_version_id)
                    .unwrap_or(&binding);

                // Resolve overrides (product is always existing — from the plan component)
                for ov in &components.overridden_components {
                    let plan_comp =
                        plan_comps
                            .iter()
                            .find(|c| c.id == ov.component_id)
                            .ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(format!(
                                    "Override component {} not found in plan",
                                    ov.component_id
                                )))
                            })?;

                    let product_id = plan_comp.product_id.ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(format!(
                            "Cannot override component {} — it has no product_id",
                            plan_comp.id
                        )))
                    })?;

                    let product = products_by_id.get(&product_id).ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(format!(
                            "Product {} not found for override component {}",
                            product_id, plan_comp.id
                        )))
                    })?;

                    let (fee, period) = resolve_fee_read_only(
                        conn,
                        &product.fee_structure,
                        &ov.price_entry,
                        tenant_id,
                    )
                    .await?;

                    overrides.insert(
                        ov.component_id,
                        ResolvedCustomComponent {
                            name: ov.name.clone(),
                            fee,
                            period,
                            price_component_id: Some(plan_comp.id),
                            product_ref: ProductRef::Existing(product_id),
                            price_entry: ov.price_entry.clone(),
                        },
                    );
                }

                // Resolve extras
                for extra in &components.extra_components {
                    let fee_structure = match &extra.product_ref {
                        ProductRef::Existing(pid) => {
                            // Product may not be in products_by_id (e.g. from product library)
                            if let Some(p) = products_by_id.get(pid) {
                                p.fee_structure.clone()
                            } else {
                                let row =
                                    ProductRow::find_by_id_and_tenant_id(conn, *pid, tenant_id)
                                        .await
                                        .map_err(Into::<Report<StoreError>>::into)?;
                                let product = Product::try_from(row)?;
                                product.fee_structure
                            }
                        }
                        ProductRef::New { fee_structure, .. } => fee_structure.clone(),
                    };

                    // Validate: New product + Existing price is an error
                    if matches!(extra.product_ref, ProductRef::New { .. })
                        && matches!(extra.price_entry, PriceEntry::Existing(_))
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Cannot use existing price with a new product".to_string(),
                        )));
                    }

                    let (fee, period) = resolve_fee_read_only(
                        conn,
                        &fee_structure,
                        &extra.price_entry,
                        tenant_id,
                    )
                    .await?;

                    extras.push(ResolvedCustomComponent {
                        name: extra.name.clone(),
                        fee,
                        period,
                        price_component_id: None,
                        product_ref: extra.product_ref.clone(),
                        price_entry: extra.price_entry.clone(),
                    });
                }
            }

            results.push(ResolvedCustomComponents { overrides, extras });
        }

        Ok(results)
    }

    async fn get_coupons_by_ids(
        &self,
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Coupon>> {
        if coupon_ids.is_empty() {
            return Ok(vec![]);
        }

        CouponRow::list_by_ids(conn, coupon_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())
    }

    async fn get_customers_by_ids(
        &self,
        conn: &mut PgConn,
        customer_ids: &[CustomerId],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Customer>> {
        if let Some((id, name)) =
            CustomerRow::find_archived_customer_in_batch(conn, *tenant_id, customer_ids.to_vec())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create subscription for archived customer: {} ({})",
                name, id
            ))
            .into());
        }

        CustomerRow::list_by_ids(conn, tenant_id, customer_ids.to_vec())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Vec<StoreResult<Customer>>>()
            .into_iter()
            .collect::<StoreResult<Vec<Customer>>>()
    }
}

/// Compute (SubscriptionFee, period) from FeeStructure + PriceEntry
/// For PriceEntry::Existing, loads the Price (read-only SELECT).
/// For PriceEntry::New, extracts Pricing directly from the payload.
/// No rows are created.
pub(crate) async fn resolve_fee_read_only(
    conn: &mut PgConn,
    fee_structure: &crate::domain::prices::FeeStructure,
    price_entry: &crate::domain::price_components::PriceEntry,
    tenant_id: TenantId,
) -> StoreResult<(
    crate::domain::SubscriptionFee,
    crate::domain::enums::SubscriptionFeeBillingPeriod,
)> {
    match price_entry {
        crate::domain::price_components::PriceEntry::Existing(price_id) => {
            let price_row = PriceRow::find_by_id_and_tenant_id(conn, *price_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let price = Price::try_from(price_row)?;
            let fee = prices::resolve_subscription_fee(fee_structure, &price.pricing, None)?;
            let period = prices::fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
        crate::domain::price_components::PriceEntry::New(price_input) => {
            let fee = prices::resolve_subscription_fee(fee_structure, &price_input.pricing, None)?;
            let period = prices::fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price_input.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
    }
}

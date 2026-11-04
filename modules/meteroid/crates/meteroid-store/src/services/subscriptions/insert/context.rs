use crate::domain::add_ons::AddOn;
use crate::domain::coupons::Coupon;
use crate::domain::credit_units::CreditUnitsById;
use crate::domain::prices::{self, LegacyPricingData, extract_legacy_pricing};
use crate::domain::{
    CreateSubscription, CreateSubscriptionFromQuote, Customer, InvoicingEntityProviderSensitive,
    PlanForSubscription, Price, PriceComponent, Product,
};
use crate::errors::StoreError;
use crate::repositories::credit_units::load_credit_units;
use crate::services::invoice_lines::invoice_lines::build_tax_engine;
use crate::services::invoice_lines::tax_inputs::customer_for_tax;
use crate::services::subscriptions::utils::{
    blended_inclusive_rate, calculate_mrr, calculate_subscription_mrr_with_floor,
    creation_minimum_for_mrr, extract_billing_period, fee_period_total_cents, fee_tax_product,
    fee_taxed_at_redemption, process_create_subscription_add_ons,
    process_create_subscription_components,
};
use crate::store::PgConn;
use crate::{StoreResult, services::Services};
use common_domain::ids::{
    CouponId, CreditUnitId, CustomerId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    TenantId,
};
use diesel_models::add_ons::AddOnRow;
use diesel_models::coupons::CouponRow;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plans::PlanRowForSubscription;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use error_stack::{Report, ResultExt};
use itertools::Itertools;
use rust_decimal::Decimal;
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

/// A subscription about to be created on a tax-inclusive plan: what the tax engine needs
/// to say how much tax its quoted prices carry.
struct QuotedSubscription<'a> {
    customer: &'a Customer,
    currency: &'a str,
    /// One entry per MRR-bearing component/add-on and minimum shortfall. See [`QuotedLine`].
    lines: Vec<QuotedLine>,
    /// The date the tax is resolved on: when the subscription starts billing.
    date: chrono::NaiveDate,
}

/// One MRR-bearing component or add-on, in the two figures the rate resolution needs.
///
/// They differ whenever a component is billed over more than a month, and each is used for
/// exactly one thing:
/// - `period_total_cents` is what the engine is asked about — the amount that component's
///   invoice line will carry, so thresholds and classification land where the invoice will
///   put them;
/// - `monthly_share_cents` is what the resolved rate is weighted by — the share this
///   component contributes to the subscription's MRR. MRR is a sum of monthly shares, so a
///   blend weighted by anything else would not net that sum to the sum of the lines' net.
pub(in crate::services) struct QuotedLine {
    pub product_id: Option<ProductId>,
    pub period_total_cents: i64,
    pub monthly_share_cents: i64,
    /// False for a currency credit pack: no tax is due on it until the credit is used.
    pub taxed: bool,
}

#[derive(Debug)]
pub struct SubscriptionCreationContext {
    pub customers: Vec<Customer>,
    pub plans: Vec<PlanForSubscription>,
    pub price_components_by_plan_version: HashMap<PlanVersionId, Vec<PriceComponent>>,
    pub products_by_id: HashMap<ProductId, Product>,
    pub addon_prices_by_id: HashMap<PriceId, Price>,
    /// The credit units of every price above and of the requests' new prices.
    pub credit_units: CreditUnitsById,
    pub all_add_ons: Vec<AddOn>,
    pub all_coupons: Vec<Coupon>,
    pub invoicing_entity_providers: Vec<InvoicingEntityProviderSensitive>,
    /// Pre-resolved extras and overrides, indexed by batch position.
    pub resolved_custom_components: Vec<ResolvedCustomComponents>,
    pub minimums_by_plan_version: HashMap<PlanVersionId, crate::domain::plan_minimums::PlanMinimum>,
    /// The tax the quoted prices carry, by batch position. See [`Services::resolve_tax_inclusive_rates`].
    pub tax_inclusive_rates: Vec<Option<Decimal>>,
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
            .chain(add_ons.iter().map(|a| a.product_id))
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
        let addon_price_ids: Vec<PriceId> = add_ons.iter().map(|a| a.price_id).unique().collect();
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
        let credit_units = load_credit_units(
            conn,
            tenant_id,
            price_components
                .values()
                .flatten()
                .flat_map(PriceComponent::credit_unit_ids)
                .chain(addon_prices_by_id.values().flat_map(Price::credit_unit_ids))
                .chain(batch.iter().flat_map(request_credit_unit_ids))
                .collect::<Vec<_>>(),
        )
        .await?;
        let coupons = self.get_coupons(conn, batch, &tenant_id).await?;
        let customers = self.get_customers(conn, batch, &tenant_id).await?;
        let invoicing_entities = self
            .list_invoicing_entities(conn, &tenant_id, secret_decoding_key)
            .await?;

        // Resolve extras and overrides for each subscription in batch (read-only fee computation)
        let resolved_custom_components = self
            .resolve_custom_components(conn, batch, &price_components, &products_by_id, tenant_id)
            .await?;

        let minimums_by_plan_version =
            load_plan_minimums_by_version(conn, &plan_version_ids).await?;

        // Resolved here rather than in the (synchronous) processing step: it needs the
        // invoicing entity and the tax engine, and this is the last async point before the
        // rows are built.
        let quoted = batch
            .iter()
            .enumerate()
            .map(|(idx, params)| {
                quoted_subscription(
                    params,
                    idx,
                    &customers,
                    &plans,
                    &price_components,
                    &products_by_id,
                    &addon_prices_by_id,
                    &credit_units,
                    &add_ons,
                    &resolved_custom_components,
                    &minimums_by_plan_version,
                )
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let tax_inclusive_rates = self
            .resolve_tax_inclusive_rates(conn, tenant_id, &quoted)
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: price_components,
            products_by_id,
            addon_prices_by_id,
            credit_units,
            all_add_ons: add_ons,
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
            resolved_custom_components,
            minimums_by_plan_version,
            tax_inclusive_rates,
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
            let pcp_rows = PlanComponentPriceRow::list_by_component_ids(conn, &all_component_ids)
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
            let plan_version_list =
                diesel_models::plan_versions::PlanVersionRow::list_by_ids_and_tenant_id(
                    conn,
                    plan_version_ids,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let plan_versions: HashMap<
                PlanVersionId,
                diesel_models::plan_versions::PlanVersionRow,
            > = plan_version_list
                .into_iter()
                .map(|pv| (pv.id, pv))
                .collect();

            for (pv_id, rows) in &rows_by_version {
                for row in rows {
                    if !prices_by_component.contains_key(&row.id)
                        && let Some(pv) = plan_versions.get(pv_id)
                        && let Some(legacy_json) = &row.legacy_fee
                    {
                        let legacy = extract_legacy_pricing(legacy_json, pv.currency.clone())?;
                        legacy_by_component.insert(row.id, legacy);
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

    /// The tax rate each subscription's quoted prices carry, by batch position.
    ///
    /// Only a tax-inclusive plan has anything to resolve: its prices already contain the
    /// tax, so the MRR read off its fees would contain it too. The resolution is
    /// deliberately the one an invoice runs — the same engine, the same `CustomerForTax`,
    /// the same product/category tax configuration, one line per component or add-on at
    /// the amount that component's invoice line will carry — so the rate the MRR is netted
    /// by is the rate the customer is actually charged at. Going through the line items
    /// rather than `calculate_customer_tax` is what makes that true: the customer-level
    /// call only answers the last rung of the tax ladder, and for a `Manual` invoicing
    /// entity — where the tax lives entirely in the product configuration — it answers
    /// "no tax" for every customer.
    ///
    /// It is one rate for the whole subscription: lines that resolve to different rates are
    /// blended by the monthly share each contributes to the MRR. It is re-resolved whenever
    /// the fees change (`restate_subscription_mrr`); a customer who later moves country or
    /// registers a VAT number keeps the rate until then.
    ///
    /// A customer with no billing address, or an entity with no tax engine, resolves to
    /// `None` — the same "no tax to apply" the invoice side reaches.
    async fn resolve_tax_inclusive_rates(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        quoted: &[Option<QuotedSubscription<'_>>],
    ) -> StoreResult<Vec<Option<Decimal>>> {
        let mut rates = vec![None; quoted.len()];

        if quoted.iter().all(Option::is_none) {
            return Ok(rates);
        }

        // Scoped to the tenant rather than looked up by the ids the customers carry: a
        // tenant has a handful of invoicing entities, and an id-only lookup would be an
        // untenanted read waiting for a caller that does not vet its ids.
        let entities: HashMap<_, crate::domain::InvoicingEntity> =
            InvoicingEntityRow::list_by_tenant_id(conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| (row.id, row.into()))
                .collect();

        for (rate, quoted) in rates.iter_mut().zip(quoted) {
            let Some(quoted) = quoted else { continue };
            let Some(entity) = entities.get(&quoted.customer.invoicing_entity_id) else {
                continue;
            };
            *rate = self
                .resolve_tax_inclusive_rate(
                    conn,
                    entity,
                    quoted.customer,
                    quoted.currency,
                    &quoted.lines,
                    quoted.date,
                )
                .await?;
        }

        Ok(rates)
    }

    /// The rate a tax-inclusive subscription's quoted prices carry, from the lines it has now:
    /// the blend of what the engine resolves each taxed line to, weighted by monthly share.
    /// `None` when nothing is taxed or there is no engine or billing address to ask.
    pub(in crate::services) async fn resolve_tax_inclusive_rate(
        &self,
        conn: &mut PgConn,
        entity: &crate::domain::InvoicingEntity,
        customer: &Customer,
        currency: &str,
        quoted_lines: &[QuotedLine],
        date: chrono::NaiveDate,
    ) -> StoreResult<Option<Decimal>> {
        let Some(billing_address) = customer.billing_address.clone() else {
            return Ok(None);
        };
        let Some(engine) = build_tax_engine(entity)? else {
            return Ok(None);
        };

        let tax_config = self
            .load_product_tax_config_with_conn(
                conn,
                entity,
                &quoted_lines
                    .iter()
                    .map(|line| line.product_id)
                    .collect::<Vec<_>>(),
            )
            .await?;

        // Keyed by position so the engine's answers can be paired back with the
        // monthly share they have to be weighted by.
        let lines: Vec<meteroid_tax::LineItemForTax> = quoted_lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.period_total_cents != 0 && line.taxed)
            .map(|(idx, line)| {
                let (custom_taxes, tax_category) = tax_config.for_product(line.product_id);
                meteroid_tax::LineItemForTax {
                    line_id: idx.to_string(),
                    amount: line.period_total_cents,
                    custom_taxes,
                    tax_category,
                    tax_inclusive: true,
                }
            })
            .collect();

        if lines.is_empty() {
            return Ok(None);
        }

        // The engine is asked about the period total, but the blend is weighted by the
        // monthly share: the MRR the resolved rate is applied to is a sum of monthly
        // shares, and a blend weighted by period totals would only agree with it when
        // every line shares one billing period.
        let weights: HashMap<String, i64> = quoted_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (idx.to_string(), line.monthly_share_cents))
            .collect();

        let computed = engine
            .calculate_line_items_tax(
                currency.to_string(),
                customer_for_tax(customer, entity, billing_address),
                entity.address().into(),
                lines,
                date,
            )
            .await
            .change_context(StoreError::TaxError)?;

        // The rate describes taxed lines only: MRR adds untaxed currency packs at face.
        let resolved: Vec<(Decimal, Decimal)> = computed
            .line_items
            .iter()
            .filter_map(|line| {
                let monthly_share = weights.get(&line.line_id)?;
                Some((Decimal::from(*monthly_share), line.effective_rate))
            })
            .collect();

        Ok(blended_inclusive_rate(&resolved))
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

        let minimums_by_plan_version =
            load_plan_minimums_by_version(conn, &plan_version_ids).await?;

        // A quote arrives with its components and add-ons already resolved, so the quoted
        // lines are read straight off them. See `resolve_tax_inclusive_rates`.
        let quoted = plans
            .iter()
            .find(|p| p.version_id == params.subscription.plan_version_id)
            .filter(|plan| plan.tax_inclusive)
            .and_then(|plan| {
                let customer = customers
                    .iter()
                    .find(|c| c.id == params.subscription.customer_id)?;
                let sub = &params.subscription;
                let minimum = creation_minimum_for_mrr(
                    &params.components,
                    minimums_by_plan_version.get(&sub.plan_version_id),
                    params.minimum_override.as_ref(),
                    false,
                );
                Some(QuotedSubscription {
                    customer,
                    currency: &plan.currency,
                    lines: quoted_lines(
                        &params.components,
                        &params.add_ons,
                        &plan.currency,
                        minimum.as_ref(),
                    ),
                    date: sub.billing_start_date.unwrap_or(sub.start_date),
                })
            });
        let tax_inclusive_rates = self
            .resolve_tax_inclusive_rates(conn, tenant_id, std::slice::from_ref(&quoted))
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: HashMap::new(), // Not needed for quote conversion
            products_by_id: HashMap::new(),                   // Not needed for quote conversion
            addon_prices_by_id: HashMap::new(),               // Not needed for quote conversion
            credit_units: CreditUnitsById::new(),             // Not needed for quote conversion
            all_add_ons: vec![],                              // Not needed for quote conversion
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
            resolved_custom_components: vec![ResolvedCustomComponents {
                overrides: HashMap::new(),
                extras: vec![],
            }],
            minimums_by_plan_version,
            tax_inclusive_rates,
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
                    let plan_comp = plan_comps
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

                    let (fee, period) =
                        resolve_fee_read_only(conn, &fee_structure, &extra.price_entry, tenant_id)
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

/// The credit units of a creation request's new prices (overrides, extras, add-on overrides).
fn request_credit_unit_ids(params: &CreateSubscription) -> Vec<CreditUnitId> {
    let components = params.price_components.iter().flat_map(|components| {
        components
            .overridden_components
            .iter()
            .map(|ov| &ov.price_entry)
            .chain(
                components
                    .extra_components
                    .iter()
                    .map(|extra| &extra.price_entry),
            )
    });
    let add_ons = params.add_ons.iter().flat_map(|add_ons| {
        add_ons
            .add_ons
            .iter()
            .flat_map(|add_on| add_on.customization.new_credit_unit_ids())
    });
    components
        .flat_map(|entry| entry.new_credit_unit_ids())
        .chain(add_ons)
        .collect()
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
            let units = load_credit_units(conn, tenant_id, price.credit_unit_ids()).await?;
            let fee = prices::resolve_subscription_fee(
                fee_structure,
                &price.pricing,
                price.credit_unit.as_ref(),
                &units,
                None,
            )?;
            let period = prices::fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
        crate::domain::price_components::PriceEntry::New(price_input) => {
            let units =
                load_credit_units(conn, tenant_id, price_entry.new_credit_unit_ids()).await?;
            let fee = prices::resolve_subscription_fee(
                fee_structure,
                &price_input.pricing,
                price_input.credit_unit.as_ref(),
                &units,
                None,
            )?;
            let period = prices::fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price_input.cadence.as_subscription_billing_period());
            Ok((fee, period))
        }
    }
}

async fn load_plan_minimums_by_version(
    conn: &mut PgConn,
    plan_version_ids: &[PlanVersionId],
) -> StoreResult<HashMap<PlanVersionId, crate::domain::plan_minimums::PlanMinimum>> {
    use diesel_models::plan_version_adjustments::{
        PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentRow,
    };
    use itertools::Itertools;

    if plan_version_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let unique_ids: Vec<PlanVersionId> = plan_version_ids.iter().copied().unique().collect();

    let rows = PlanVersionAdjustmentRow::list_by_plan_version_ids(conn, &unique_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let scoped_ids: Vec<PlanVersionId> = rows
        .iter()
        .filter(|r| !r.includes_all_components)
        .map(|r| r.plan_version_id)
        .collect();

    let component_rows = if scoped_ids.is_empty() {
        vec![]
    } else {
        PlanVersionAdjustmentComponentRow::list_by_plan_version_ids(conn, &scoped_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    };

    let components_by_pv: HashMap<PlanVersionId, Vec<PlanVersionAdjustmentComponentRow>> =
        component_rows
            .into_iter()
            .into_group_map_by(|c| c.plan_version_id);

    let mut out = HashMap::new();
    for row in rows {
        let comps = components_by_pv
            .get(&row.plan_version_id)
            .cloned()
            .unwrap_or_default();
        let pv_id = row.plan_version_id;
        out.insert(
            pv_id,
            crate::domain::plan_minimums::PlanMinimum::from_rows(row, comps),
        );
    }
    Ok(out)
}

/// The tax-engine input for one batch position, or `None` when its plan is tax-exclusive and
/// there is nothing to resolve.
///
/// The components and add-ons are resolved the same way the processing step will resolve them
/// a moment later — the quoted lines have to be the ones the subscription is actually born
/// with — and thrown away again: this runs before that step because only here can it await
/// the tax engine.
#[allow(clippy::too_many_arguments)]
fn quoted_subscription<'a>(
    params: &CreateSubscription,
    idx: usize,
    customers: &'a [Customer],
    plans: &'a [PlanForSubscription],
    price_components: &HashMap<PlanVersionId, Vec<PriceComponent>>,
    products_by_id: &HashMap<ProductId, Product>,
    addon_prices_by_id: &HashMap<PriceId, Price>,
    credit_units: &CreditUnitsById,
    add_ons: &[AddOn],
    resolved_custom_components: &[ResolvedCustomComponents],
    minimums_by_plan_version: &HashMap<PlanVersionId, crate::domain::plan_minimums::PlanMinimum>,
) -> StoreResult<Option<QuotedSubscription<'a>>> {
    let sub = &params.subscription;

    let plan = plans
        .iter()
        .find(|p| p.version_id == sub.plan_version_id)
        .ok_or_else(|| Report::new(StoreError::ValueNotFound("Plan id not found".to_string())))?;

    if !plan.tax_inclusive {
        return Ok(None);
    }

    let customer = customers
        .iter()
        .find(|c| c.id == sub.customer_id)
        .ok_or_else(|| Report::new(StoreError::InsertError).attach("Customer not found"))?;

    let resolved = resolved_custom_components
        .get(idx)
        .ok_or_else(|| Report::new(StoreError::InsertError))
        .attach("Missing resolved custom components for batch index")?;

    let effective_from = sub.billing_start_date.unwrap_or(sub.start_date);

    let (components, _) = process_create_subscription_components(
        &params.price_components,
        price_components,
        sub,
        products_by_id,
        credit_units,
        resolved,
        plan.product_family_id,
        &plan.currency,
        effective_from,
    )?;
    let (add_ons, _) = process_create_subscription_add_ons(
        &params.add_ons,
        add_ons,
        products_by_id,
        addon_prices_by_id,
        credit_units,
        plan.product_family_id,
        &plan.currency,
        effective_from,
    )?;

    let minimum = creation_minimum_for_mrr(
        &components,
        minimums_by_plan_version.get(&sub.plan_version_id),
        params.minimum_override.as_ref(),
        params.drop_minimum,
    );

    Ok(Some(QuotedSubscription {
        customer,
        currency: &plan.currency,
        lines: quoted_lines(&components, &add_ons, &plan.currency, minimum.as_ref()),
        date: effective_from,
    }))
}

/// One tax-engine line per MRR-bearing component and add-on: the product it is sold under,
/// its quoted total for its own billing period — what its invoice line will carry — and the
/// monthly share of that total, which is what it contributes to MRR. A minimum shortfall is
/// split like the synthesized invoice lines (`minimum::split_shortfall`): no product for an
/// all-components minimum (the entity's default tax), the scoped products' otherwise.
fn quoted_lines(
    components: &[crate::domain::SubscriptionComponentNewInternal],
    add_ons: &[crate::domain::SubscriptionAddOnNewInternal],
    currency: &str,
    minimum: Option<&(i64, crate::domain::plan_minimums::MinimumScope)>,
) -> Vec<QuotedLine> {
    let precision = crate::constants::Currencies::resolve_currency_precision(currency).unwrap_or(2);

    let mut lines: Vec<QuotedLine> = components
        .iter()
        .map(|c| QuotedLine {
            product_id: fee_tax_product(&c.fee, c.product_id),
            period_total_cents: fee_period_total_cents(&c.fee, precision),
            monthly_share_cents: calculate_mrr(&c.fee, &c.period, precision),
            taxed: !fee_taxed_at_redemption(&c.fee),
        })
        .chain(add_ons.iter().map(|a| {
            let quantity = i64::from(a.quantity);
            QuotedLine {
                product_id: fee_tax_product(&a.fee, a.product_id),
                period_total_cents: fee_period_total_cents(&a.fee, precision) * quantity,
                monthly_share_cents: calculate_mrr(&a.fee, &a.period, precision) * quantity,
                taxed: !fee_taxed_at_redemption(&a.fee),
            }
        }))
        .filter(|line| line.period_total_cents != 0)
        .collect();

    if minimum.is_some() {
        let period = extract_billing_period(components, add_ons);
        let base_mrr: i64 = lines.iter().map(|line| line.monthly_share_cents).sum();
        let floored_mrr = calculate_subscription_mrr_with_floor(
            components, add_ons, minimum, period, precision, None,
        );
        let shortfall = floored_mrr - base_mrr;
        if shortfall > 0
            && let Some((_, scope)) = minimum
        {
            let weights: Vec<_> = lines
                .iter()
                .filter_map(|line| {
                    line.product_id
                        .map(|product| (product, line.monthly_share_cents))
                })
                .collect();
            let carried: Vec<_> = lines.iter().filter_map(|line| line.product_id).collect();
            for (product_id, share) in crate::services::invoice_lines::minimum::split_shortfall(
                scope, &weights, &carried, shortfall,
            ) {
                lines.push(QuotedLine {
                    product_id,
                    period_total_cents: share * i64::from(period.as_months()).max(1),
                    monthly_share_cents: share,
                    taxed: true,
                });
            }
        }
    }

    lines
}

#[cfg(test)]
mod quoted_lines_tests {
    use super::quoted_lines;
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    use crate::domain::plan_minimums::MinimumScope;
    use crate::domain::{
        SubscriptionAddOnNewInternal, SubscriptionComponentNewInternal, SubscriptionFee,
    };
    use common_domain::ids::{AddOnId, BaseId, ProductId};
    use rust_decimal_macros::dec;

    fn component(
        rate: rust_decimal::Decimal,
        period: SubscriptionFeeBillingPeriod,
    ) -> SubscriptionComponentNewInternal {
        SubscriptionComponentNewInternal {
            price_component_id: None,
            product_id: None,
            name: "component".to_string(),
            period,
            fee: SubscriptionFee::Rate { rate },
            is_override: false,
            price_id: None,
            effective_from: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        }
    }

    fn add_on(
        rate: rust_decimal::Decimal,
        period: SubscriptionFeeBillingPeriod,
        quantity: i32,
    ) -> SubscriptionAddOnNewInternal {
        SubscriptionAddOnNewInternal {
            add_on_id: AddOnId::new(),
            name: "add-on".to_string(),
            period,
            fee: SubscriptionFee::Rate { rate },
            product_id: None,
            price_id: None,
            quantity,
            effective_from: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        }
    }

    /// The engine is asked about the billing-period total — that is what the invoice line
    /// will carry — while the rate it answers is weighted by the monthly share. On a
    /// subscription that mixes periods the two figures diverge, and confusing them is what
    /// lets an annual line's rate dominate a blend the MRR is not weighted by.
    #[test]
    fn a_line_carries_its_period_total_and_its_monthly_share_separately() {
        let lines = quoted_lines(
            &[
                component(dec!(1200), SubscriptionFeeBillingPeriod::Annual),
                component(dec!(100), SubscriptionFeeBillingPeriod::Monthly),
            ],
            &[],
            "EUR",
            None,
        );

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].period_total_cents, 120_000);
        assert_eq!(lines[0].monthly_share_cents, 10_000);
        assert_eq!(lines[1].period_total_cents, 10_000);
        assert_eq!(lines[1].monthly_share_cents, 10_000);
    }

    /// An add-on's fee is stored per instance; both figures scale by the instance count, so
    /// the engine sees the line the invoice will issue and the blend sees the MRR the
    /// add-on contributes.
    #[test]
    fn an_add_on_scales_both_figures_by_its_quantity() {
        let lines = quoted_lines(
            &[],
            &[add_on(
                dec!(300),
                SubscriptionFeeBillingPeriod::Quarterly,
                3,
            )],
            "EUR",
            None,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].period_total_cents, 90_000);
        assert_eq!(lines[0].monthly_share_cents, 30_000);
    }

    #[test]
    fn a_currency_credit_pack_counts_in_mrr_but_is_not_quoted_for_tax() {
        let mut scale = component(dec!(500), SubscriptionFeeBillingPeriod::Monthly);
        scale.fee = SubscriptionFee::Credit {
            rate: dec!(500),
            quantity: 1,
            credit_amount: dec!(550),
            expiry: None,
            credit_unit: None,
            rollover: None,
            once: false,
        };
        let lines = quoted_lines(
            &[
                scale,
                component(dec!(20), SubscriptionFeeBillingPeriod::Monthly),
            ],
            &[],
            "USD",
            None,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].monthly_share_cents, 50_000);
        assert!(!lines[0].taxed);
        assert!(lines[1].taxed);
    }

    #[test]
    fn a_minimum_without_recurring_fees_still_has_a_tax_input() {
        let lines = quoted_lines(
            &[],
            &[],
            "EUR",
            Some(&(12_000, MinimumScope::AllComponents)),
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].product_id, None);
        assert_eq!(lines[0].period_total_cents, 12_000);
        assert_eq!(lines[0].monthly_share_cents, 12_000);
    }

    #[test]
    fn a_scoped_shortfall_counts_add_on_quantity_and_monthly_shares() {
        let product_id = ProductId::new();
        let mut addon = add_on(dec!(30), SubscriptionFeeBillingPeriod::Quarterly, 2);
        addon.product_id = Some(product_id);
        let lines = quoted_lines(
            &[component(dec!(60), SubscriptionFeeBillingPeriod::Monthly)],
            &[addon],
            "EUR",
            Some(&(5_000, MinimumScope::Products(vec![product_id]))),
        );
        // The out-of-scope fee contributes 60/month; the two add-ons contribute 20/month.
        // Only the add-ons count toward the 50/month floor, leaving a 30/month shortfall.
        assert_eq!(lines.len(), 3);
        // Taxed with its scoped product's settings, like the invoice line (OQ17).
        assert_eq!(lines[2].product_id, Some(product_id));
        assert_eq!(lines[2].monthly_share_cents, 3_000);
        assert_eq!(lines[2].period_total_cents, 3_000);
        assert_eq!(
            lines.iter().map(|l| l.monthly_share_cents).sum::<i64>(),
            11_000
        );
    }

    #[test]
    fn an_annual_shortfall_has_a_monthly_weight_and_period_total() {
        let lines = quoted_lines(
            &[component(dec!(600), SubscriptionFeeBillingPeriod::Annual)],
            &[],
            "EUR",
            Some(&(120_000, MinimumScope::AllComponents)),
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].period_total_cents, 60_000);
        assert_eq!(lines[1].monthly_share_cents, 5_000);
    }

    #[test]
    fn spend_above_the_minimum_has_no_shortfall_tax_input() {
        let lines = quoted_lines(
            &[component(dec!(150), SubscriptionFeeBillingPeriod::Monthly)],
            &[],
            "EUR",
            Some(&(12_000, MinimumScope::AllComponents)),
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].monthly_share_cents, 15_000);
    }
}

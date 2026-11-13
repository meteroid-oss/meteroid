use crate::StoreResult;
use crate::domain::Product;
use crate::domain::enums::InvoiceType;
use crate::domain::plan_minimums::{CreateMinimumScope, MinimumScope};
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::domain::subscription_coupons::AppliedCoupon;
use crate::domain::{
    AppliedCouponDetailed, BillingPeriodEnum, CheckoutSession, CheckoutType, CreateSubscription,
    CreateSubscriptionAddOns, CreateSubscriptionComponents, InvoicingEntity, PriceComponent,
    Subscription, SubscriptionActivationCondition, SubscriptionComponent, SubscriptionDetails,
    SubscriptionMinimum, SubscriptionStatusEnum, TrialConfig,
};
use crate::errors::StoreError;
use crate::repositories::add_ons::AddOnInterface;
use crate::repositories::customers::CustomersInterface;
use crate::repositories::plans::PlansInterface;
use crate::repositories::price_components::PriceComponentInterface;
use crate::services::Services;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::Datelike;
use chrono::Duration;
use common_domain::ids::{
    AddOnId, AppliedCouponId, BaseId, ProductId, SubscriptionAddOnId, SubscriptionId,
    SubscriptionPriceComponentId, TenantId,
};
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::products::ProductRow;
use error_stack::Report;
use std::collections::HashMap;

impl Services {
    /// Builds a virtual SubscriptionDetails from a checkout session for invoice preview.
    /// This is used in the self-serve checkout flow where there's no subscription yet.
    pub async fn build_preview_subscription_details(
        &self,
        conn: &mut PgConn,
        session: &CheckoutSession,
        tenant_id: TenantId,
        coupon_code: Option<&str>,
    ) -> StoreResult<SubscriptionDetails> {
        // For subscription activation, we should use the actual subscription
        if session.checkout_type == CheckoutType::SubscriptionActivation {
            return Err(Report::new(StoreError::InvalidArgument(
                "Cannot build preview for subscription activation sessions. Use the linked subscription instead."
                    .to_string(),
            )));
        }

        let plan_with_version = self
            .store
            .get_plan_by_version_id_with_conn(
                conn,
                session.subscription_plan_version_id()?,
                tenant_id,
            )
            .await?;

        // Ensure we have version info
        let plan_version = plan_with_version.version.as_ref().ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(
                "Plan version not found".to_string(),
            ))
        })?;

        let price_components = self
            .store
            .list_price_components_with_conn(
                conn,
                session.subscription_plan_version_id()?,
                tenant_id,
            )
            .await?;

        let customer = self
            .store
            .find_customer_by_id_with_conn(conn, session.customer_id, tenant_id)
            .await?;

        let invoicing_entity_providers = InvoicingEntityProvidersRow::resolve_providers_by_id(
            conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entity: InvoicingEntity = invoicing_entity_providers.entity.clone().into();

        // Load products referenced by price components for v2 resolution
        let product_ids: Vec<ProductId> = price_components
            .iter()
            .filter_map(|c| c.product_id)
            .collect();
        let products_map: HashMap<ProductId, Product> = if product_ids.is_empty() {
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

        // Resolve overrides and extras (read-only fee computation, no row creation)
        let resolved_custom = self
            .resolve_preview_custom_components(
                conn,
                session.components.as_ref(),
                &price_components,
                &products_map,
                tenant_id,
            )
            .await?;

        // Note: This builds subscription details for invoice computation only.
        // Payment method resolution is handled separately in build_checkout_response.
        let units = crate::repositories::credit_units::load_credit_units(
            conn,
            tenant_id,
            price_components
                .iter()
                .flat_map(PriceComponent::credit_unit_ids)
                .collect::<Vec<_>>(),
        )
        .await?;
        let subscription_components = self.build_preview_components(
            &price_components,
            session.components.as_ref(),
            &products_map,
            &units,
            &resolved_custom,
        )?;

        let subscription_add_ons = self
            .build_preview_add_ons(conn, tenant_id, session.add_ons.as_ref())
            .await?;

        let billing_period = self.extract_billing_period_from_components_and_add_ons(
            &subscription_components,
            &subscription_add_ons,
        );

        let billing_start_date = session
            .billing_start_date
            .unwrap_or_else(|| common_domain::clock::now().date_naive());

        let plan_trial_days = plan_version.trial_duration_days.map(|d| d as u32);

        let effective_trial_duration: Option<u32> = session
            .trial_duration_days
            .map(|d| d as u32)
            .or(plan_trial_days)
            .filter(|&d| d > 0);

        let billing_day_anchor =
            session
                .billing_day_anchor
                .map(|a| a as u16)
                .unwrap_or_else(|| {
                    if let Some(trial_days) = effective_trial_duration {
                        let trial_end =
                            billing_start_date + chrono::Duration::days(i64::from(trial_days));
                        trial_end.day() as u16
                    } else {
                        billing_start_date.day() as u16
                    }
                });

        let net_terms = session
            .net_terms
            .map(|n| n as u32)
            .unwrap_or(plan_version.net_terms as u32);

        let trial_config = if let Some(trial_days) = effective_trial_duration {
            let trialing_plan_name = if let Some(trialing_plan_id) = &plan_version.trialing_plan_id
            {
                diesel_models::plans::PlanRow::get_overview_by_id(
                    conn,
                    *trialing_plan_id,
                    tenant_id,
                )
                .await
                .ok()
                .map(|p| p.name)
            } else {
                None
            };

            Some(TrialConfig {
                duration_days: trial_days,
                is_free: plan_version.trial_is_free,
                trialing_plan_id: plan_version.trialing_plan_id,
                trialing_plan_name,
            })
        } else {
            None
        };

        let currency = plan_version.currency.clone();
        let version = plan_version.version as u32;

        let virtual_subscription = Subscription {
            id: SubscriptionId::new(),
            customer_id: session.customer_id,
            customer_alias: customer.alias.clone(),
            customer_name: customer.name.clone(),
            billing_day_anchor,
            tenant_id,
            currency,
            trial_duration: effective_trial_duration,
            start_date: billing_start_date,
            end_date: session.end_date,
            billing_start_date: Some(billing_start_date),
            plan_id: plan_with_version.plan.id,
            plan_name: plan_with_version.plan.name.clone(),
            plan_description: plan_with_version.plan.description.clone(),
            plan_version_id: session.subscription_plan_version_id()?,
            version,
            created_at: common_domain::clock::now().naive_utc(),
            net_terms,
            invoice_memo: session.invoice_memo.clone(),
            invoice_threshold: session.invoice_threshold,
            activated_at: None,
            activation_condition: SubscriptionActivationCondition::OnCheckout,
            mrr_cents: 0,
            period: billing_period,
            pending_checkout: true,
            conn_meta: None,
            invoicing_entity_id: customer.invoicing_entity_id,
            current_period_start: billing_start_date,
            current_period_end: None,
            cycle_index: Some(0),
            status: SubscriptionStatusEnum::PendingActivation,
            auto_advance_invoices: session.auto_advance_invoices,
            charge_automatically: session.charge_automatically,
            purchase_order: session.purchase_order.clone(),
            error_count: 0,
            last_error: None,
            next_retry: None,
            quote_id: None,
            payment_methods_config: session.payment_methods_config.clone(),
            custom_properties: serde_json::json!({}),
            terminated_at: None,
            tax_inclusive: plan_version.tax_inclusive,
            // A preview books no MRR, so it needs no snapshot of the embedded rate.
            tax_inclusive_rate: None,
            usage_filter: None,
            usage_filter_since: None,
        };

        let mut applied_coupons = Vec::new();

        // The same coupons, by the same precedence, as `resolve_coupon_ids_for_checkout_tx` gives
        // the payment, checked as it checks them: a coupon it would refuse is refused here too.
        if session.coupon_ids.is_empty() {
            if let Some(code) = super::checkout_coupon_code(session, coupon_code.map(str::to_owned))
            {
                applied_coupons.push(
                    self.build_preview_coupon(conn, tenant_id, &code, &virtual_subscription)
                        .await?,
                );
            }
        } else {
            for coupon_id in &session.coupon_ids {
                applied_coupons.push(
                    self.build_preview_coupon_by_id(
                        conn,
                        tenant_id,
                        *coupon_id,
                        &virtual_subscription,
                    )
                    .await?,
                );
            }
        }

        let minimum = crate::repositories::plans::load_plan_minimum(
            conn,
            session.subscription_plan_version_id()?,
        )
        .await?
        .map(|pm| crate::domain::SubscriptionMinimum {
            subscription_id: virtual_subscription.id,
            amount_subunit: pm.amount_subunit,
            scope: pm.scope,
            is_override: false,
        });

        Ok(SubscriptionDetails {
            subscription: virtual_subscription,
            invoicing_entity,
            customer,
            schedules: Vec::new(),
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: Vec::new(),
            usage_scopes: Default::default(),
            checkout_url: None,
            trial_config,
            pending_events: Vec::new(),
            entitlements: Vec::new(),
            minimum,
        })
    }

    /// Computes the first invoice a `CreateSubscription` request would produce — including the
    /// minimum-commitment floor — without persisting anything. Powers the create-subscription
    /// wizard's "Subscription Summary": it builds a virtual `SubscriptionDetails` for the first
    /// billing period (mirroring the period derivation in `process_subscription`) and runs the
    /// same `compute_invoice` pipeline used at billing time, so the preview matches what creation
    /// will bill.
    pub async fn preview_create_subscription(
        &self,
        conn: &mut PgConn,
        create: &CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<(ComputedInvoiceContent, SubscriptionDetails)> {
        let sub = &create.subscription;

        let plan_with_version = self
            .store
            .get_plan_by_version_id_with_conn(conn, sub.plan_version_id, tenant_id)
            .await?;

        let plan_version = plan_with_version.version.as_ref().ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(
                "Plan version not found".to_string(),
            ))
        })?;

        let price_components = self
            .store
            .list_price_components_with_conn(conn, sub.plan_version_id, tenant_id)
            .await?;

        let customer = self
            .store
            .find_customer_by_id_with_conn(conn, sub.customer_id, tenant_id)
            .await?;

        let invoicing_entity_providers = InvoicingEntityProvidersRow::resolve_providers_by_id(
            conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entity: InvoicingEntity = invoicing_entity_providers.entity.clone().into();

        let product_ids: Vec<ProductId> = price_components
            .iter()
            .filter_map(|c| c.product_id)
            .collect();
        let products_map: HashMap<ProductId, Product> = if product_ids.is_empty() {
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

        let resolved_custom = self
            .resolve_preview_custom_components(
                conn,
                create.price_components.as_ref(),
                &price_components,
                &products_map,
                tenant_id,
            )
            .await?;

        let units = crate::repositories::credit_units::load_credit_units(
            conn,
            tenant_id,
            price_components
                .iter()
                .flat_map(PriceComponent::credit_unit_ids)
                .collect::<Vec<_>>(),
        )
        .await?;

        let subscription_components = self.build_preview_components(
            &price_components,
            create.price_components.as_ref(),
            &products_map,
            &units,
            &resolved_custom,
        )?;

        let subscription_add_ons = self
            .build_preview_add_ons(conn, tenant_id, create.add_ons.as_ref())
            .await?;

        let billing_period = self.extract_billing_period_from_components_and_add_ons(
            &subscription_components,
            &subscription_add_ons,
        );

        // Mirror process_subscription's first-period derivation. The summary always shows the
        // first *billable* period: for a free trial that is the post-trial period (the in-trial
        // invoice is zero), so billing starts after the trial. Paid/no trial bill immediately.
        let effective_trial_duration: Option<u32> = sub
            .trial_duration
            .or(plan_version.trial_duration_days.map(|d| d as u32))
            .filter(|&d| d > 0);
        let has_free_trial = effective_trial_duration.is_some() && plan_version.trial_is_free;

        let contract_start = sub.start_date;
        let billing_start_date = if has_free_trial {
            contract_start + Duration::days(i64::from(effective_trial_duration.unwrap()))
        } else {
            sub.billing_start_date.unwrap_or(contract_start)
        };

        let billing_day_anchor = sub
            .billing_day_anchor
            .unwrap_or_else(|| billing_start_date.day() as u16);

        let net_terms = sub.net_terms.unwrap_or(plan_version.net_terms as u32);

        let range = calculate_advance_period_range(
            billing_start_date,
            u32::from(billing_day_anchor),
            true,
            &billing_period,
        );
        let current_period_start = range.start;
        let current_period_end = Some(range.end);

        let currency = plan_version.currency.clone();
        let version = plan_version.version as u32;
        let subscription_id = SubscriptionId::new();

        // Resolve the minimum the same way creation does: an operator override wins, an explicit
        // drop removes the floor, otherwise the plan snapshot applies. Scope resolution is
        // best-effort for preview — new-product extras have no product id yet (their lines carry
        // product_id = None and never count toward a product-scoped floor anyway).
        let minimum = if let Some((amount_subunit, scope)) = &create.minimum_override {
            Some(SubscriptionMinimum {
                subscription_id,
                amount_subunit: *amount_subunit,
                scope: preview_minimum_scope(scope, &subscription_components),
                is_override: true,
            })
        } else if create.drop_minimum {
            None
        } else {
            crate::repositories::plans::load_plan_minimum(conn, sub.plan_version_id)
                .await?
                .map(|pm| SubscriptionMinimum {
                    subscription_id,
                    amount_subunit: pm.amount_subunit,
                    scope: pm.scope,
                    is_override: false,
                })
        };

        let virtual_subscription = Subscription {
            id: subscription_id,
            customer_id: sub.customer_id,
            customer_alias: customer.alias.clone(),
            customer_name: customer.name.clone(),
            billing_day_anchor,
            tenant_id,
            currency,
            trial_duration: effective_trial_duration,
            start_date: contract_start,
            end_date: sub.end_date,
            billing_start_date: Some(billing_start_date),
            plan_id: plan_with_version.plan.id,
            plan_name: plan_with_version.plan.name.clone(),
            plan_description: plan_with_version.plan.description.clone(),
            plan_version_id: sub.plan_version_id,
            version,
            created_at: common_domain::clock::now().naive_utc(),
            net_terms,
            invoice_memo: sub.invoice_memo.clone(),
            invoice_threshold: sub.invoice_threshold,
            activated_at: None,
            activation_condition: sub.activation_condition.clone(),
            mrr_cents: 0,
            period: billing_period,
            pending_checkout: false,
            conn_meta: None,
            invoicing_entity_id: customer.invoicing_entity_id,
            current_period_start,
            current_period_end,
            cycle_index: Some(0),
            // Active (not TrialActive): the minimum floor must show in the summary even for trials.
            status: SubscriptionStatusEnum::Active,
            auto_advance_invoices: sub.auto_advance_invoices,
            charge_automatically: sub.charge_automatically,
            purchase_order: sub.purchase_order.clone(),
            error_count: 0,
            last_error: None,
            next_retry: None,
            quote_id: None,
            payment_methods_config: sub.payment_methods_config.clone(),
            custom_properties: serde_json::json!({}),
            terminated_at: None,
            tax_inclusive: plan_version.tax_inclusive,
            // A preview books no MRR, so it needs no snapshot of the embedded rate.
            tax_inclusive_rate: None,
            usage_filter: sub.usage_filter.clone(),
            usage_filter_since: None,
        };

        let mut applied_coupons = Vec::new();
        if let Some(coupons) = &create.coupons {
            for c in &coupons.coupons {
                if let Ok(preview_coupon) = self
                    .build_preview_coupon_by_id(conn, tenant_id, c.coupon_id, &virtual_subscription)
                    .await
                {
                    applied_coupons.push(preview_coupon);
                }
            }
        }

        let details = SubscriptionDetails {
            subscription: virtual_subscription,
            invoicing_entity,
            customer,
            schedules: Vec::new(),
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: Vec::new(),
            usage_scopes: Default::default(),
            checkout_url: None,
            trial_config: None,
            pending_events: Vec::new(),
            entitlements: Vec::new(),
            minimum,
        };

        let content = self
            .compute_invoice(
                conn,
                &current_period_start,
                &details,
                None,
                None,
                InvoiceType::Recurring,
            )
            .await?;

        Ok((content, details))
    }

    /// Builds a SubscriptionComponent from a PriceComponent for preview purposes.
    fn build_subscription_component_from_price_component(
        &self,
        pc: crate::domain::PriceComponent,
        products: &HashMap<ProductId, Product>,
        units: &crate::domain::credit_units::CreditUnitsById,
    ) -> StoreResult<SubscriptionComponent> {
        let resolved = pc.resolve_fee(products, units, None).map_err(Report::new)?;
        Ok(SubscriptionComponent {
            id: SubscriptionPriceComponentId::new(),
            name: pc.name,
            subscription_id: SubscriptionId::new(),
            price_component_id: Some(pc.id),
            product_id: pc.product_id,
            fee: resolved.fee,
            period: resolved.period,
            price_id: resolved.price_id,
            effective_from: common_domain::clock::now().naive_utc().date(),
            effective_to: None,
            lineage_id: None,
            added_by_amendment: false,
        })
    }

    /// Extracts the billing period from subscription components and add-ons.
    fn extract_billing_period_from_components_and_add_ons(
        &self,
        components: &[SubscriptionComponent],
        add_ons: &[SubscriptionAddOn],
    ) -> BillingPeriodEnum {
        // Find the first recurring period that's not OneTime from components
        for component in components {
            if let Some(period) = component.period.as_billing_period_opt() {
                return period;
            }
        }

        // Check add-ons if no recurring period found in components
        for add_on in add_ons {
            if let Some(period) = add_on.period.as_billing_period_opt() {
                return period;
            }
        }

        // Default to monthly if no recurring period found
        BillingPeriodEnum::Monthly
    }

    /// Builds subscription components from price components, applying session customizations.
    fn build_preview_components(
        &self,
        price_components: &[PriceComponent],
        components: Option<&CreateSubscriptionComponents>,
        products: &HashMap<ProductId, Product>,
        units: &crate::domain::credit_units::CreditUnitsById,
        resolved_custom: &crate::services::subscriptions::insert::context::ResolvedCustomComponents,
    ) -> StoreResult<Vec<SubscriptionComponent>> {
        let mut processed_components = Vec::new();

        let (parameterized, remove) = if let Some(pc) = components {
            (&pc.parameterized_components, &pc.remove_components)
        } else {
            (&Vec::new(), &Vec::new())
        };

        for pc in price_components {
            let component_id = pc.id;

            if remove.contains(&component_id) {
                continue;
            }

            if let Some(param) = parameterized
                .iter()
                .find(|p| p.component_id == component_id)
            {
                use crate::domain::price_components::ComponentParameters;
                let params = ComponentParameters {
                    initial_slot_count: param.parameters.initial_slot_count,
                    billing_period: param.parameters.billing_period,
                    committed_capacity: param.parameters.committed_capacity,
                };
                let resolved = pc
                    .resolve_fee(products, units, Some(&params))
                    .map_err(Report::new)?;
                processed_components.push(SubscriptionComponent {
                    id: SubscriptionPriceComponentId::new(),
                    name: pc.name.clone(),
                    subscription_id: SubscriptionId::new(),
                    price_component_id: Some(pc.id),
                    product_id: pc.product_id,
                    fee: resolved.fee,
                    period: resolved.period,
                    price_id: resolved.price_id,
                    effective_from: common_domain::clock::now().naive_utc().date(),
                    effective_to: None,
                    lineage_id: None,
                    added_by_amendment: false,
                });
                continue;
            }

            if let Some(resolved_override) = resolved_custom.overrides.get(&component_id) {
                processed_components
                    .push(self.resolved_to_subscription_component(resolved_override));
                continue;
            }

            let comp = self.build_subscription_component_from_price_component(
                pc.clone(),
                products,
                units,
            )?;
            processed_components.push(comp);
        }

        for extra in &resolved_custom.extras {
            processed_components.push(self.resolved_to_subscription_component(extra));
        }

        Ok(processed_components)
    }

    /// Converts a ResolvedCustomComponent to a SubscriptionComponent for preview.
    fn resolved_to_subscription_component(
        &self,
        resolved: &crate::services::subscriptions::insert::context::ResolvedCustomComponent,
    ) -> SubscriptionComponent {
        SubscriptionComponent {
            id: SubscriptionPriceComponentId::new(),
            name: resolved.name.clone(),
            subscription_id: SubscriptionId::new(),
            price_component_id: resolved.price_component_id,
            product_id: resolved.existing_product_id(),
            fee: resolved.fee.clone(),
            period: resolved.period,
            price_id: resolved.existing_price_id(),
            effective_from: common_domain::clock::now().naive_utc().date(),
            effective_to: None,
            lineage_id: None,
            added_by_amendment: false,
        }
    }

    /// Resolves override and extra component fees for checkout preview (read-only, no row creation).
    async fn resolve_preview_custom_components(
        &self,
        conn: &mut PgConn,
        components: Option<&CreateSubscriptionComponents>,
        price_components: &[PriceComponent],
        products_map: &HashMap<ProductId, Product>,
        tenant_id: TenantId,
    ) -> StoreResult<crate::services::subscriptions::insert::context::ResolvedCustomComponents>
    {
        use crate::domain::price_components::{PriceEntry, ProductRef};
        use crate::services::subscriptions::insert::context::{
            ResolvedCustomComponent, ResolvedCustomComponents, resolve_fee_read_only,
        };

        let mut overrides = HashMap::new();
        let mut extras = Vec::new();

        if let Some(components) = components {
            for ov in &components.overridden_components {
                let plan_comp = price_components
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

                let product = products_map.get(&product_id).ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Product {} not found for override component {}",
                        product_id, plan_comp.id
                    )))
                })?;

                let (fee, period) =
                    resolve_fee_read_only(conn, &product.fee_structure, &ov.price_entry, tenant_id)
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

            for extra in &components.extra_components {
                let fee_structure = match &extra.product_ref {
                    ProductRef::Existing(pid) => {
                        if let Some(p) = products_map.get(pid) {
                            p.fee_structure.clone()
                        } else {
                            let row = ProductRow::find_by_id_and_tenant_id(conn, *pid, tenant_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            let product = Product::try_from(row)?;
                            product.fee_structure
                        }
                    }
                    ProductRef::New { fee_structure, .. } => fee_structure.clone(),
                };

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

        Ok(ResolvedCustomComponents { overrides, extras })
    }

    /// Builds add-ons from session.add_ons if present.
    async fn build_preview_add_ons(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        add_ons: Option<&CreateSubscriptionAddOns>,
    ) -> StoreResult<Vec<SubscriptionAddOn>> {
        let Some(create_add_ons) = add_ons else {
            return Ok(Vec::new());
        };

        if create_add_ons.add_ons.is_empty() {
            return Ok(Vec::new());
        }

        let add_on_ids: Vec<AddOnId> = create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();

        let add_ons = self
            .store
            .list_add_ons_by_ids_with_conn(conn, tenant_id, add_on_ids)
            .await?;

        // Load products and prices referenced by add-ons
        let ao_product_ids: Vec<ProductId> = add_ons.iter().map(|a| a.product_id).collect();
        let ao_products_map: HashMap<ProductId, Product> = if ao_product_ids.is_empty() {
            HashMap::new()
        } else {
            ProductRow::list_by_ids(conn, &ao_product_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Product::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?
        };

        let ao_price_ids: Vec<common_domain::ids::PriceId> =
            add_ons.iter().map(|a| a.price_id).collect();
        let ao_prices_map: HashMap<common_domain::ids::PriceId, crate::domain::prices::Price> =
            if ao_price_ids.is_empty() {
                HashMap::new()
            } else {
                diesel_models::prices::PriceRow::list_by_ids(conn, &ao_price_ids, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .map(|row| {
                        let id = row.id;
                        crate::domain::prices::Price::try_from(row).map(|p| (id, p))
                    })
                    .collect::<Result<HashMap<_, _>, _>>()?
            };

        let units = crate::repositories::credit_units::load_add_on_credit_units(
            conn,
            tenant_id,
            &ao_prices_map,
            &create_add_ons.add_ons,
        )
        .await?;

        let mut result = Vec::new();

        for create_ao in &create_add_ons.add_ons {
            let add_on = add_ons
                .iter()
                .find(|a| a.id == create_ao.add_on_id)
                .ok_or_else(|| {
                    Report::new(StoreError::ValueNotFound(format!(
                        "Add-on {} not found",
                        create_ao.add_on_id
                    )))
                })?;

            let resolved = add_on
                .resolve_customized(
                    &ao_products_map,
                    &ao_prices_map,
                    &units,
                    &create_ao.customization,
                )
                .map_err(Report::new)?;

            result.push(SubscriptionAddOn {
                id: SubscriptionAddOnId::new(),
                subscription_id: SubscriptionId::new(),
                add_on_id: add_on.id,
                name: resolved.name,
                period: resolved.period,
                fee: resolved.fee,
                product_id: resolved.product_id,
                price_id: resolved.price_id,
                created_at: common_domain::clock::now().naive_utc(),
                quantity: create_ao.quantity,
                effective_from: common_domain::clock::now().naive_utc().date(),
                effective_to: None,
                lineage_id: None,
                added_by_amendment: false,
            });
        }

        Ok(result)
    }

    /// Builds a preview AppliedCouponDetailed for invoice computation (not persisted).
    async fn build_preview_coupon(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
        subscription: &Subscription,
    ) -> StoreResult<AppliedCouponDetailed> {
        let coupon = self
            .checkout_coupon_by_code_tx(conn, tenant_id, code)
            .await?;

        coupon
            .validate_for_use_with_message(&subscription.currency)
            .map_err(|msg| Report::new(StoreError::InvalidArgument(msg)))?;
        ensure_not_redeemed_by(conn, &coupon, subscription.customer_id).await?;

        Ok(Self::create_preview_applied_coupon(coupon, subscription))
    }

    /// A code the customer typed at checkout, priced on `subscription` with the checks the payment
    /// runs, so a code that would be refused there is refused here.
    pub(crate) async fn preview_checkout_coupon(
        &self,
        tenant_id: TenantId,
        code: &str,
        subscription: &Subscription,
    ) -> StoreResult<AppliedCouponDetailed> {
        let mut conn = self.store.get_conn().await?;
        self.build_preview_coupon(&mut conn, tenant_id, code, subscription)
            .await
    }

    /// Builds a preview AppliedCouponDetailed by coupon ID (not persisted).
    async fn build_preview_coupon_by_id(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: common_domain::ids::CouponId,
        subscription: &Subscription,
    ) -> StoreResult<AppliedCouponDetailed> {
        use crate::repositories::coupons::CouponInterface;

        let coupon = self
            .store
            .get_coupon_by_id_with_conn(conn, tenant_id, coupon_id)
            .await?;

        coupon
            .validate_for_use_with_message(&subscription.currency)
            .map_err(|msg| Report::new(StoreError::InvalidArgument(msg)))?;
        ensure_not_redeemed_by(conn, &coupon, subscription.customer_id).await?;

        Ok(Self::create_preview_applied_coupon(coupon, subscription))
    }

    /// Creates a preview AppliedCouponDetailed (not persisted) for invoice computation.
    fn create_preview_applied_coupon(
        coupon: crate::domain::Coupon,
        subscription: &Subscription,
    ) -> AppliedCouponDetailed {
        let now = common_domain::clock::now().naive_utc();

        let preview_applied = AppliedCoupon {
            id: AppliedCouponId::new(),
            coupon_id: coupon.id,
            customer_id: subscription.customer_id,
            subscription_id: subscription.id,
            is_active: true,
            applied_amount: None,
            applied_count: Some(0),
            last_applied_at: None,
            created_at: now,
        };

        AppliedCouponDetailed {
            coupon,
            applied_coupon: preview_applied,
        }
    }
}

/// The once-per-customer rule `lock_and_validate_coupons_for_checkout` applies at payment.
async fn ensure_not_redeemed_by(
    conn: &mut PgConn,
    coupon: &crate::domain::Coupon,
    customer_id: common_domain::ids::CustomerId,
) -> StoreResult<()> {
    if coupon.reusable && !coupon.discount.is_credit() {
        return Ok(());
    }
    let redeemed =
        diesel_models::applied_coupons::AppliedCouponRow::find_existing_customer_coupon_pairs(
            conn,
            &[(coupon.id, customer_id)],
        )
        .await?;
    if redeemed.is_empty() {
        return Ok(());
    }
    Err(Report::new(StoreError::InvalidArgument(
        if coupon.discount.is_credit() {
            crate::services::credit::coupon_grant::CODE_ALREADY_REDEEMED.to_string()
        } else {
            format!(
                "Coupon {} has already been used by this customer",
                coupon.code
            )
        },
    )))
}

/// Best-effort resolution of an override minimum scope for preview, against the resolved preview
/// components. Unlike `CreateMinimumScope::resolve`, this never errors on by-name extras: a
/// new-product extra has no product id yet (its preview line carries `product_id = None` and is
/// excluded from a product-scoped floor anyway), so unmatched names are simply dropped.
fn preview_minimum_scope(
    scope: &CreateMinimumScope,
    components: &[SubscriptionComponent],
) -> MinimumScope {
    match scope {
        CreateMinimumScope::AllComponents => MinimumScope::AllComponents,
        CreateMinimumScope::Products {
            product_ids,
            extra_product_names,
        } => {
            let mut ids = product_ids.clone();
            for name in extra_product_names {
                if let Some(pid) = components
                    .iter()
                    .find(|c| &c.name == name)
                    .and_then(|c| c.product_id)
                {
                    ids.push(pid);
                }
            }
            MinimumScope::Products(ids)
        }
    }
}

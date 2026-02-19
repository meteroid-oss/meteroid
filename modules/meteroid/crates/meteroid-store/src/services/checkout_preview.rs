use crate::StoreResult;
use crate::domain::Product;
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::domain::subscription_coupons::AppliedCoupon;
use crate::domain::{
    AppliedCouponDetailed, BillingPeriodEnum, CheckoutSession, CheckoutType, InvoicingEntity,
    PriceComponent, Subscription, SubscriptionActivationCondition, SubscriptionComponent,
    SubscriptionDetails, SubscriptionStatusEnum, TrialConfig,
};
use crate::errors::StoreError;
use crate::repositories::add_ons::AddOnInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::plans::PlansInterface;
use crate::repositories::price_components::PriceComponentInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::{Datelike, Utc};
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
            .get_plan_by_version_id(session.plan_version_id, tenant_id)
            .await?;

        // Ensure we have version info
        let plan_version = plan_with_version.version.as_ref().ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(
                "Plan version not found".to_string(),
            ))
        })?;

        let price_components = self
            .store
            .list_price_components(session.plan_version_id, tenant_id)
            .await?;

        let customer = self
            .store
            .find_customer_by_id(session.customer_id, tenant_id)
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
                session,
                &price_components,
                &products_map,
                tenant_id,
            )
            .await?;

        // Note: This builds subscription details for invoice computation only.
        // Payment method resolution is handled separately in build_checkout_response.
        let subscription_components = self.build_preview_components(
            &price_components,
            session,
            &products_map,
            &resolved_custom,
        )?;

        let subscription_add_ons = self.build_preview_add_ons(conn, tenant_id, session).await?;

        let billing_period = self.extract_billing_period_from_components_and_add_ons(
            &subscription_components,
            &subscription_add_ons,
        );

        let billing_start_date = session
            .billing_start_date
            .unwrap_or_else(|| Utc::now().date_naive());

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
            plan_version_id: session.plan_version_id,
            version,
            created_at: Utc::now().naive_utc(),
            created_by: session.created_by,
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
        };

        let mut applied_coupons = Vec::new();

        for coupon_id in &session.coupon_ids {
            if let Ok(preview_coupon) = self
                .build_preview_coupon_by_id(conn, tenant_id, *coupon_id, &virtual_subscription)
                .await
            {
                applied_coupons.push(preview_coupon);
            }
        }

        if applied_coupons.is_empty()
            && let Some(ref code) = session.coupon_code
            && let Ok(preview_coupon) = self
                .build_preview_coupon(conn, tenant_id, code, &virtual_subscription)
                .await
        {
            applied_coupons.push(preview_coupon);
        }

        if let Some(code) = coupon_code {
            let already_added = session
                .coupon_code
                .as_ref()
                .map(|c| c == code)
                .unwrap_or(false);
            if !already_added
                && let Ok(preview_coupon) = self
                    .build_preview_coupon(conn, tenant_id, code, &virtual_subscription)
                    .await
            {
                applied_coupons.push(preview_coupon);
            }
        }

        Ok(SubscriptionDetails {
            subscription: virtual_subscription,
            invoicing_entity,
            customer,
            schedules: Vec::new(),
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: Vec::new(),
            checkout_url: None,
            trial_config,
            pending_plan_change: None,
        })
    }

    /// Builds a SubscriptionComponent from a PriceComponent for preview purposes.
    fn build_subscription_component_from_price_component(
        &self,
        pc: crate::domain::PriceComponent,
        products: &HashMap<ProductId, Product>,
    ) -> StoreResult<SubscriptionComponent> {
        let resolved = pc.resolve_fee(products, None).map_err(Report::new)?;
        Ok(SubscriptionComponent {
            id: SubscriptionPriceComponentId::new(),
            name: pc.name,
            subscription_id: SubscriptionId::new(),
            price_component_id: Some(pc.id),
            product_id: pc.product_id,
            fee: resolved.fee,
            period: resolved.period,
            price_id: resolved.price_id,
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
        session: &CheckoutSession,
        products: &HashMap<ProductId, Product>,
        resolved_custom: &crate::services::subscriptions::insert::context::ResolvedCustomComponents,
    ) -> StoreResult<Vec<SubscriptionComponent>> {
        let mut processed_components = Vec::new();

        let (parameterized, remove) = if let Some(ref pc) = session.components {
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
                    .resolve_fee(products, Some(&params))
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
                });
                continue;
            }

            if let Some(resolved_override) = resolved_custom.overrides.get(&component_id) {
                processed_components
                    .push(self.resolved_to_subscription_component(resolved_override));
                continue;
            }

            let comp =
                self.build_subscription_component_from_price_component(pc.clone(), products)?;
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
        }
    }

    /// Resolves override and extra component fees for checkout preview (read-only, no row creation).
    async fn resolve_preview_custom_components(
        &self,
        conn: &mut PgConn,
        session: &CheckoutSession,
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

        if let Some(ref components) = session.components {
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
        session: &CheckoutSession,
    ) -> StoreResult<Vec<SubscriptionAddOn>> {
        let Some(ref create_add_ons) = session.add_ons else {
            return Ok(Vec::new());
        };

        if create_add_ons.add_ons.is_empty() {
            return Ok(Vec::new());
        }

        let add_on_ids: Vec<AddOnId> = create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();

        let add_ons = self
            .store
            .list_add_ons_by_ids(tenant_id, add_on_ids)
            .await?;

        // Load products and prices referenced by add-ons
        let ao_product_ids: Vec<ProductId> = add_ons.iter().filter_map(|a| a.product_id).collect();
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
            add_ons.iter().filter_map(|a| a.price_id).collect();
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
                .resolve_customized(&ao_products_map, &ao_prices_map, &create_ao.customization)
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
                created_at: chrono::Utc::now().naive_utc(),
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
        use crate::repositories::coupons::CouponInterface;

        let coupons = self
            .store
            .list_coupons_by_codes_tx(conn, tenant_id, &[code.to_string()])
            .await?;

        let coupon = coupons.into_iter().next().ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Coupon code '{}' not found",
                code
            )))
        })?;

        coupon
            .validate_for_use_with_message(&subscription.currency)
            .map_err(|msg| Report::new(StoreError::InvalidArgument(msg)))?;

        Ok(Self::create_preview_applied_coupon(coupon, subscription))
    }

    /// Builds a preview AppliedCouponDetailed by coupon ID (not persisted).
    async fn build_preview_coupon_by_id(
        &self,
        _conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: common_domain::ids::CouponId,
        subscription: &Subscription,
    ) -> StoreResult<AppliedCouponDetailed> {
        use crate::repositories::coupons::CouponInterface;

        let coupon = self.store.get_coupon_by_id(tenant_id, coupon_id).await?;

        coupon
            .validate_for_use_with_message(&subscription.currency)
            .map_err(|msg| Report::new(StoreError::InvalidArgument(msg)))?;

        Ok(Self::create_preview_applied_coupon(coupon, subscription))
    }

    /// Creates a preview AppliedCouponDetailed (not persisted) for invoice computation.
    fn create_preview_applied_coupon(
        coupon: crate::domain::Coupon,
        subscription: &Subscription,
    ) -> AppliedCouponDetailed {
        let now = chrono::Utc::now().naive_utc();

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

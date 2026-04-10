use crate::api::customers::mapping::customer::ServerCustomerWrapper;
use crate::api::portal::checkout::PortalCheckoutServiceComponents;
use crate::api::portal::checkout::error::PortalCheckoutApiError;
use crate::api::shared::conversions::ProtoConv;
use crate::services::storage::Prefix;
use common_domain::ids::{AppliedCouponId, BaseId, CustomerPaymentMethodId, TenantId};
use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;
use error_stack::ResultExt;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutService;
use meteroid_grpc::meteroid::portal::checkout::v1::{
    AddOnPurchaseCheckoutContext, AppliedCoupon, Checkout, CheckoutType, ConfirmCheckoutRequest,
    ConfirmCheckoutResponse, ConfirmCheckoutStatus, ConfirmSlotUpgradeCheckoutRequest,
    ConfirmSlotUpgradeCheckoutResponse, GetCheckoutRequest, GetCheckoutResponse,
    GetSlotUpgradeCheckoutRequest, GetSlotUpgradeCheckoutResponse, PlanChangeCheckoutContext,
    SlotUpgradeCheckout, TaxBreakdownItem,
};
use meteroid_store::constants::Currencies;
use meteroid_store::domain::SubscriptionFeeInterface;
use meteroid_store::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutType as DomainCheckoutType,
};
use meteroid_store::domain::subscription_coupons::{
    AppliedCoupon as DomainAppliedCoupon, AppliedCouponDetailed,
};
use meteroid_store::domain::{Period, SubscriptionDetails};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::utils::periods::calculate_proration_factor;
use rust_decimal::prelude::FromPrimitive;
use std::time::Duration;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PortalCheckoutService for PortalCheckoutServiceComponents {
    /// GetCheckout - works with CheckoutSession tokens only
    #[tracing::instrument(skip_all)]
    async fn get_checkout(
        &self,
        request: Request<GetCheckoutRequest>,
    ) -> Result<Response<GetCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let portal_resource = request.portal_resource()?;
        let inner = request.into_inner();
        let coupon_code = inner.coupon_code;

        // Only accept CheckoutSession tokens
        let session_id = match portal_resource.resource_access {
            ResourceAccess::CheckoutSession(id) => id,
            _ => {
                return Err(Status::invalid_argument(
                    "Invalid token type. Expected CheckoutSession token.",
                ));
            }
        };

        let session = self
            .store
            .get_checkout_session(tenant, session_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        if session.is_expired() {
            return Err(Status::failed_precondition("Checkout session has expired"));
        }

        if session.is_completed() {
            return Err(Status::failed_precondition(
                "Checkout session has already been completed",
            ));
        }

        let (subscription_details, proto_checkout_type, plan_change_context) = match session
            .checkout_type
        {
            DomainCheckoutType::SubscriptionActivation => {
                let subscription_id = session.subscription_id.ok_or_else(|| {
                    Status::internal("Session has no linked subscription for activation flow")
                })?;

                let mut sub_details = self
                    .store
                    .get_subscription_details(tenant, subscription_id)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                if let Some(ref code) = coupon_code {
                    let preview_coupon = self
                        .validate_and_create_preview_coupon(code, tenant, &sub_details)
                        .await?;
                    sub_details.applied_coupons.push(preview_coupon);
                }

                (sub_details, CheckoutType::SubscriptionActivation, None)
            }
            DomainCheckoutType::SelfServe => {
                let sub_details = self
                    .services
                    .build_preview_subscription_details(&session, tenant, coupon_code.as_deref())
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                tracing::info!(
                    "SelfServe preview subscription: period={:?}, billing_start={:?}, current_period_start={}, current_period_end={:?}, pending_checkout={}, cycle_index={:?}, components_count={}",
                    sub_details.subscription.period,
                    sub_details.subscription.billing_start_date,
                    sub_details.subscription.current_period_start,
                    sub_details.subscription.current_period_end,
                    sub_details.subscription.pending_checkout,
                    sub_details.subscription.cycle_index,
                    sub_details.price_components.len()
                );

                for (i, comp) in sub_details.price_components.iter().enumerate() {
                    tracing::info!(
                        "  Component[{}]: name={}, period={:?}, fee={:?}",
                        i,
                        comp.name,
                        comp.period,
                        comp.fee
                    );
                }

                (sub_details, CheckoutType::SelfServe, None)
            }
            DomainCheckoutType::PlanChange => {
                let subscription_id = session.subscription_id.ok_or_else(|| {
                    Status::internal("Session has no linked subscription for plan change flow")
                })?;

                let sub_details = self
                    .store
                    .get_subscription_details(tenant, subscription_id)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let new_plan_version_id = session.plan_version_id;

                let target_plan = self
                    .store
                    .get_plan_by_version_id(new_plan_version_id, tenant)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let change_date = session.change_date.ok_or_else(|| {
                    Status::internal("PlanChange checkout session missing change_date")
                })?;

                let preview = self
                    .services
                    .preview_plan_change(
                        subscription_id,
                        tenant,
                        new_plan_version_id,
                        vec![],
                        Some(
                            meteroid_store::domain::subscription_changes::PlanChangeMode::Immediate,
                        ),
                    )
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let ctx = PlanChangeCheckoutContext {
                    current_plan_name: sub_details.subscription.plan_name.clone(),
                    new_plan_name: target_plan.plan.name.clone(),
                    effective_date: preview.preview.effective_date.as_proto(),
                    net_amount_cents: preview.proration.as_ref().map(|p| p.net_amount_cents),
                    credits_total_cents: preview.proration.as_ref().map(|p| p.credits_total_cents),
                    charges_total_cents: preview.proration.as_ref().map(|p| p.charges_total_cents),
                };

                let invoice_content = self
                    .services
                    .compute_plan_change_checkout_invoice(
                        subscription_id,
                        tenant,
                        new_plan_version_id,
                        change_date,
                    )
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let checkout = self
                    .build_checkout_response(tenant, sub_details.clone(), invoice_content)
                    .await?;

                return Ok(Response::new(GetCheckoutResponse {
                    checkout: Some(checkout),
                    checkout_type: CheckoutType::PlanChange as i32,
                    plan_change_context: Some(ctx),
                    addon_purchase_context: None,
                }));
            }
            DomainCheckoutType::AddonPurchase => {
                let subscription_id = session.subscription_id.ok_or_else(|| {
                    Status::internal("Session has no linked subscription for addon purchase flow")
                })?;

                let sub_details = self
                    .store
                    .get_subscription_details_minimal(tenant, subscription_id)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let create_add_ons = session.add_ons.as_ref().ok_or_else(|| {
                    Status::internal("AddonPurchase checkout session has no add_ons")
                })?;

                // Resolve addon fees and compute prorated charge
                let addon_ids: Vec<_> =
                    create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();
                let addons = self
                    .store
                    .list_add_ons_by_ids(tenant, addon_ids)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let addon_name = addons.first().map(|a| a.name.clone()).unwrap_or_default();

                // Build shared products/prices maps for resolution
                let mut products_map = std::collections::HashMap::new();
                let mut prices_map = std::collections::HashMap::new();
                for addon in &addons {
                    if let Some(price) = &addon.price {
                        prices_map.insert(addon.price_id, price.clone());
                    }
                    if let Some(fee_structure) = &addon.fee_structure {
                        products_map.insert(
                            addon.product_id,
                            meteroid_store::domain::Product {
                                id: addon.product_id,
                                name: addon.name.clone(),
                                description: None,
                                created_at: addon.created_at,
                                created_by: uuid::Uuid::nil(),
                                updated_at: None,
                                archived_at: None,
                                tenant_id: addon.tenant_id,
                                product_family_id: common_domain::ids::ProductFamilyId::from(
                                    uuid::Uuid::nil(),
                                ),
                                fee_type: addon
                                    .fee_type
                                    .unwrap_or(meteroid_store::domain::enums::FeeTypeEnum::Rate),
                                fee_structure: fee_structure.clone(),
                                catalog: false,
                            },
                        );
                    }
                }

                let now = chrono::Utc::now().date_naive();

                let mut sub_details = sub_details;

                for (cs_ao, addon) in create_add_ons.add_ons.iter().zip(addons.iter()) {
                    let resolved = addon
                        .resolve_customized(&products_map, &prices_map, &cs_ao.customization)
                        .map_err(|e| {
                            Status::internal(format!("Failed to resolve add-on: {}", e))
                        })?;

                    sub_details.add_ons.push(
                        meteroid_store::domain::subscription_add_ons::SubscriptionAddOn {
                            id: common_domain::ids::SubscriptionAddOnId::new(),
                            subscription_id,
                            add_on_id: addon.id,
                            name: resolved.name,
                            period: resolved.period,
                            fee: resolved.fee,
                            created_at: chrono::Utc::now().naive_utc(),
                            product_id: resolved.product_id,
                            price_id: resolved.price_id,
                            quantity: cs_ao.quantity,
                        },
                    );
                }

                // Override billing_start_date and cycle_index so that compute_invoice
                // treats the addon as a first-period item starting now, producing a
                // prorated advance line (from now to period_end).
                sub_details.subscription.billing_start_date = Some(now);
                sub_details.subscription.cycle_index = Some(0);

                let invoice_content = self
                    .services
                    .compute_invoice(&now, &sub_details, None)
                    .await
                    .change_context(StoreError::InvoiceComputationError)
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let ctx = AddOnPurchaseCheckoutContext {
                    add_on_name: addon_name,
                    prorated_amount_cents: invoice_content.total,
                };

                let checkout = self
                    .build_checkout_response(tenant, sub_details, invoice_content)
                    .await?;

                return Ok(Response::new(GetCheckoutResponse {
                    checkout: Some(checkout),
                    checkout_type: CheckoutType::AddonPurchase as i32,
                    plan_change_context: None,
                    addon_purchase_context: Some(ctx),
                }));
            }
        };

        let invoice_content = self
            .services
            .compute_invoice(
                &subscription_details.subscription.current_period_start,
                &subscription_details,
                None,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        tracing::info!(
            "Invoice content: invoice_lines_count={}, subtotal={}, total={}",
            invoice_content.invoice_lines.len(),
            invoice_content.subtotal,
            invoice_content.total
        );

        let checkout = self
            .build_checkout_response(tenant, subscription_details.clone(), invoice_content)
            .await?;

        tracing::info!(
            "Checkout response: payment_methods_count={}, card_connection_id={:?}, direct_debit_connection_id={:?}, bank_account={:?}",
            checkout.payment_methods.len(),
            checkout.card_connection_id,
            checkout.direct_debit_connection_id,
            checkout.bank_account.is_some()
        );

        Ok(Response::new(GetCheckoutResponse {
            checkout: Some(checkout),
            checkout_type: proto_checkout_type as i32,
            plan_change_context,
            addon_purchase_context: None,
        }))
    }

    /// ConfirmCheckout - works with CheckoutSession tokens only
    #[tracing::instrument(skip_all)]
    async fn confirm_checkout(
        &self,
        request: Request<ConfirmCheckoutRequest>,
    ) -> Result<Response<ConfirmCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let portal_resource = request.portal_resource()?;
        let inner = request.into_inner();

        // Only accept CheckoutSession tokens
        let session_id = match portal_resource.resource_access {
            ResourceAccess::CheckoutSession(id) => id,
            _ => {
                return Err(Status::invalid_argument(
                    "Invalid token type. Expected CheckoutSession token.",
                ));
            }
        };

        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        let result = self
            .services
            .complete_checkout(
                tenant,
                session_id,
                payment_method_id,
                inner.displayed_amount,
                inner.displayed_currency,
                inner.coupon_code,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let (subscription_id, transaction, status) = match result {
            CheckoutCompletionResult::Completed {
                subscription_id,
                transaction,
            } => (
                Some(subscription_id.as_proto()),
                transaction,
                ConfirmCheckoutStatus::Completed,
            ),
            CheckoutCompletionResult::AwaitingPayment { transaction } => (
                None,
                Some(transaction),
                ConfirmCheckoutStatus::AwaitingPayment,
            ),
        };

        Ok(Response::new(ConfirmCheckoutResponse {
            transaction: transaction
                .map(crate::api::invoices::mapping::transactions::domain_to_server),
            subscription_id,
            status: status as i32,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_slot_upgrade_checkout(
        &self,
        request: Request<GetSlotUpgradeCheckoutRequest>,
    ) -> Result<Response<GetSlotUpgradeCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;

        let inner = request.into_inner();

        let subscription_id =
            common_domain::ids::SubscriptionId::from_proto(inner.subscription_id)?;
        let price_component_id =
            common_domain::ids::PriceComponentId::from_proto(inner.price_component_id)?;

        let subscription_details = self
            .store
            .get_subscription_details(tenant, subscription_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        // Verify subscription belongs to the customer
        if subscription_details.subscription.customer_id != customer_id {
            return Err(Status::permission_denied(
                "Subscription does not belong to the customer",
            ));
        }

        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Price component {} not found",
                    price_component_id
                ))
            })?;

        let unit_name = match slot_component.fee_ref() {
            meteroid_store::domain::SubscriptionFee::Slot { unit, .. } => unit.clone(),
            _ => {
                return Err(tonic::Status::invalid_argument(
                    "Price component is not a slot component",
                ));
            }
        };

        let result = self
            .services
            .update_subscription_slots(
                tenant,
                subscription_id,
                price_component_id,
                inner.delta,
                meteroid_store::domain::SlotUpgradeBillingMode::OnCheckout,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(subscription_details.subscription.customer_id, tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        // Resolve payment methods based on subscription's config
        let resolved = self
            .services
            .resolve_subscription_payment_methods(
                tenant,
                subscription_details
                    .subscription
                    .payment_methods_config
                    .as_ref(),
                &customer,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let payment_methods = resolved
            .filter_payment_methods(customer_methods)
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let currency_code = subscription_details.subscription.currency.clone();
        let currency = Currencies::resolve_currency(&currency_code).ok_or_else(|| {
            tonic::Status::internal(format!("Currency {} not found", currency_code))
        })?;

        let subscription_proto =
            crate::api::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription_details,
            )?;

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            self.object_store
                .get_url(
                    logo_attachment_id,
                    Prefix::ImageLogo,
                    Duration::from_secs(3600 * 24),
                )
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        } else {
            None
        };

        let prorated_amount_subunits = result
            .prorated_amount
            .and_then(|amount| amount.to_subunit_opt(currency.precision))
            .unwrap_or(0);

        if prorated_amount_subunits <= 0 {
            return Err(tonic::Status::invalid_argument(
                "Prorated change is 0 or negative",
            ));
        }

        let prorated_amount_subunits = prorated_amount_subunits.to_non_negative_u64();

        Ok(Response::new(GetSlotUpgradeCheckoutResponse {
            checkout: Some(SlotUpgradeCheckout {
                subscription: Some(subscription_proto),
                customer: Some(customer_proto),
                payment_methods,
                price_component_id: price_component_id.as_proto(),
                unit_name,
                current_slot_count: (result.new_slot_count - inner.delta) as u32,
                new_slot_count: result.new_slot_count as u32,
                delta: inner.delta,
                prorated_amount: prorated_amount_subunits,
                currency: currency_code,
                logo_url,
                trade_name: organization.trade_name,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_slot_upgrade_checkout(
        &self,
        request: Request<ConfirmSlotUpgradeCheckoutRequest>,
    ) -> Result<Response<ConfirmSlotUpgradeCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;

        let inner = request.into_inner();

        let subscription_id =
            common_domain::ids::SubscriptionId::from_proto(inner.subscription_id)?;
        let price_component_id =
            common_domain::ids::PriceComponentId::from_proto(inner.price_component_id)?;
        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        let subscription_details = self
            .store
            .get_subscription_details(tenant, subscription_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        // Verify subscription belongs to the customer
        if subscription_details.subscription.customer_id != customer_id {
            return Err(Status::permission_denied(
                "Subscription does not belong to the customer",
            ));
        }

        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| tonic::Status::not_found("Price component not found"))?;

        let (unit_rate, _unit) = match slot_component.fee_ref() {
            meteroid_store::domain::SubscriptionFee::Slot {
                unit_rate, unit, ..
            } => (unit_rate, unit),
            _ => return Err(tonic::Status::invalid_argument("Not a slot component")),
        };

        let now = chrono::Utc::now().date_naive();
        let period_end = subscription_details
            .subscription
            .current_period_end
            .ok_or_else(|| tonic::Status::invalid_argument("No current_period_end"))?;

        let full_period = Period {
            start: subscription_details.subscription.current_period_start,
            end: period_end,
        };
        let partial_period = Period {
            start: now,
            end: period_end,
        };
        let proration_factor = calculate_proration_factor(&partial_period, &full_period);
        let base_amount = rust_decimal::Decimal::from(inner.delta) * unit_rate;
        let prorated = if let Some(factor) = proration_factor {
            base_amount
                * rust_decimal::Decimal::from_f64(factor).unwrap_or(rust_decimal::Decimal::ONE)
        } else {
            base_amount
        };
        let currency = Currencies::resolve_currency(&subscription_details.subscription.currency)
            .ok_or_else(|| {
                tonic::Status::internal(format!(
                    "Currency {} not found",
                    subscription_details.subscription.currency
                ))
            })?;
        let expected_amount_subunits = prorated
            .max(rust_decimal::Decimal::ZERO)
            .to_subunit_opt(currency.precision)
            .unwrap_or(0);

        // Validate with tolerance
        let displayed_amount = inner.displayed_amount;
        let diff = (expected_amount_subunits as i64 - displayed_amount as i64).abs();
        if diff > 1 {
            return Err(tonic::Status::invalid_argument(format!(
                "Amount mismatch: displayed {} but expected {} (currency: {})",
                displayed_amount, expected_amount_subunits, inner.displayed_currency
            )));
        }

        if inner.displayed_currency != subscription_details.subscription.currency {
            return Err(tonic::Status::invalid_argument(format!(
                "Currency mismatch: displayed {} but expected {}",
                inner.displayed_currency, subscription_details.subscription.currency
            )));
        }

        let (transaction, new_slot_count) = self
            .services
            .complete_slot_upgrade_checkout(
                tenant,
                subscription_id,
                price_component_id,
                inner.delta,
                payment_method_id,
                None,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        Ok(Response::new(ConfirmSlotUpgradeCheckoutResponse {
            transaction: Some(
                crate::api::invoices::mapping::transactions::domain_to_server(transaction),
            ),
            new_slot_count: new_slot_count as u32,
        }))
    }
}

impl PortalCheckoutServiceComponents {
    /// Builds the Checkout response from subscription details and invoice content.
    async fn build_checkout_response(
        &self,
        tenant: TenantId,
        subscription_details: SubscriptionDetails,
        invoice_content: meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent,
    ) -> Result<Checkout, Status> {
        let customer = &subscription_details.customer;

        let invoicing_entity = &subscription_details.invoicing_entity;

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        // Resolve payment methods at runtime based on subscription's config
        let resolved = self
            .services
            .resolve_subscription_payment_methods(
                tenant,
                subscription_details
                    .subscription
                    .payment_methods_config
                    .as_ref(),
                customer,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let card_connection_id = resolved.card_connection_id;
        let direct_debit_connection_id = resolved.direct_debit_connection_id;

        // Fetch customer payment methods and filter to only those usable with resolved connections
        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let payment_methods = resolved
            .filter_payment_methods(customer_methods)
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let bank_account = if let Some(bank_account_id) = resolved.bank_account_id {
            self.store
                .get_bank_account_by_id(bank_account_id, tenant)
                .await
                .ok()
                .map(crate::api::bankaccounts::mapping::bank_accounts::domain_to_proto)
        } else {
            None
        };

        let subscription_proto =
            crate::api::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription_details.clone(),
            )?;

        let customer_proto = ServerCustomerWrapper::try_from(subscription_details.customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            self.object_store
                .get_url(
                    logo_attachment_id,
                    Prefix::ImageLogo,
                    Duration::from_secs(7 * 86400),
                )
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        } else {
            None
        };

        let invoice_lines = crate::api::invoices::mapping::invoices::domain_invoice_lines_to_server(
            invoice_content.invoice_lines,
        );

        let tax_breakdown = invoice_content
            .tax_breakdown
            .into_iter()
            .map(|item| TaxBreakdownItem {
                name: item.name,
                rate: item.tax_rate.to_string(),
                amount: item.tax_amount,
            })
            .collect();

        let applied_coupons = invoice_content
            .applied_coupons
            .clone()
            .into_iter()
            .map(|coupon| AppliedCoupon {
                coupon_code: coupon.code,
                coupon_name: coupon.name,
                amount: coupon.value.to_non_negative_u64(),
            })
            .collect();

        let coupon_amount: i64 = invoice_content
            .applied_coupons
            .iter()
            .map(|c| c.value)
            .sum();

        Ok(Checkout {
            subscription: Some(subscription_proto),
            customer: Some(customer_proto),
            invoice_lines,
            logo_url,
            trade_name: organization.trade_name,
            payment_methods,
            total_amount: invoice_content.total.to_non_negative_u64(),
            subtotal_amount: invoice_content.subtotal.to_non_negative_u64(),
            tax_amount: invoice_content.tax_amount.to_non_negative_u64(),
            discount_amount: invoice_content.discount.to_non_negative_u64(),
            applied_credits: invoice_content.applied_credits.to_non_negative_u64(),
            amount_due: invoice_content.amount_due.to_non_negative_u64(),
            coupon_amount: coupon_amount.to_non_negative_u64(),
            tax_breakdown,
            applied_coupons,
            card_connection_id: card_connection_id.map(|id| id.as_proto()),
            direct_debit_connection_id: direct_debit_connection_id.map(|id| id.as_proto()),
            bank_account,
        })
    }

    /// Validates a coupon code and creates a preview AppliedCouponDetailed for invoice computation.
    /// This does NOT persist the coupon - it's only used for previewing the discount.
    async fn validate_and_create_preview_coupon(
        &self,
        code: &str,
        tenant_id: TenantId,
        subscription: &SubscriptionDetails,
    ) -> Result<AppliedCouponDetailed, Status> {
        let coupons = self
            .store
            .list_coupons_by_codes(tenant_id, &[code.to_string()])
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let coupon = coupons
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument(format!("Coupon code '{}' not found", code)))?;

        coupon
            .validate_for_use_with_message(&subscription.subscription.currency)
            .map_err(Status::invalid_argument)?;

        // Preview only - not persisted
        let now = chrono::Utc::now().naive_utc();
        let preview_applied = DomainAppliedCoupon {
            id: AppliedCouponId::new(),
            coupon_id: coupon.id,
            customer_id: subscription.subscription.customer_id,
            subscription_id: subscription.subscription.id,
            is_active: true,
            applied_amount: None,
            applied_count: Some(0),
            last_applied_at: None,
            created_at: now,
        };

        Ok(AppliedCouponDetailed {
            coupon,
            applied_coupon: preview_applied,
        })
    }
}

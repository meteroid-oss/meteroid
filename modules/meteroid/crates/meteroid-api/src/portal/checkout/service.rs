use crate::customers::mapping::customer::ServerCustomerWrapper;
use crate::portal::checkout::PortalCheckoutServiceComponents;
use crate::portal::checkout::error::PortalCheckoutApiError;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CreditUnitId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, TenantId,
};
use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;
use error_stack::ResultExt;
use meteroid_app::conversions::ProtoConv;
use meteroid_app::services::storage;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutService;
use meteroid_grpc::meteroid::portal::checkout::v1::{
    AddOnPurchaseCheckoutContext, AppliedCoupon, Checkout, CheckoutType, ConfirmCheckoutRequest,
    ConfirmCheckoutResponse, ConfirmCheckoutStatus, ConfirmSlotUpgradeCheckoutRequest,
    ConfirmSlotUpgradeCheckoutResponse, GetCheckoutRequest, GetCheckoutResponse,
    GetSlotUpgradeCheckoutRequest, GetSlotUpgradeCheckoutResponse, InitiateHostedCheckoutRequest,
    InitiateHostedCheckoutResponse, PlanChangeCheckoutContext, PromotionalCreditCoupon,
    SlotUpgradeCheckout, TaxBreakdownItem, TopUpCredits,
};
use meteroid_store::constants::Currencies;
use meteroid_store::domain::SubscriptionFeeInterface;
use meteroid_store::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutSession as DomainCheckoutSession,
    CheckoutType as DomainCheckoutType,
};
use meteroid_store::domain::coupons::CouponDiscount;
use meteroid_store::domain::subscription_coupons::AppliedCouponDetailed;
use meteroid_store::domain::{Period, SubscriptionDetails};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::purchase_links::PurchaseLinksInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::services::invoice_lines::discount::calculate_next_invoice_coupons_discount;
use meteroid_store::utils::periods::calculate_proration_factor;
use std::time::Duration;
use tonic::{Request, Response, Status};

/// A delegated session cannot buy credit, as its credentials cannot.
fn delegated_top_up_denied() -> Status {
    Status::permission_denied(
        "This operation is not available in a portal session opened by a platform.",
    )
}

/// A promo code tried at checkout, holding one try of the customer's code budget until settled.
struct CouponCodeTry {
    tenant: TenantId,
    customer_id: CustomerId,
    code_try: crate::portal::wallets::CodeTry,
}

impl PortalCheckoutServiceComponents {
    /// A typed code takes one try from the budget portal redemptions draw on, so checkout cannot
    /// probe codes; `settle_coupon_code` gives it back once the code proves usable.
    async fn try_coupon_code(
        &self,
        tenant: TenantId,
        customer_id: CustomerId,
        code: Option<&str>,
    ) -> Result<Option<CouponCodeTry>, Status> {
        if code.is_none_or(|code| code.trim().is_empty()) {
            return Ok(None);
        }
        let code_try = self.promo_codes.try_code(tenant, customer_id).await?;
        Ok(Some(CouponCodeTry {
            tenant,
            customer_id,
            code_try,
        }))
    }

    async fn settle_coupon_code(&self, code_try: Option<CouponCodeTry>, usable: bool) {
        if let Some(CouponCodeTry {
            tenant,
            customer_id,
            code_try,
        }) = code_try
        {
            self.promo_codes
                .settle(tenant, customer_id, code_try, usable)
                .await;
        }
    }

    async fn rail_capabilities(
        &self,
        tenant_id: common_domain::ids::TenantId,
        connection_id: Option<common_domain::ids::CustomerConnectionId>,
    ) -> Result<
        Option<meteroid_grpc::meteroid::api::connectors::v1::PaymentProviderCapabilities>,
        PortalCheckoutApiError,
    > {
        let Some(connection_id) = connection_id else {
            return Ok(None);
        };
        let connection = self
            .store
            .get_connection_by_id(&tenant_id, &connection_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let connector = self
            .store
            .get_connector_with_data(connection.connector_id, tenant_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        Ok(
            meteroid_store::adapters::payment::payment_capabilities(&connector.provider())
                .map(crate::connectors::mapping::connectors::capabilities_to_server),
        )
    }

    /// The customer cannot list the tenant's credit units: fees priced in one carry its names.
    async fn name_credit_units(
        &self,
        tenant_id: TenantId,
        details: &mut meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionDetails,
    ) -> Result<(), Status> {
        use meteroid_store::repositories::CreditUnitsInterface;

        let mut units: Vec<_> = details
            .price_components
            .iter_mut()
            .filter_map(|component| component.fee.as_mut())
            .chain(
                details
                    .add_ons
                    .iter_mut()
                    .filter_map(|add_on| add_on.fee.as_mut()),
            )
            .filter_map(|fee| fee.credit_unit.as_mut())
            .collect();
        if units.is_empty() {
            return Ok(());
        }
        let mut ids = units
            .iter()
            .map(|unit| CreditUnitId::from_proto(&unit.credit_unit_id))
            .collect::<Result<Vec<_>, _>>()?;
        ids.sort_by_key(|id| id.as_uuid());
        ids.dedup();
        let catalog = self
            .store
            .get_credit_units_by_ids(tenant_id, ids)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        for unit in &mut units {
            if let Some(named) = CreditUnitId::from_proto(&unit.credit_unit_id)
                .ok()
                .and_then(|id| catalog.get(&id))
            {
                unit.name = Some(named.name.clone());
                unit.plural_name = Some(named.plural_name.clone());
            }
        }
        Ok(())
    }
}

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

        if session.checkout_type == DomainCheckoutType::TopUp {
            if portal_resource.delegated {
                return Err(delegated_top_up_denied());
            }
            return Ok(Response::new(GetCheckoutResponse {
                checkout: Some(self.build_top_up_checkout(&session, tenant).await?),
                checkout_type: CheckoutType::TopUp as i32,
                success_url: session.success_url,
                cancel_url: session.cancel_url,
                ..Default::default()
            }));
        }
        let code_try = self
            .try_coupon_code(tenant, session.customer_id, coupon_code.as_deref())
            .await?;
        let computation = self
            .compute_checkout_invoice(&session, tenant, coupon_code)
            .await;
        self.settle_coupon_code(code_try, computation.is_ok()).await;
        let computation = computation?;

        let checkout = self
            .build_checkout_response(
                tenant,
                computation.subscription_details,
                computation.invoice_content,
            )
            .await?;

        Ok(Response::new(GetCheckoutResponse {
            checkout: Some(checkout),
            checkout_type: computation.checkout_type as i32,
            plan_change_context: computation.plan_change_context,
            addon_purchase_context: computation.addon_purchase_context,
            success_url: session
                .success_url
                .as_deref()
                .map(|u| with_checkout_session_id(u, session.id)),
            cancel_url: session.cancel_url.clone(),
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

        let has_code = inner
            .coupon_code
            .as_deref()
            .is_some_and(|code| !code.trim().is_empty());
        let session = if has_code || portal_resource.delegated {
            Some(
                self.store
                    .get_checkout_session(tenant, session_id)
                    .await
                    .map_err(Into::<PortalCheckoutApiError>::into)?,
            )
        } else {
            None
        };
        if portal_resource.delegated
            && session
                .as_ref()
                .is_some_and(|session| session.checkout_type == DomainCheckoutType::TopUp)
        {
            return Err(delegated_top_up_denied());
        }
        let code_try = match &session {
            Some(session) if has_code => {
                self.try_coupon_code(tenant, session.customer_id, inner.coupon_code.as_deref())
                    .await?
            }
            _ => None,
        };
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
            .await;
        self.settle_coupon_code(code_try, result.is_ok()).await;
        let result = result.map_err(Into::<PortalCheckoutApiError>::into)?;

        let (subscription_id, transaction, status, next_action) = match result {
            CheckoutCompletionResult::TopUpCompleted { transaction } => (
                None,
                Some(transaction),
                ConfirmCheckoutStatus::Completed,
                None,
            ),
            CheckoutCompletionResult::Completed {
                subscription_id,
                transaction,
            } => (
                Some(subscription_id.as_proto()),
                transaction,
                ConfirmCheckoutStatus::Completed,
                None,
            ),
            CheckoutCompletionResult::AwaitingPayment {
                transaction,
                next_action,
            } => (
                None,
                Some(transaction),
                ConfirmCheckoutStatus::AwaitingPayment,
                next_action,
            ),
        };

        Ok(Response::new(ConfirmCheckoutResponse {
            transaction: transaction
                .map(crate::invoices::mapping::transactions::domain_to_server)
                .map(crate::portal::customer_facing_transaction),
            subscription_id,
            status: status as i32,
            next_action: next_action
                .map(crate::invoices::mapping::payment_action::domain_to_server),
        }))
    }

    /// InitiateHostedCheckout - works with CheckoutSession tokens only.
    /// Starts a hosted-redirect flow (GoCardless, Stancer) for the
    /// server-computed first payment, collected in the hosted flow itself —
    /// the return handler (or the sweeper on a lost return) records the
    /// captured payment, it never charges again.
    #[tracing::instrument(skip_all)]
    async fn initiate_hosted_checkout(
        &self,
        request: Request<InitiateHostedCheckoutRequest>,
    ) -> Result<Response<InitiateHostedCheckoutResponse>, Status> {
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

        let connection_id = CustomerConnectionId::from_proto(inner.connection_id)?;

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

        // Authoritative server-side amount — the client figure is only checked
        // against it, never charged. Keep the coupon to hand to initiate so it's
        // persisted on the session and re-applied at fulfillment (else the amount
        // computed here with the coupon won't match the invoice built later).
        let coupon_code = inner.coupon_code.clone();
        let code_try = self
            .try_coupon_code(tenant, session.customer_id, coupon_code.as_deref())
            .await?;
        let computation = self
            .compute_checkout_invoice(&session, tenant, inner.coupon_code)
            .await;
        self.settle_coupon_code(code_try, computation.is_ok()).await;
        let computation = computation?;
        let amount_due = computation.invoice_content.amount_due.to_non_negative_u64();
        let currency = computation
            .subscription_details
            .subscription
            .currency
            .clone();

        if inner.displayed_amount != amount_due || inner.displayed_currency != currency {
            return Err(Status::failed_precondition(
                "The checkout total has changed. Please refresh the page and retry.",
            ));
        }

        let rail = inner
            .connection_type
            .and_then(|raw| {
                meteroid_grpc::meteroid::portal::shared::v1::ConnectionTypeEnum::try_from(raw).ok()
            })
            .map(|ct| match ct {
                meteroid_grpc::meteroid::portal::shared::v1::ConnectionTypeEnum::Card => {
                    meteroid_store::domain::ConnectionTypeEnum::Card
                }
                meteroid_grpc::meteroid::portal::shared::v1::ConnectionTypeEnum::DirectDebit => {
                    meteroid_store::domain::ConnectionTypeEnum::DirectDebit
                }
            });

        let result = self
            .services
            .initiate_hosted_checkout(
                tenant,
                session_id,
                connection_id,
                amount_due as i64,
                currency,
                coupon_code,
                rail,
                inner.return_url,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        match result {
            CheckoutCompletionResult::AwaitingPayment {
                transaction,
                next_action,
            } => {
                let next_action = next_action.ok_or_else(|| {
                    Status::internal("Hosted checkout returned no redirect action")
                })?;
                Ok(Response::new(InitiateHostedCheckoutResponse {
                    transaction: Some(crate::portal::customer_facing_transaction(
                        crate::invoices::mapping::transactions::domain_to_server(transaction),
                    )),
                    next_action: Some(crate::invoices::mapping::payment_action::domain_to_server(
                        next_action,
                    )),
                }))
            }
            CheckoutCompletionResult::Completed { .. }
            | CheckoutCompletionResult::TopUpCompleted { .. } => Err(Status::internal(
                "Hosted checkout unexpectedly completed synchronously",
            )),
        }
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_slot_upgrade_checkout(
        &self,
        request: Request<GetSlotUpgradeCheckoutRequest>,
    ) -> Result<Response<GetSlotUpgradeCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;

        let inner = request.into_inner();

        // A read-only token can call this preview; a negative delta would book a downgrade.
        if inner.delta <= 0 {
            return Err(Status::invalid_argument(
                "Slot upgrades via checkout must have positive delta",
            ));
        }

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
                common_domain::actor::Actor::Customer { id: customer_id },
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

        let branding = crate::portal::branding::resolve_portal_branding(
            &self.store,
            tenant,
            &invoicing_entity,
        )
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
            .map(crate::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let currency_code = subscription_details.subscription.currency.clone();
        let currency = Currencies::resolve_currency(&currency_code).ok_or_else(|| {
            tonic::Status::internal(format!("Currency {} not found", currency_code))
        })?;

        let mut subscription_proto =
            crate::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription_details,
            )?;
        self.name_credit_units(tenant, &mut subscription_proto)
            .await?;

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = match branding.logo_attachment_id {
            Some(id) => storage::signed_url(
                self.object_store.as_ref(),
                &self.store,
                id,
                Duration::from_secs(3600 * 24),
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?,
            None => None,
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
                brand_color: branding.brand_color,
                theme_mode: branding.theme_mode,
                roundness: branding.roundness,
            }),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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
            factor.prorate_decimal(base_amount)
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

        let (transaction, new_slot_count, next_action) = self
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
            transaction: Some(crate::portal::customer_facing_transaction(
                crate::invoices::mapping::transactions::domain_to_server(transaction),
            )),
            new_slot_count: new_slot_count as u32,
            next_action: next_action
                .map(crate::invoices::mapping::payment_action::domain_to_server),
        }))
    }
}

impl PortalCheckoutServiceComponents {
    /// Computes the authoritative first-invoice content for a checkout session,
    /// per checkout_type. Shared by `get_checkout` (full checkout response) and
    /// `initiate_hosted_checkout` (server-side amount/currency validation).
    async fn compute_checkout_invoice(
        &self,
        session: &DomainCheckoutSession,
        tenant: TenantId,
        coupon_code: Option<String>,
    ) -> Result<CheckoutInvoiceComputation, Status> {
        let buyer_code = coupon_code
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty() && session.coupon_code.as_deref() != Some(*c));
        if buyer_code.is_some()
            && !self
                .store
                .checkout_accepts_coupon_codes(tenant, session.id)
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        {
            return Err(PortalCheckoutApiError::InvalidCoupon(
                "Coupon codes are not accepted for this purchase".to_string(),
            )
            .into());
        }

        let (subscription_details, proto_checkout_type, plan_change_context) = match &session
            .checkout_type
        {
            DomainCheckoutType::TopUp => {
                return Err(Status::invalid_argument("top-ups use card checkout"));
            }
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
                    .build_preview_subscription_details(session, tenant, coupon_code.as_deref())
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

                let new_plan_version_id = session
                    .subscription_plan_version_id()
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

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

                return Ok(CheckoutInvoiceComputation {
                    subscription_details: sub_details,
                    invoice_content,
                    checkout_type: CheckoutType::PlanChange,
                    plan_change_context: Some(ctx),
                    addon_purchase_context: None,
                });
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
                                tax_category_id: None,
                            },
                        );
                    }
                }

                let units = {
                    use meteroid_store::repositories::CreditUnitsInterface;
                    self.store
                        .get_credit_units_by_ids(
                            tenant,
                            prices_map
                                .values()
                                .flat_map(meteroid_store::domain::Price::credit_unit_ids)
                                .chain(
                                    create_add_ons
                                        .add_ons
                                        .iter()
                                        .flat_map(|a| a.customization.new_credit_unit_ids()),
                                )
                                .collect(),
                        )
                        .await
                        .map_err(Into::<PortalCheckoutApiError>::into)?
                };

                let now = chrono::Utc::now().date_naive();

                let mut sub_details = sub_details;

                for (cs_ao, addon) in create_add_ons.add_ons.iter().zip(addons.iter()) {
                    let resolved = addon
                        .resolve_customized(
                            &products_map,
                            &prices_map,
                            &units,
                            &cs_ao.customization,
                        )
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
                            effective_from: chrono::Utc::now().naive_utc().date(),
                            effective_to: None,
                            lineage_id: None,
                            added_by_amendment: false,
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
                    .compute_invoice(
                        &now,
                        &sub_details,
                        None,
                        meteroid_store::domain::enums::InvoiceType::OneOff,
                    )
                    .await
                    .change_context(StoreError::InvoiceComputationError)
                    .map_err(Into::<PortalCheckoutApiError>::into)?;

                let ctx = AddOnPurchaseCheckoutContext {
                    add_on_name: addon_name,
                    prorated_amount_cents: invoice_content.total,
                };

                return Ok(CheckoutInvoiceComputation {
                    subscription_details: sub_details,
                    invoice_content,
                    checkout_type: CheckoutType::AddonPurchase,
                    plan_change_context: None,
                    addon_purchase_context: Some(ctx),
                });
            }
        };

        let invoice_content = self
            .services
            .compute_invoice(
                &subscription_details.subscription.current_period_start,
                &subscription_details,
                None,
                meteroid_store::domain::enums::InvoiceType::Recurring,
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

        Ok(CheckoutInvoiceComputation {
            subscription_details,
            invoice_content,
            checkout_type: proto_checkout_type,
            plan_change_context,
            addon_purchase_context: None,
        })
    }

    async fn build_top_up_checkout(
        &self,
        session: &DomainCheckoutSession,
        tenant: TenantId,
    ) -> Result<Checkout, Status> {
        let purchase = session
            .top_up()
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let customer = self
            .store
            .find_customer_by_id(session.customer_id, tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let entity = self
            .store
            .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let branding =
            crate::portal::branding::resolve_portal_branding(&self.store, tenant, &entity)
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?;
        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let show_powered_by = !self
            .feature_resolver
            .resolve(organization.id)
            .await
            .is_enabled(meteroid_app::features::gates::WHITELABEL);
        let resolved = self
            .services
            .resolve_subscription_payment_methods(tenant, None, &customer)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let card_capabilities = self
            .rail_capabilities(tenant, resolved.card_connection_id)
            .await?;
        let logo_url = match branding.logo_attachment_id {
            Some(id) => storage::signed_url(
                self.object_store.as_ref(),
                &self.store,
                id,
                Duration::from_secs(7 * 86400),
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?,
            None => None,
        };
        let charge = purchase.charge_minor().to_non_negative_u64();
        let net = purchase.net_minor().to_non_negative_u64();
        let tax = purchase.tax_minor().to_non_negative_u64();
        // Display only: the page still sells the credits without their label.
        let top_up_credits = match purchase.unit_sale {
            Some(_) => match self
                .store
                .customer_credit_wallet(tenant, purchase.wallet_id)
                .await
            {
                Ok(wallet) => {
                    wallet
                        .and_then(|wallet| wallet.credit_unit)
                        .map(|unit| TopUpCredits {
                            amount: purchase.amount_minor,
                            credit_unit: Some(crate::portal::pricing::credit_unit_label(&unit)),
                        })
                }
                Err(error) => {
                    tracing::warn!(
                        wallet_id = %purchase.wallet_id,
                        "top-up checkout shown without its credits: {error:?}"
                    );
                    None
                }
            },
            None => None,
        };
        Ok(Checkout {
            top_up_currency: Some(purchase.currency),
            top_up_credits,
            customer: Some(
                ServerCustomerWrapper::try_from(customer)
                    .map_err(Into::<PortalCheckoutApiError>::into)?
                    .0,
            ),
            payment_methods: resolved
                .filter_payment_methods(methods)
                .into_iter()
                .filter(|method| Some(method.connection_id) == resolved.card_connection_id)
                .map(crate::customers::mapping::customer_payment_method::domain_to_server)
                .collect(),
            total_amount: charge,
            subtotal_amount: net,
            tax_amount: tax,
            amount_due: charge,
            card_connection_id: resolved.card_connection_id.map(|id| id.as_proto()),
            card_capabilities,
            trade_name: organization.trade_name,
            logo_url,
            brand_color: branding.brand_color,
            theme_mode: branding.theme_mode,
            roundness: branding.roundness,
            show_powered_by,
            require_billing_information: entity.require_billing_information,
            ..Default::default()
        })
    }

    /// Builds the Checkout response from subscription details and invoice content.
    async fn build_checkout_response(
        &self,
        tenant: TenantId,
        subscription_details: SubscriptionDetails,
        invoice_content: meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent,
    ) -> Result<Checkout, Status> {
        let customer = &subscription_details.customer;

        let invoicing_entity = &subscription_details.invoicing_entity;
        // The customer's entity, as checkout completion checks it.
        let require_billing_information = if invoicing_entity.id == customer.invoicing_entity_id {
            invoicing_entity.require_billing_information
        } else {
            self.store
                .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
                .require_billing_information
        };

        let branding =
            crate::portal::branding::resolve_portal_branding(&self.store, tenant, invoicing_entity)
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?;

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let show_powered_by = !self
            .feature_resolver
            .resolve(organization.id)
            .await
            .is_enabled(meteroid_app::features::gates::WHITELABEL);

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
        let card_capabilities = self.rail_capabilities(tenant, card_connection_id).await?;
        let direct_debit_capabilities = self
            .rail_capabilities(tenant, direct_debit_connection_id)
            .await?;

        // Fetch customer payment methods and filter to only those usable with resolved connections
        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let payment_methods = resolved
            .filter_payment_methods(customer_methods)
            .into_iter()
            .map(crate::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let bank_account = if let Some(bank_account_id) = resolved.bank_account_id {
            self.store
                .get_bank_account_by_id(bank_account_id, tenant)
                .await
                .ok()
                .map(crate::bankaccounts::mapping::bank_accounts::domain_to_proto)
        } else {
            None
        };

        let mut subscription_proto =
            crate::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription_details.clone(),
            )?;
        self.name_credit_units(tenant, &mut subscription_proto)
            .await?;

        let customer_proto = ServerCustomerWrapper::try_from(subscription_details.customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = match branding.logo_attachment_id {
            Some(id) => storage::signed_url(
                self.object_store.as_ref(),
                &self.store,
                id,
                Duration::from_secs(7 * 86400),
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?,
            None => None,
        };

        // Each line's share is in the figures it was quoted in, like the coupon amounts below.
        let promotional_credits: i64 = invoice_content
            .invoice_lines
            .iter()
            .filter_map(|line| line.promotional_credit)
            .sum();

        let invoice_lines = crate::invoices::mapping::invoices::domain_invoice_lines_to_server(
            invoice_content.invoice_lines,
        );

        let tax_breakdown = invoice_content
            .tax_breakdown
            .into_iter()
            .map(|item| TaxBreakdownItem {
                name: item.name,
                rate: item.tax_rate.to_string(),
                amount: item.tax_amount.to_non_negative_u64(),
            })
            .collect();

        let applied_coupons = invoice_content
            .applied_coupons
            .clone()
            .into_iter()
            .map(|coupon| AppliedCoupon {
                // What the coupon took off the prices the customer sees; on a tax-inclusive
                // checkout `value` is only the net share of it.
                amount: coupon.consumed_value().to_non_negative_u64(),
                coupon_code: coupon.code,
                coupon_name: coupon.name,
            })
            .collect();

        let coupon_amount: i64 = invoice_content
            .applied_coupons
            .iter()
            .map(|c| c.consumed_value())
            .sum();

        // Before the subscription starts, nothing has funded them yet: the summary says what will.
        let credit_coupons = if subscription_details.subscription.pending_checkout {
            subscription_details
                .applied_coupons
                .iter()
                .filter(|applied| applied.applied_coupon.is_active)
                .filter_map(|applied| match &applied.coupon.discount {
                    CouponDiscount::Credit {
                        currency,
                        expires_in_days,
                        ..
                    } => applied.coupon.discount.credit_minor().map(|amount| {
                        PromotionalCreditCoupon {
                            coupon_code: applied.coupon.code.clone(),
                            coupon_name: applied.coupon.description.clone(),
                            amount: amount.to_non_negative_u64(),
                            currency: currency.clone(),
                            expires_in_days: *expires_in_days,
                        }
                    }),
                    _ => None,
                })
                .collect()
        } else {
            vec![]
        };

        let next_invoice_minimum = subscription_details.minimum.as_ref().map(|minimum| {
            let discount = calculate_next_invoice_coupons_discount(
                minimum.amount_subunit,
                &subscription_details.subscription.currency,
                &subscription_details.applied_coupons,
                &invoice_content.applied_coupons,
            );
            (minimum.amount_subunit - discount).to_non_negative_u64()
        });

        Ok(Checkout {
            top_up_currency: None,
            top_up_credits: None,
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
            promotional_credits: promotional_credits.to_non_negative_u64(),
            tax_breakdown,
            applied_coupons,
            card_connection_id: card_connection_id.map(|id| id.as_proto()),
            direct_debit_connection_id: direct_debit_connection_id.map(|id| id.as_proto()),
            card_capabilities,
            direct_debit_capabilities,
            bank_account,
            brand_color: branding.brand_color,
            theme_mode: branding.theme_mode,
            roundness: branding.roundness,
            next_invoice_minimum,
            credit_coupons,
            show_powered_by,
            require_billing_information,
        })
    }

    /// Validates a coupon code, with the checks the payment runs, and creates a preview
    /// AppliedCouponDetailed for invoice computation. Nothing is persisted.
    async fn validate_and_create_preview_coupon(
        &self,
        code: &str,
        tenant_id: TenantId,
        subscription: &SubscriptionDetails,
    ) -> Result<AppliedCouponDetailed, Status> {
        self.services
            .preview_checkout_coupon(tenant_id, code, &subscription.subscription)
            .await
            .map_err(|err| PortalCheckoutApiError::from(err).into())
    }
}

/// The per-checkout-type first-invoice computation shared by `get_checkout`
/// and `initiate_hosted_checkout`.
struct CheckoutInvoiceComputation {
    subscription_details: SubscriptionDetails,
    invoice_content: meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent,
    checkout_type: CheckoutType,
    plan_change_context: Option<PlanChangeCheckoutContext>,
    addon_purchase_context: Option<AddOnPurchaseCheckoutContext>,
}

/// Appends `checkout_session_id` to a merchant-set success URL so the landing page can look the
/// outcome up. The URL was validated as absolute http(s) at session creation.
fn with_checkout_session_id(success_url: &str, session_id: CheckoutSessionId) -> String {
    match url::Url::parse(success_url) {
        Ok(mut u) => {
            u.query_pairs_mut()
                .append_pair("checkout_session_id", &session_id.as_base62());
            u.to_string()
        }
        Err(_) => success_url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::with_checkout_session_id;
    use common_domain::ids::{BaseId, CheckoutSessionId};

    #[test]
    fn success_url_gets_session_id_appended() {
        let id = CheckoutSessionId::new();
        let b62 = id.as_base62();
        assert_eq!(
            with_checkout_session_id("https://acme.com/done", id),
            format!("https://acme.com/done?checkout_session_id={b62}")
        );
        assert_eq!(
            with_checkout_session_id("https://acme.com/done?x=1#frag", id),
            format!("https://acme.com/done?x=1&checkout_session_id={b62}#frag")
        );
    }
}

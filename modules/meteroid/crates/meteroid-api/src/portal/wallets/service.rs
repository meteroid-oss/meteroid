use super::PortalWalletsServiceComponents;
use super::mapping;
use crate::wallets::{WalletApiError, grant_product_names};
use common_domain::ids::{CustomerId, CustomerPaymentMethodId, SubscriptionId, TenantId, WalletId};
use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use meteroid_app::features::ensure_portal_self_service;
use meteroid_grpc::meteroid::api::invoices::v1::PaymentActionRequired;
use meteroid_grpc::meteroid::portal::wallets::v1::{
    GetBalancesRequest, GetBalancesResponse, GetCreditOverviewRequest, GetCreditOverviewResponse,
    GetTopUpRequest, GetTopUpResponse, ListLedgerRequest, ListLedgerResponse, QuoteTopUpRequest,
    QuoteTopUpResponse, RedeemCodeRequest, RedeemCodeResponse, RedeemableSubscription,
    RedeemedCredit, TopUpStatus, TopUpWalletRequest, TopUpWalletResponse, Wallet,
    portal_wallets_service_server::PortalWalletsService, redeem_code_response,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_grpc_mapping::wallets::portal::{portal_error, quote_to_server};
use meteroid_store::domain::coupons::SubscriptionRequired;
use meteroid_store::domain::credit::{
    CreditWallet, PortalCreditCapabilities, SelfServiceTopUpState, TopUpWallet, TopUpWalletQuote,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use tonic::{Request, Response, Status};

fn portal_customer<T>(request: &Request<T>) -> Result<(TenantId, CustomerId), Status> {
    let resource = request.portal_resource()?;
    match resource.resource_access {
        ResourceAccess::CustomerPortal(customer_id) => Ok((resource.tenant_id, customer_id)),
        _ => Err(Status::permission_denied(
            "wallets are only available in the customer portal",
        )),
    }
}

impl PortalWalletsServiceComponents {
    async fn wallet_views(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        capabilities: &PortalCreditCapabilities,
    ) -> Result<Vec<Wallet>, Status> {
        let wallets = self
            .services
            .customer_credit_wallets_with_auto_top_up(tenant_id, customer_id)
            .await
            .map_err(portal_error)?;
        let health = {
            let mut conn = self.store.get_conn().await.map_err(portal_error)?;
            self.store
                .hooks
                .credit
                .live_wallets(&mut conn, tenant_id, customer_id, &wallets)
                .await
                .map_err(portal_error)?
                .health
        };
        let sales = if wallets
            .iter()
            .any(|(wallet, _)| wallet.credit_unit.is_some())
        {
            self.services
                .sellable_unit_prices(tenant_id, customer_id)
                .await
                .map_err(portal_error)?
        } else {
            Default::default()
        };
        let product_names = grant_product_names(
            &self.store,
            tenant_id,
            wallets.iter().map(|(wallet, _)| wallet),
        )
        .await?;
        let plans = self
            .store
            .credit_grant_plans(
                tenant_id,
                wallets.iter().flat_map(|(wallet, _)| wallet.grants.iter()),
            )
            .await
            .map_err(portal_error)?;
        wallets
            .iter()
            .zip(&health)
            .map(|((wallet, stored), health)| {
                mapping::wallet(
                    wallet,
                    stored.as_ref(),
                    health,
                    capabilities.top_up_enabled,
                    sales.get(&wallet.id),
                    &product_names,
                    &plans,
                )
            })
            .collect()
    }

    async fn capabilities(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<PortalCreditCapabilities, Status> {
        self.services
            .portal_credit_capabilities(tenant_id, customer_id)
            .await
            .map_err(portal_error)
    }

    /// Whether the merchant's plan lets automatic top-ups be turned on or changed.
    async fn auto_top_up_configurable(&self, tenant_id: TenantId) -> Result<bool, Status> {
        meteroid_app::features::tenant_gate_open(
            self.feature_resolver.as_ref(),
            &self.store,
            tenant_id,
            meteroid_app::features::gates::AUTO_TOP_UPS,
        )
        .await
        .map_err(portal_error)
    }

    async fn customer_currency(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<String, Status> {
        Ok(self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await
            .map_err(portal_error)?
            .currency)
    }

    async fn ensure_top_up_offered(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<(), Status> {
        if self
            .capabilities(tenant_id, customer_id)
            .await?
            .top_up_enabled
        {
            return Ok(());
        }
        Err(Status::failed_precondition(
            "Credit can't be bought from this portal. Contact the merchant to add credit.",
        ))
    }

    /// The wallet a top-up buys into; none named is the customer's money wallet in their billing
    /// currency, opened on first use once the portal may sell credit.
    async fn top_up_target(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: &str,
    ) -> Result<CreditWallet, Status> {
        if !wallet_id.trim().is_empty() {
            return self.own_wallet(tenant_id, customer_id, wallet_id).await;
        }
        self.ensure_top_up_offered(tenant_id, customer_id).await?;
        let currency = self.customer_currency(tenant_id, customer_id).await?;
        self.store
            .create_customer_credit_wallet(tenant_id, customer_id, &currency)
            .await
            .map_err(portal_error)
    }

    /// Another customer's wallet is reported as missing, never as forbidden.
    async fn own_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: &str,
    ) -> Result<CreditWallet, Status> {
        let wallet_id = WalletId::from_proto(wallet_id)?;
        self.store
            .customer_credit_wallet(tenant_id, wallet_id)
            .await
            .map_err(portal_error)?
            .filter(|wallet| wallet.customer_id == customer_id)
            .ok_or_else(|| Status::not_found("wallet not found"))
    }
}

#[tonic::async_trait]
impl PortalWalletsService for PortalWalletsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_balances(
        &self,
        request: Request<GetBalancesRequest>,
    ) -> Result<Response<GetBalancesResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        let capabilities = self.capabilities(tenant_id, customer_id).await?;
        let wallets = self
            .wallet_views(tenant_id, customer_id, &capabilities)
            .await?;
        Ok(Response::new(GetBalancesResponse {
            wallets,
            top_up_enabled: capabilities.top_up_enabled,
            auto_top_up_enabled: capabilities.auto_top_up_enabled,
            customer_currency: self.customer_currency(tenant_id, customer_id).await?,
            auto_top_up_configurable: self.auto_top_up_configurable(tenant_id).await?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_ledger(
        &self,
        request: Request<ListLedgerRequest>,
    ) -> Result<Response<ListLedgerResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        let req = request.into_inner();
        let wallet = self
            .own_wallet(tenant_id, customer_id, &req.wallet_id)
            .await?;
        let pagination = req.pagination.into_domain();
        let per_page = pagination.per_page.unwrap_or(20).clamp(1, 100);
        let (entries, total) = self
            .store
            .customer_credit_ledger(
                tenant_id,
                wallet.id,
                i64::from(per_page),
                i64::from(pagination.page) * i64::from(per_page),
            )
            .await
            .map_err(portal_error)?;
        let total_pages = u32::try_from(total.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX);
        let invoice_ids = entries
            .iter()
            .filter_map(|entry| entry.invoice_id)
            .collect::<Vec<_>>();
        let invoice_numbers = self
            .services
            .portal_invoice_numbers(tenant_id, customer_id, &invoice_ids)
            .await
            .map_err(portal_error)?;
        let automatic_top_up_grants = self
            .services
            .portal_automatic_top_up_grants(tenant_id, customer_id)
            .await
            .map_err(portal_error)?;
        Ok(Response::new(ListLedgerResponse {
            entries: entries
                .iter()
                .map(|entry| {
                    mapping::ledger_entry(
                        entry,
                        &wallet,
                        &automatic_top_up_grants,
                        &invoice_numbers,
                    )
                })
                .collect(),
            pagination_meta: req.pagination.into_response(total_pages, total),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_credit_overview(
        &self,
        request: Request<GetCreditOverviewRequest>,
    ) -> Result<Response<GetCreditOverviewResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        let mut overview = self
            .services
            .portal_credit_overview(tenant_id, customer_id)
            .await
            .map_err(portal_error)?;
        let sales = if overview
            .wallets
            .iter()
            .any(|entry| entry.wallet.credit_unit.is_some())
        {
            self.services
                .sellable_unit_prices(tenant_id, customer_id)
                .await
                .map_err(portal_error)?
        } else {
            Default::default()
        };
        let product_names = grant_product_names(
            &self.store,
            tenant_id,
            overview.wallets.iter().map(|entry| &entry.wallet),
        )
        .await?;
        let plans = self
            .store
            .credit_grant_plans(
                tenant_id,
                overview
                    .wallets
                    .iter()
                    .flat_map(|entry| entry.wallet.grants.iter()),
            )
            .await
            .map_err(portal_error)?;
        let mut upcoming_invoices = Vec::with_capacity(overview.upcoming.len());
        for upcoming in std::mem::take(&mut overview.upcoming) {
            upcoming_invoices.push(
                crate::portal::upcoming::upcoming_invoice_view(
                    &self.store,
                    &upcoming.details,
                    upcoming.invoice_date,
                    upcoming.content,
                    upcoming.credit_estimate_shared,
                    &upcoming.billed_first,
                )
                .await?,
            );
        }
        Ok(Response::new(GetCreditOverviewResponse {
            wallets: overview
                .wallets
                .iter()
                .map(|entry| {
                    mapping::wallet_overview(
                        entry,
                        overview.capabilities.top_up_enabled,
                        sales.get(&entry.wallet.id),
                        &product_names,
                        &plans,
                        &overview.outlooks,
                    )
                })
                .collect::<Result<_, _>>()?,
            top_up_enabled: overview.capabilities.top_up_enabled,
            auto_top_up_enabled: overview.capabilities.auto_top_up_enabled,
            credit_funded: overview.credit_funded,
            upcoming_invoices,
            unavailable_upcoming_subscription_ids: overview
                .upcoming_unavailable
                .iter()
                .map(|id| id.as_proto())
                .collect(),
            customer_currency: self.customer_currency(tenant_id, customer_id).await?,
            auto_top_up_configurable: self.auto_top_up_configurable(tenant_id).await?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn quote_top_up(
        &self,
        request: Request<QuoteTopUpRequest>,
    ) -> Result<Response<QuoteTopUpResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        let req = request.into_inner();
        // Without a wallet the quote opens none: only paying does.
        let quote = if req.wallet_id.trim().is_empty() {
            self.ensure_top_up_offered(tenant_id, customer_id).await?;
            self.services
                .quote_money_wallet_top_up(tenant_id, customer_id, req.amount)
                .await
                .map_err(portal_error)?
        } else {
            let wallet = self
                .own_wallet(tenant_id, customer_id, &req.wallet_id)
                .await?;
            self.services
                .ensure_self_service_top_up(tenant_id, customer_id, wallet.id)
                .await
                .map_err(portal_error)?;
            self.services
                .quote_wallet_top_up(tenant_id, customer_id, wallet.id, req.amount)
                .await
                .map_err(portal_error)?
        };
        Ok(Response::new(QuoteTopUpResponse {
            quote: Some(quote_to_server(quote)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn top_up_wallet(
        &self,
        request: Request<TopUpWalletRequest>,
    ) -> Result<Response<TopUpWalletResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        ensure_portal_self_service(self.feature_resolver.as_ref(), &self.store, tenant_id).await?;
        let req = request.into_inner();
        let wallet = self
            .top_up_target(tenant_id, customer_id, &req.wallet_id)
            .await?;
        self.services
            .ensure_self_service_top_up(tenant_id, customer_id, wallet.id)
            .await
            .map_err(portal_error)?;
        let expected = req.expected_quote.map(|quote| TopUpWalletQuote {
            charge_minor: quote.charge,
            currency: quote.currency,
            net_minor: quote.net,
            tax_minor: quote.tax,
        });
        if req.payment_method_id.is_empty() {
            let session = self
                .services
                .create_portal_top_up_checkout(
                    tenant_id,
                    customer_id,
                    wallet.id,
                    req.amount,
                    req.idempotency_key,
                    expected,
                    req.return_url.filter(|url| !url.trim().is_empty()),
                )
                .await
                .map_err(portal_error)?;
            let charge_currency = session.top_up().map_err(portal_error)?.currency;
            let checkout_url = self
                .services
                .top_up_checkout_url(tenant_id, session.id)
                .await
                .map_err(portal_error)?;
            return Ok(Response::new(TopUpWalletResponse {
                status: TopUpStatus::RequiresAction.into(),
                currency: charge_currency,
                checkout_url: Some(checkout_url),
                ..Default::default()
            }));
        }
        let receipt = self
            .services
            .top_up_wallet(
                TopUpWallet {
                    tenant_id,
                    customer_id,
                    wallet_id: wallet.id,
                    amount_minor: req.amount,
                    payment_method_id: CustomerPaymentMethodId::from_proto(&req.payment_method_id)?,
                    idempotency_key: req.idempotency_key,
                    expires_at: None,
                    unit_price: None,
                    expected_unit_price: None,
                },
                expected,
            )
            .await
            .map_err(portal_error)?;
        let status = match crate::wallets::top_up_status(&receipt) {
            meteroid_grpc::meteroid::api::wallets::v1::TopUpStatus::Succeeded => {
                TopUpStatus::Succeeded
            }
            meteroid_grpc::meteroid::api::wallets::v1::TopUpStatus::RequiresAction => {
                TopUpStatus::RequiresAction
            }
            _ => TopUpStatus::Pending,
        };
        let checkout_url = if status == TopUpStatus::RequiresAction {
            let id = receipt
                .transaction
                .checkout_session_id
                .ok_or_else(|| Status::internal("top-up checkout is missing"))?;
            Some(
                self.services
                    .top_up_checkout_url(tenant_id, id)
                    .await
                    .map_err(portal_error)?,
            )
        } else {
            None
        };
        let next_action: Option<PaymentActionRequired> = receipt
            .next_action
            .map(crate::invoices::mapping::payment_action::domain_to_server);
        let mut response = Response::new(TopUpWalletResponse {
            status: status.into(),
            checkout_url,
            charged_amount: receipt.charge_minor,
            currency: receipt.currency,
            invoice_id: receipt.invoice_id.as_proto(),
            next_action,
        });
        // The next action may carry a client secret.
        response.metadata_mut().insert(
            "cache-control",
            tonic::metadata::MetadataValue::from_static("no-store"),
        );
        Ok(response)
    }

    #[tracing::instrument(skip_all)]
    async fn get_top_up(
        &self,
        request: Request<GetTopUpRequest>,
    ) -> Result<Response<GetTopUpResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        let req = request.into_inner();
        let wallet = self
            .own_wallet(tenant_id, customer_id, &req.wallet_id)
            .await?;
        let state = self
            .services
            .self_service_top_up_state(tenant_id, customer_id, wallet.id, req.idempotency_key)
            .await
            .map_err(portal_error)?;
        let (status, failure_reason) = match state {
            None => (None, None),
            Some(SelfServiceTopUpState::Succeeded) => (Some(TopUpStatus::Succeeded), None),
            Some(SelfServiceTopUpState::Pending { requires_action }) => (
                Some(if requires_action {
                    TopUpStatus::RequiresAction
                } else {
                    TopUpStatus::Pending
                }),
                None,
            ),
            Some(SelfServiceTopUpState::Failed { reason }) => {
                (Some(TopUpStatus::Failed), Some(reason))
            }
        };
        Ok(Response::new(GetTopUpResponse {
            status: status.map(Into::into),
            failure_reason,
        }))
    }

    #[tracing::instrument(skip_all, fields(
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn redeem_code(
        &self,
        request: Request<RedeemCodeRequest>,
    ) -> Result<Response<RedeemCodeResponse>, Status> {
        let (tenant_id, customer_id) = portal_customer(&request)?;
        ensure_portal_self_service(self.feature_resolver.as_ref(), &self.store, tenant_id).await?;
        let code_try = self.promo_codes.try_code(tenant_id, customer_id).await?;
        let req = request.into_inner();
        let subscription_id = SubscriptionId::from_proto_opt(req.subscription_id)?;
        let redeemed = self
            .services
            .redeem_coupon_credit_code(tenant_id, customer_id, &req.code, subscription_id)
            .await;
        // A code that asks which subscription to credit is a real code too.
        let usable = match &redeemed {
            Ok(_) => true,
            Err(error) => error
                .frames()
                .any(|frame| frame.downcast_ref::<SubscriptionRequired>().is_some()),
        };
        self.promo_codes
            .settle(tenant_id, customer_id, code_try, usable)
            .await;
        let outcome = match redeemed {
            Ok(redemption) => redeem_code_response::Outcome::Redeemed(RedeemedCredit {
                amount: redemption.amount_minor,
                currency: redemption.currency,
                expires_at: redemption
                    .expires_at
                    .map(crate::wallets::mapping::timestamp),
            }),
            Err(error) => {
                let Some(SubscriptionRequired(choices)) = error
                    .frames()
                    .find_map(|frame| frame.downcast_ref::<SubscriptionRequired>())
                    .cloned()
                else {
                    return Err(WalletApiError::from(error).into());
                };
                redeem_code_response::Outcome::SubscriptionRequired(
                    meteroid_grpc::meteroid::portal::wallets::v1::SubscriptionRequired {
                        subscriptions: choices
                            .into_iter()
                            .map(|choice| RedeemableSubscription {
                                id: choice.id.as_proto(),
                                plan_name: choice.plan_name,
                                start_date: choice.start_date.to_string(),
                            })
                            .collect(),
                    },
                )
            }
        };
        Ok(Response::new(RedeemCodeResponse {
            outcome: Some(outcome),
        }))
    }
}

use super::ledger_export::{self, LedgerScope};
use super::{WalletApiError, WalletServiceComponents};
use super::{activity, mapping};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use common_domain::actor::{Actor, ActorType};
use common_domain::ids::{
    BaseId, CreditGrantId, CustomerId, CustomerPaymentMethodId, ProductId, SubscriptionId,
    TenantId, WalletId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::{SCOPED_CREDIT_NOT_IN_PLAN, ensure_change_allowed, gates};
use meteroid_grpc::meteroid::api::wallets::v1::{
    ConfigureWalletRequest, ConfigureWalletResponse, CreateWalletRequest, CreateWalletResponse,
    CreditAppliesTo, CreditGrantType, ExportCustomerLedgerRequest, ExportCustomerLedgerResponse,
    GetCustomerCreditActivityRequest, GetCustomerCreditActivityResponse, GrantCreditRequest,
    GrantCreditResponse, ListCustomerLedgerEntriesRequest, ListCustomerLedgerEntriesResponse,
    ListCustomerWalletsRequest, ListCustomerWalletsResponse, ListLedgerEntriesRequest,
    ListLedgerEntriesResponse, ListWalletDailyUsageCostRequest, ListWalletDailyUsageCostResponse,
    ListWalletGrantsRequest, ListWalletGrantsResponse, ListWalletSummariesRequest,
    ListWalletSummariesResponse, QuoteCreditSaleRequest, QuoteCreditSaleResponse,
    RemoveCreditRequest, RemoveCreditResponse, SellCreditRequest, SellCreditResponse, TopUpStatus,
    TopUpWalletRequest, TopUpWalletResponse, UnitPriceSource as ProtoUnitPriceSource,
    UsageCostPeriod, Wallet, WalletDailyUsageCost,
    configure_wallet_request::AutoTopUpChange as AutoTopUpChangeProto,
    wallets_service_server::WalletsService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::Store;
use meteroid_store::domain::credit::{
    AutoTopUpChange, ConfigureCreditWallet, CreditApplication, CreditGrant, CreditWallet,
    FundingReference, GrantClassification, GrantScope, GrantSubscriptionFilter, TopUpWallet,
    UnitPriceChange,
};
use meteroid_store::domain::enums::PaymentStatusEnum;
use meteroid_store::domain::price_components::CreditExpiry;
use meteroid_store::domain::usage_costs::WalletUsageSpan;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::{
    DebitCustomerCreditGrant, FundCustomerCreditWallet,
};
use meteroid_store::repositories::credit::{
    CustomerCreditActivityInterface, CustomerCreditWalletsInterface,
};
use meteroid_store::repositories::entity_activity::EntityActivityInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::services::{SellCustomerCredit, TopUpWalletReceipt, UnitPriceSource};
use std::collections::{HashMap, HashSet};
use tonic::{Request, Response, Status};
use uuid::Uuid;

fn idempotency_key(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value.trim())
        .map_err(|_| Status::invalid_argument("the idempotency key must be a UUID"))
}

/// An operator's negotiated price of one credit, as a decimal string.
fn unit_price(value: Option<&str>) -> Result<Option<rust_decimal::Decimal>, Status> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse()
                .map_err(|_| Status::invalid_argument("the unit price must be a decimal number"))
        })
        .transpose()
}

/// The same bound as a credit sale's `expires_in_months`.
const MAX_EXPIRY_MONTHS: u32 = 120;

/// The expiry an operator sets as a timestamp or as months from `now`, never both.
fn credit_expiry(
    expires_at: Option<prost_types::Timestamp>,
    expires_in_months: Option<u32>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, Status> {
    match (expires_at, expires_in_months) {
        (Some(_), Some(_)) => Err(Status::invalid_argument(
            "choose absolute or relative credit expiry",
        )),
        (Some(at), None) => mapping::from_timestamp(at).map(Some),
        (None, Some(months)) => months_after(now, months).map(Some),
        (None, None) => Ok(None),
    }
}

/// `months` after `from` in UTC, a day past the target month's end clamped to its last day, as a
/// sold pack's expiry resolves.
fn months_after(from: DateTime<Utc>, months: u32) -> Result<DateTime<Utc>, Status> {
    if !(1..=MAX_EXPIRY_MONTHS).contains(&months) {
        return Err(Status::invalid_argument(format!(
            "credit expires between 1 and {MAX_EXPIRY_MONTHS} months from now"
        )));
    }
    CreditExpiry::Month(months)
        .resolve(from)
        .ok_or_else(|| Status::invalid_argument("the credit expiry is out of range"))
}

/// The names of every product the wallets' grants are restricted to, in one lookup.
pub(crate) async fn grant_product_names<'a>(
    store: &Store,
    tenant_id: TenantId,
    wallets: impl IntoIterator<Item = &'a CreditWallet>,
) -> Result<HashMap<ProductId, String>, Status> {
    let ids = grant_product_ids(wallets.into_iter().flat_map(|wallet| wallet.grants.iter()));
    product_names(store, tenant_id, ids).await
}

// Collected before any await: a borrowing iterator held across one makes the future not `Send`.
fn grant_product_ids<'a>(grants: impl IntoIterator<Item = &'a CreditGrant>) -> Vec<ProductId> {
    grants
        .into_iter()
        .flat_map(|grant| grant.scope.product_ids.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

async fn product_names(
    store: &Store,
    tenant_id: TenantId,
    ids: Vec<ProductId>,
) -> Result<HashMap<ProductId, String>, Status> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(store
        .find_products_by_ids(&ids, tenant_id)
        .await
        .map_err(Into::<WalletApiError>::into)?
        .into_iter()
        .map(|product| (product.id, product.name))
        .collect())
}

impl WalletServiceComponents {
    async fn customer_wallet_views(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> Result<Vec<Wallet>, Status> {
        let wallets = self
            .services
            .customer_credit_wallets_with_auto_top_up(tenant_id, customer_id)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let controls = &self.store.hooks.credit;
        let live = {
            let mut conn = self
                .store
                .get_conn()
                .await
                .map_err(Into::<WalletApiError>::into)?;
            controls
                .live_wallets(&mut conn, tenant_id, customer_id, &wallets)
                .await
                .map_err(Into::<WalletApiError>::into)?
        };
        let product_names = grant_product_names(
            &self.store,
            tenant_id,
            wallets.iter().map(|(wallet, _)| wallet),
        )
        .await?;
        let month_spend = controls
            .automatic_top_up_month_spend(&self.services, tenant_id, customer_id, &wallets)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let plans = self
            .store
            .credit_grant_plans(
                tenant_id,
                wallets.iter().flat_map(|(wallet, _)| wallet.grants.iter()),
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let period_start = self
            .store
            .customer_current_period_start(tenant_id, customer_id)
            .await
            .map_err(Into::<WalletApiError>::into)?
            .map(|day| day.and_time(NaiveTime::MIN).and_utc());
        let attesters: Vec<(ActorType, Uuid)> = wallets
            .iter()
            .filter_map(|(_, stored)| stored.as_ref()?.consent_attestation.as_ref())
            .filter_map(|attestation| {
                let actor = &attestation.attested_by;
                Some((actor.actor_type(), actor.as_uuid()?))
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let attester_names = if attesters.is_empty() {
            HashMap::new()
        } else {
            self.store
                .resolve_actor_names(tenant_id, &attesters)
                .await
                .map_err(Into::<WalletApiError>::into)?
        };
        wallets
            .iter()
            .zip(&live.health)
            .map(|((wallet, stored), health)| {
                mapping::wallet(
                    wallet,
                    stored.as_ref(),
                    month_spend.get(&wallet.id),
                    health,
                    live.usage_paused_at.get(&wallet.id).copied(),
                    &product_names,
                    &plans,
                    &attester_names,
                    period_start,
                )
            })
            .collect()
    }

    /// The wallet as it now stands; its status depends on the customer's other wallets.
    async fn wallet_view(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet: &CreditWallet,
    ) -> Result<Wallet, Status> {
        let id = wallet.id.as_proto();
        self.customer_wallet_views(tenant_id, customer_id)
            .await?
            .into_iter()
            .find(|view| view.id == id)
            .ok_or_else(|| Status::not_found("wallet not found"))
    }

    async fn customer_wallet(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
    ) -> Result<Wallet, Status> {
        let wallet = self
            .store
            .customer_credit_wallet(tenant_id, wallet_id)
            .await
            .map_err(Into::<WalletApiError>::into)?
            .filter(|wallet| wallet.customer_id == customer_id)
            .ok_or_else(|| Status::not_found("wallet not found"))?;
        self.wallet_view(tenant_id, customer_id, &wallet).await
    }
}

pub(crate) fn top_up_status(receipt: &TopUpWalletReceipt) -> TopUpStatus {
    if receipt.transaction.status == PaymentStatusEnum::Settled {
        TopUpStatus::Succeeded
    } else if receipt.next_action.is_some() {
        TopUpStatus::RequiresAction
    } else {
        TopUpStatus::Pending
    }
}

#[tonic::async_trait]
impl WalletsService for WalletServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_customer_wallets(
        &self,
        request: Request<ListCustomerWalletsRequest>,
    ) -> Result<Response<ListCustomerWalletsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = CustomerId::from_proto(&request.into_inner().customer_id)?;
        let wallets = self.customer_wallet_views(tenant_id, customer_id).await?;
        Ok(Response::new(ListCustomerWalletsResponse { wallets }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_ledger_entries(
        &self,
        request: Request<ListLedgerEntriesRequest>,
    ) -> Result<Response<ListLedgerEntriesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let wallet_id = WalletId::from_proto(&req.wallet_id)?;
        let pagination = req.pagination.into_domain();
        let per_page = pagination.per_page.unwrap_or(20).clamp(1, 100);
        let (entries, total) = self
            .store
            .customer_credit_ledger(
                tenant_id,
                wallet_id,
                i64::from(per_page),
                i64::from(pagination.page) * i64::from(per_page),
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let total_pages = u32::try_from(total.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX);
        Ok(Response::new(ListLedgerEntriesResponse {
            entries: entries.iter().map(mapping::ledger_entry).collect(),
            pagination_meta: req.pagination.into_response(total_pages, total),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn list_wallet_grants(
        &self,
        request: Request<ListWalletGrantsRequest>,
    ) -> Result<Response<ListWalletGrantsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let subscription = SubscriptionId::from_proto_opt(req.subscription_id.clone())?
            .map(GrantSubscriptionFilter::UsableBy);
        let pagination = req.pagination.into_domain();
        let per_page = pagination.per_page.unwrap_or(20).clamp(1, 100);
        let page = self
            .store
            .customer_wallet_grants(
                tenant_id,
                customer_id,
                WalletId::from_proto(&req.wallet_id)?,
                subscription,
                req.used_up,
                i64::from(per_page),
                i64::from(pagination.page) * i64::from(per_page),
            )
            .await
            .map_err(Into::<WalletApiError>::into)?
            .ok_or_else(|| Status::not_found("wallet not found"))?;
        let product_names =
            product_names(&self.store, tenant_id, grant_product_ids(&page.grants)).await?;
        let plans = self
            .store
            .credit_grant_plans(tenant_id, &page.grants)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let consumed = self
            .store
            .credit_grants_activity(tenant_id, customer_id, &page.grants)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let total_pages =
            u32::try_from(page.total.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX);
        Ok(Response::new(ListWalletGrantsResponse {
            grants: page
                .grants
                .iter()
                .map(|grant| mapping::grant(grant, false, page.credit_unit, &product_names, &plans))
                .collect::<Result<Vec<_>, _>>()?,
            activity: consumed.iter().map(activity::grant_activity).collect(),
            pagination_meta: req.pagination.into_response(total_pages, page.total),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_wallet_summaries(
        &self,
        request: Request<ListWalletSummariesRequest>,
    ) -> Result<Response<ListWalletSummariesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let pagination = req.pagination.into_domain();
        let per_page = pagination.per_page.unwrap_or(20).clamp(1, 100);
        let (wallets, total) = self
            .services
            .list_credit_wallet_summaries(
                tenant_id,
                req.search.as_deref(),
                req.status
                    .and_then(|_| mapping::wallet_status_from_server(req.status())),
                req.auto_top_up_failing,
                i64::from(per_page),
                i64::from(pagination.page) * i64::from(per_page),
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let total_pages = u32::try_from(total.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX);
        Ok(Response::new(ListWalletSummariesResponse {
            wallets: wallets.iter().map(mapping::wallet_summary).collect(),
            pagination_meta: req.pagination.into_response(total_pages, total),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn create_wallet(
        &self,
        request: Request<CreateWalletRequest>,
    ) -> Result<Response<CreateWalletResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let wallet = self
            .store
            .create_customer_credit_wallet(tenant_id, customer_id, &req.currency)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(CreateWalletResponse {
            wallet: Some(self.wallet_view(tenant_id, customer_id, &wallet).await?),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn grant_credit(
        &self,
        request: Request<GrantCreditRequest>,
    ) -> Result<Response<GrantCreditResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let wallet_id = WalletId::from_proto(&req.wallet_id)?;
        let classification = match req.grant_type() {
            CreditGrantType::Promotional => GrantClassification::Promotional,
            CreditGrantType::Paid => GrantClassification::Paid,
            CreditGrantType::Unspecified => {
                return Err(Status::invalid_argument(
                    "say whether the credit is promotional or paid outside Meteroid",
                ));
            }
        };
        if classification == GrantClassification::Paid {
            let wallet = self
                .store
                .customer_credit_wallet(tenant_id, wallet_id)
                .await
                .map_err(Into::<WalletApiError>::into)?
                .filter(|wallet| wallet.customer_id == customer_id)
                .ok_or_else(|| Status::not_found("wallet not found"))?;
            if wallet.credit_unit.is_some() {
                return Err(Status::invalid_argument(
                    "credit paid outside Meteroid is recorded in a currency wallet; sell credits of a unit instead",
                ));
            }
        }
        let applies_to = req.applies_to();
        let reference = req
            .reference
            .filter(|reference| !reference.trim().is_empty())
            .map(FundingReference::new)
            .transpose()
            .map_err(|error| Status::from(WalletApiError::from(error)))?;
        let applicable_product_ids = req
            .applicable_product_ids
            .iter()
            .map(ProductId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;
        let scope = GrantScope {
            subscription_id: SubscriptionId::from_proto_opt(req.subscription_id.clone())?,
            application: match applies_to {
                CreditAppliesTo::Unspecified | CreditAppliesTo::All => CreditApplication::Invoice,
                CreditAppliesTo::Usage => CreditApplication::Usage,
            },
            product_ids: Vec::new(),
        }
        .with_products(applicable_product_ids);
        ensure_change_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            gates::SCOPED_CREDIT,
            scope.is_scoped(),
            SCOPED_CREDIT_NOT_IN_PLAN,
        )
        .await?;
        let command = FundCustomerCreditWallet {
            tenant_id,
            customer_id,
            wallet_id,
            amount_minor: req.amount,
            classification,
            priority: req.priority.unwrap_or(DEFAULT_PRIORITY),
            expires_at: credit_expiry(req.expires_at, req.expires_in_months, Utc::now())?,
            idempotency_key: idempotency_key(&req.idempotency_key)?,
            reason: req.reason,
            reference,
            actor_id: Some(actor.as_uuid()),
            cost: None,
        };
        self.services
            .ensure_supporting_document(&command)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let funded = self
            .store
            .fund_scoped_customer_credit_wallet(command, scope)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(GrantCreditResponse {
            wallet: Some(
                self.wallet_view(tenant_id, customer_id, &funded.wallet)
                    .await?,
            ),
            credit_grant_id: funded.grant_id.as_proto(),
            replayed: funded.replayed,
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn remove_credit(
        &self,
        request: Request<RemoveCreditRequest>,
    ) -> Result<Response<RemoveCreditResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let debited = self
            .store
            .debit_customer_credit_grant(DebitCustomerCreditGrant {
                tenant_id,
                customer_id,
                grant_id: CreditGrantId::from_proto(&req.credit_grant_id)?,
                amount_minor: req.amount,
                idempotency_key: idempotency_key(&req.idempotency_key)?,
                reason: req.reason,
                actor_id: Some(actor.as_uuid()),
            })
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(RemoveCreditResponse {
            wallet: Some(
                self.wallet_view(tenant_id, customer_id, &debited.wallet)
                    .await?,
            ),
            removed: debited.removed_minor,
            replayed: debited.replayed,
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn sell_credit(
        &self,
        request: Request<SellCreditRequest>,
    ) -> Result<Response<SellCreditResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let sold = self
            .services
            .sell_customer_credit(
                idempotency_key(&req.idempotency_key)?,
                SellCustomerCredit {
                    expires_at: None,
                    tenant_id,
                    customer_id,
                    wallet_id: WalletId::from_proto(&req.wallet_id)?,
                    amount_minor: req.amount,
                    expires_in_months: req.expires_in_months,
                    note: req.note.filter(|note| !note.trim().is_empty()),
                    actor,
                    unit_price: unit_price(req.unit_price.as_deref())?,
                    expected_unit_price: unit_price(req.expected_unit_price.as_deref())?,
                },
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(SellCreditResponse {
            wallet: Some(
                self.wallet_view(tenant_id, customer_id, &sold.wallet)
                    .await?,
            ),
            invoice_id: sold.invoice.invoice.id.as_proto(),
            credit_grant_id: sold.grant_id.map(|id| id.as_proto()),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn top_up_wallet(
        &self,
        request: Request<TopUpWalletRequest>,
    ) -> Result<Response<TopUpWalletResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let receipt = self
            .services
            .top_up_wallet(
                TopUpWallet {
                    tenant_id,
                    customer_id,
                    wallet_id: WalletId::from_proto(&req.wallet_id)?,
                    amount_minor: req.amount,
                    payment_method_id: CustomerPaymentMethodId::from_proto(&req.payment_method_id)?,
                    idempotency_key: req.idempotency_key,
                    expires_at: credit_expiry(req.expires_at, req.expires_in_months, Utc::now())?,
                    unit_price: unit_price(req.unit_price.as_deref())?,
                    expected_unit_price: unit_price(req.expected_unit_price.as_deref())?,
                },
                None,
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(TopUpWalletResponse {
            wallet: Some(
                self.customer_wallet(tenant_id, customer_id, receipt.wallet_id)
                    .await?,
            ),
            status: top_up_status(&receipt).into(),
            charged_amount: receipt.charge_minor,
            currency: receipt.currency,
            invoice_id: receipt.invoice_id.as_proto(),
            payment_transaction_id: receipt.transaction.id.as_proto(),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn quote_credit_sale(
        &self,
        request: Request<QuoteCreditSaleRequest>,
    ) -> Result<Response<QuoteCreditSaleResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let sale = self
            .services
            .quote_credit_sale(
                tenant_id,
                CustomerId::from_proto(&req.customer_id)?,
                WalletId::from_proto(&req.wallet_id)?,
                req.amount,
                unit_price(req.unit_price.as_deref())?,
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(QuoteCreditSaleResponse {
            net: sale.quote.net_minor,
            tax: sale.quote.tax_minor,
            total: sale.quote.charge_minor,
            currency: sale.quote.currency,
            list_unit_price: sale
                .list_unit_price
                .map(|price| price.normalize().to_string()),
            unit_price: sale.unit_price.map(|price| price.normalize().to_string()),
            unit_price_source: sale.unit_price_source.map(|source| {
                match source {
                    UnitPriceSource::List => ProtoUnitPriceSource::List,
                    UnitPriceSource::Customer => ProtoUnitPriceSource::Customer,
                    UnitPriceSource::Override => ProtoUnitPriceSource::Override,
                }
                .into()
            }),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn configure_wallet(
        &self,
        request: Request<ConfigureWalletRequest>,
    ) -> Result<Response<ConfigureWalletResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let organization_id = request.organization();
        let req = request.into_inner();
        let usage_at_zero = super::mapping::usage_at_zero_from_proto(req.usage_at_zero());
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let auto_top_up = match req.auto_top_up_change {
            None => AutoTopUpChange::Keep,
            Some(AutoTopUpChangeProto::RemoveAutoTopUp(true)) => AutoTopUpChange::Remove,
            Some(AutoTopUpChangeProto::RemoveAutoTopUp(false)) => {
                return Err(Status::invalid_argument(
                    "remove_auto_top_up must be true; leave the change unset to keep it",
                ));
            }
            Some(AutoTopUpChangeProto::AutoTopUp(config)) => {
                let config = mapping::auto_top_up_config(config)?;
                if req.auto_top_up_at_current_price {
                    AutoTopUpChange::SetAtCurrentPrice(config)
                } else {
                    AutoTopUpChange::Set(config)
                }
            }
        };
        // Without the module, core's own refusal of any change answers instead.
        let auto_top_up = if auto_top_up.sets_terms()
            && self.store.hooks.credit.available()
            && !self
                .feature_resolver
                .resolve(organization_id?)
                .await
                .is_enabled(gates::AUTO_TOP_UPS)
        {
            auto_top_up.resend_only()
        } else {
            auto_top_up
        };
        let (wallet, _) = self
            .services
            .configure_credit_wallet(ConfigureCreditWallet {
                tenant_id,
                customer_id,
                wallet_id: WalletId::from_proto(&req.wallet_id)?,
                auto_top_up,
                consent_attested_by: req
                    .customer_consent_attested
                    .then_some(Actor::User { id: actor }),
                low_balance_thresholds_minor: Some(req.low_balance_thresholds),
                usage_at_zero: Some(usage_at_zero),
                // Like every field of this request, an unset price clears it.
                unit_price: match unit_price(req.unit_price.as_deref())? {
                    Some(price) => UnitPriceChange::Set(price),
                    None => UnitPriceChange::Clear,
                },
                actor_id: Some(actor.as_uuid()),
            })
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(ConfigureWalletResponse {
            wallet: Some(self.wallet_view(tenant_id, customer_id, &wallet).await?),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn get_customer_credit_activity(
        &self,
        request: Request<GetCustomerCreditActivityRequest>,
    ) -> Result<Response<GetCustomerCreditActivityResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let since = req.since.map(mapping::from_timestamp).transpose()?;
        let subscription_id = SubscriptionId::from_proto_opt(req.subscription_id)?;
        let wallets = self
            .store
            .customer_credit_activity(tenant_id, customer_id, since, subscription_id)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(GetCustomerCreditActivityResponse {
            wallets: wallets.iter().map(activity::wallet_activity).collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn list_customer_ledger_entries(
        &self,
        request: Request<ListCustomerLedgerEntriesRequest>,
    ) -> Result<Response<ListCustomerLedgerEntriesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;
        let wallet_id = WalletId::from_proto_opt(req.wallet_id.clone())?;
        let subscription_id = SubscriptionId::from_proto_opt(req.subscription_id.clone())?;
        let pagination = req.pagination.into_domain();
        let per_page = pagination.per_page.unwrap_or(20).clamp(1, 100);
        let (entries, total) = self
            .store
            .customer_credit_labelled_ledger(
                tenant_id,
                customer_id,
                wallet_id,
                subscription_id,
                i64::from(per_page),
                i64::from(pagination.page) * i64::from(per_page),
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let total_pages = u32::try_from(total.div_ceil(u64::from(per_page))).unwrap_or(u32::MAX);
        Ok(Response::new(ListCustomerLedgerEntriesResponse {
            entries: entries
                .iter()
                .map(activity::labelled_ledger_entry)
                .collect(),
            pagination_meta: req.pagination.into_response(total_pages, total),
        }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn export_customer_ledger(
        &self,
        request: Request<ExportCustomerLedgerRequest>,
    ) -> Result<Response<ExportCustomerLedgerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let scope = LedgerScope {
            tenant_id,
            customer_id: CustomerId::from_proto(&req.customer_id)?,
            wallet_id: WalletId::from_proto_opt(req.wallet_id)?,
            subscription_id: SubscriptionId::from_proto_opt(req.subscription_id)?,
        };
        ledger_export::check_scope(&self.store, &scope)
            .await
            .map_err(Into::<WalletApiError>::into)?;
        let token = ledger_export::sign(scope, &self.jwt_secret)
            .map_err(|_| Status::internal("the export link could not be signed"))?;
        Ok(Response::new(ExportCustomerLedgerResponse { token }))
    }

    #[tracing::instrument(skip_all, fields(
        customer_id = %request.get_ref().customer_id,
        subscription_id = request.get_ref().subscription_id.as_deref(),
    ))]
    async fn list_wallet_daily_usage_cost(
        &self,
        request: Request<ListWalletDailyUsageCostRequest>,
    ) -> Result<Response<ListWalletDailyUsageCostResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let span = match req.period() {
            UsageCostPeriod::Current => WalletUsageSpan::CurrentPeriod,
            UsageCostPeriod::Previous => WalletUsageSpan::PreviousPeriod,
            UsageCostPeriod::Unspecified => WalletUsageSpan::Days {
                from: usage_day(req.from.as_deref(), "from")?,
                to: usage_day(req.to.as_deref(), "to")?,
            },
        };
        let usage = self
            .services
            .wallet_daily_usage_costs(
                tenant_id,
                CustomerId::from_proto(&req.customer_id)?,
                WalletId::from_proto(&req.wallet_id)?,
                SubscriptionId::from_proto_opt(req.subscription_id)?,
                span,
            )
            .await
            .map_err(Into::<WalletApiError>::into)?;
        Ok(Response::new(ListWalletDailyUsageCostResponse {
            from: usage.from.map(|day| day.to_string()),
            to: usage.to.map(|day| day.to_string()),
            rows: usage
                .rows
                .iter()
                .map(|row| WalletDailyUsageCost {
                    date: row.date.to_string(),
                    subscription_id: row.subscription_id.as_proto(),
                    metric_id: row.metric_id.as_proto(),
                    metric_name: row.metric_name.clone(),
                    product_id: row.product_id.map(|id| id.as_proto()),
                    amount: row.amount_minor,
                    quantity: row.quantity.normalize().to_string(),
                })
                .collect(),
        }))
    }
}

/// A day of a usage range, YYYY-MM-DD.
fn usage_day(value: Option<&str>, name: &str) -> Result<NaiveDate, Status> {
    value
        .ok_or_else(|| Status::invalid_argument(format!("`{name}` is required without a period")))?
        .parse::<NaiveDate>()
        .map_err(|_| Status::invalid_argument(format!("`{name}` must be a date, YYYY-MM-DD")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn months_from_a_month_end_clamp_to_the_last_day_in_utc() {
        let jan_31 = |year| Utc.with_ymd_and_hms(year, 1, 31, 23, 30, 0).unwrap();
        assert_eq!(
            months_after(jan_31(2026), 1).unwrap(),
            Utc.with_ymd_and_hms(2026, 2, 28, 23, 30, 0).unwrap()
        );
        assert_eq!(
            months_after(jan_31(2028), 1).unwrap(),
            Utc.with_ymd_and_hms(2028, 2, 29, 23, 30, 0).unwrap()
        );
    }

    #[test]
    fn expiry_is_a_timestamp_or_months_never_both_and_months_are_bounded() {
        let now = Utc.with_ymd_and_hms(2026, 3, 15, 0, 0, 0).unwrap();
        let at = prost_types::Timestamp {
            seconds: now.timestamp() + 86_400,
            nanos: 0,
        };
        assert!(credit_expiry(Some(at), Some(1), now).is_err());
        assert!(credit_expiry(None, Some(0), now).is_err());
        assert!(credit_expiry(None, Some(MAX_EXPIRY_MONTHS + 1), now).is_err());
        assert_eq!(credit_expiry(None, None, now).unwrap(), None);
        assert_eq!(
            credit_expiry(None, Some(12), now).unwrap(),
            Some(Utc.with_ymd_and_hms(2027, 3, 15, 0, 0, 0).unwrap())
        );
    }
}

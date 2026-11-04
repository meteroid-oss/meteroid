//! One purchase workflow for self-service and automatic top-ups. An anchor operation arms a draft
//! purchase document and a pinned payment, the card is charged with no transaction open, and the
//! invoice-paid path funds the wallet; the anchor settles in the same transaction.
use super::purchase::credit_purchase_line;
use super::top_up_support::{
    customer_safe_decline_reason, increment_attempts, metadata_expires_at, metadata_object,
    metadata_payment_method_id, minimum_charge_minor, truncate_error,
};
use super::unit_wallets::{
    UNIT_SALE_KEY, UnitPriceSource, UnitSale, pin_charge, pinned_total_minor, pinned_unit_sale,
    resolve_unit_sale,
};
use super::wallet_payment::preparation_rejected;
use super::{invalid, precondition};
use crate::StoreResult;
use crate::adapters::payment::error::CustomerFacingMessage;
use crate::domain::credit::{
    AutomaticTopUp, AutomaticTopUpConfig, AutomaticTopUpMethod, MAX_SELF_SERVICE_CHARGE_MINOR,
    SelfServiceTopUpState, TopUpWallet, TopUpWalletQuote,
};
use crate::domain::entity_activity::Actor;
use crate::domain::enums::DeclineKind;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::domain::{
    CreditPurchaseLineName, Customer, Invoice, PaymentStatusEnum as DomainPaymentStatus,
};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::customer_credit::{lock_customer_tx, normalized_currency, source};
use crate::repositories::outbox::publish_outbox_events_tx;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use crate::utils::money::format_money;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{
    BaseId, CreditOperationId, CustomerId, CustomerPaymentMethodId, InvoiceId,
    PaymentTransactionId, TenantId, WalletId,
};
use diesel_models::customer_credit::{
    CreditPurchaseRow, CreditPurchaseRowNew, CustomerCreditAccountRow, CustomerCreditGrantRow,
    CustomerCreditOperationRow, CustomerCreditOperationRowNew,
};
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::{InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use diesel_models::query::credit_inputs;
use diesel_models::query::customer_credit_operation::state;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const MANUAL_TOP_UP_NAMESPACE: &str = "MANUAL_TOP_UP";
/// A top-up whose price moved since the customer saw it is refused before anything is charged.
pub(crate) const PRICE_CHANGED_REFRESH: &str = "The price changed; refresh before paying.";
pub const AUTOMATIC_TOP_UP_NAMESPACE: &str = "AUTOMATIC_TOP_UP";
/// Self-service purchases a customer may open, and charges it may attempt, per rolling day,
/// whatever their outcome.
const MAX_TOP_UPS_PER_ROLLING_DAY: i64 = 10;
const ROLLING_LIMIT_HOURS: i64 = 24;
const EXECUTOR_BATCH_SIZE: i64 = 50;
/// An attempt untouched this long lost its provider answer and is resumed by the reclaimer.
const STUCK_PROCESSING_MINUTES: i64 = 10;
/// An authentication the customer never completed ends the attempt and voids its document; a
/// capture that still arrives reissues it, or is refunded.
const ABANDONED_AUTHENTICATION_HOURS: i64 = 24;
/// Reclaims after which a charge the provider never acknowledged is ended.
const MAX_UNACKNOWLEDGED_ATTEMPTS: i64 = 36;
/// Times a self-service request restarts when its key was re-armed between read and lock.
const MAX_RELOCKS: usize = 3;

/// Who started the purchase: the workflow is shared, the origin decides the charge context and
/// what a failed attempt leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PurchaseOrigin {
    SelfService,
    Automatic,
}

impl PurchaseOrigin {
    fn of(operation: &CustomerCreditOperationRow) -> Option<Self> {
        match operation.namespace.as_str() {
            MANUAL_TOP_UP_NAMESPACE => Some(Self::SelfService),
            AUTOMATIC_TOP_UP_NAMESPACE => Some(Self::Automatic),
            _ => None,
        }
    }

    fn payment_type(self) -> PaymentTypeEnum {
        match self {
            Self::SelfService => PaymentTypeEnum::TopUp,
            Self::Automatic => PaymentTypeEnum::AutoTopup,
        }
    }

    fn customer_initiated(self) -> bool {
        self == Self::SelfService
    }

    fn actor(self, customer_id: CustomerId) -> Actor {
        match self {
            Self::SelfService => Actor::Customer { id: customer_id },
            Self::Automatic => Actor::System,
        }
    }

    /// An off-session charge cannot wait for the customer to authenticate.
    fn accepts(self, intent: &PaymentIntent) -> bool {
        match intent.status {
            DomainPaymentStatus::Settled => true,
            DomainPaymentStatus::Pending => {
                self == Self::SelfService || intent.next_action.is_none()
            }
            _ => false,
        }
    }

    fn document_name(self) -> &'static str {
        match self {
            Self::SelfService => CreditPurchaseLineName::WalletTopUp.stored_name(),
            Self::Automatic => CreditPurchaseLineName::AutomaticTopUp.stored_name(),
        }
    }
}

/// The outcome of a top-up request: the charge and the purchase document it funds. A charge
/// waiting on the customer's authentication carries the step to complete.
#[derive(Debug, Clone)]
pub struct TopUpWalletReceipt {
    pub transaction: PaymentTransaction,
    pub next_action: Option<PaymentNextAction>,
    pub invoice_id: InvoiceId,
    pub wallet_id: WalletId,
    pub charge_minor: i64,
    pub currency: String,
    pub credit_minor: i64,
}

/// One armed charge attempt: everything the provider call and the accept transaction need, all
/// of it committed.
struct ArmedPurchase {
    origin: PurchaseOrigin,
    tenant_id: TenantId,
    customer_id: CustomerId,
    wallet_id: WalletId,
    operation_id: CreditOperationId,
    payment_transaction_id: PaymentTransactionId,
    invoice_id: InvoiceId,
    charge_minor: i64,
    currency: String,
    credit_minor: i64,
}

enum PurchaseCharge {
    Accepted(Box<TopUpWalletReceipt>),
    Declined(String),
    InFlight(ArmedPurchase),
}

enum SelfServiceAttempt {
    Replayed(Box<TopUpWalletReceipt>),
    Charge(ArmedPurchase),
    /// The key was re-armed after its payment was read: lock the new payment first.
    Relock,
}

enum ResumedTopUp {
    Receipt(Box<TopUpWalletReceipt>),
    Charge(ArmedPurchase),
    /// The previous attempt is definitively dead; a fresh one is armed under the same key.
    Restart(Box<CustomerCreditOperationRow>),
}

enum StuckPurchase {
    Resend(ArmedPurchase),
    Captured(PaymentTransactionEvent, InvoiceId),
}

enum InFlight {
    Dead(String),
    Captured(PaymentTransactionEvent, InvoiceId),
    Pending(ArmedPurchase),
}

pub(crate) enum ReissuedPurchase {
    Document(InvoiceId),
    Detached,
}

impl ServicesEdge {
    #[doc(hidden)]
    pub async fn arm_automatic_top_up_for_test(
        &self,
        candidate: CustomerCreditOperationRow,
    ) -> StoreResult<bool> {
        let services = self.services.clone();
        self.store
            .transaction(move |conn| {
                async move {
                    services
                        .arm_automatic_top_up_tx(conn, &candidate)
                        .await
                        .map(|armed| armed.is_some())
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn top_up_checkout_receipt(
        &self,
        session: &crate::domain::CheckoutSession,
    ) -> StoreResult<Option<TopUpWalletReceipt>> {
        let purchase = session.top_up()?;
        self.store
            .transaction(|conn| {
                async move {
                    let lock =
                        lock_customer_tx(conn, session.tenant_id, session.customer_id).await?;
                    let Some(operation) = CustomerCreditOperationRow::find_idempotent_for_update(
                        conn,
                        &lock,
                        MANUAL_TOP_UP_NAMESPACE,
                        &purchase.idempotency_key,
                    )
                    .await?
                    .filter(|operation| {
                        operation.account_id == purchase.wallet_id && operation.invoice_id.is_some()
                    }) else {
                        return Ok(None);
                    };
                    let mut receipt = replay_receipt(conn, &operation).await?;
                    receipt.next_action = receipt.transaction.next_action.clone();
                    Ok(Some(receipt))
                }
                .scope_boxed()
            })
            .await
    }

    /// The price the purchase will charge, checked by the same rules the purchase applies. A
    /// credit unit wallet sells at the customer's agreed price, else its list price.
    pub async fn quote_wallet_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
    ) -> StoreResult<TopUpWalletQuote> {
        self.quote_wallet_top_up_at(tenant_id, customer_id, wallet_id, amount_minor, None)
            .await
    }

    /// `quote_wallet_top_up` for the customer's money wallet in their billing currency, priced
    /// without opening it: money credit is charged at face value and taxed when spent.
    pub async fn quote_money_wallet_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        amount_minor: i64,
    ) -> StoreResult<TopUpWalletQuote> {
        if amount_minor <= 0 {
            return Err(invalid("a top-up adds a positive amount of credit"));
        }
        let mut conn = self.store.get_conn().await?;
        let customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let quote = TopUpWalletQuote {
            charge_minor: amount_minor,
            currency: normalized_currency(&customer.currency),
            net_minor: amount_minor,
            tax_minor: 0,
        };
        ensure_chargeable(&quote)?;
        Ok(quote)
    }

    /// `quote_wallet_top_up` at an operator's negotiated price for one credit of a unit wallet.
    pub async fn quote_wallet_top_up_at(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<TopUpWalletQuote> {
        let mut conn = self.store.get_conn().await?;
        let account = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        self.services
            .price_top_up_tx(&mut conn, &account, amount_minor, None, unit_price)
            .await
            .map(|(quote, _)| quote)
    }

    /// Charges the customer's saved payment method for wallet credit. With a quote, a price that
    /// changed since it was shown is refused before anything is charged.
    pub async fn top_up_wallet(
        &self,
        command: TopUpWallet,
        quote: Option<TopUpWalletQuote>,
    ) -> StoreResult<TopUpWalletReceipt> {
        self.services.top_up_wallet(command, quote).await
    }

    /// Where the customer's top-up sent under `idempotency_key` stands; None when none was sent.
    pub async fn self_service_top_up_state(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        idempotency_key: String,
    ) -> StoreResult<Option<SelfServiceTopUpState>> {
        self.services
            .self_service_top_up_state(tenant_id, customer_id, wallet_id, idempotency_key)
            .await
    }

    /// Resumes top-ups whose provider answer was lost: a captured one is settled, an unanswered
    /// one is resent under its pinned payment, a dead one is ended.
    pub async fn run_wallet_purchase_executor(&self) -> StoreResult<()> {
        self.services.run_wallet_purchase_executor().await
    }
}

/// Why a top-up payment that reports no error did not settle, in words a customer can read.
fn unsettled_reason(
    status: &crate::domain::enums::PaymentStatusEnum,
    awaits_authentication: bool,
) -> String {
    use crate::domain::enums::PaymentStatusEnum as Status;
    match status {
        Status::Pending if awaits_authentication => {
            "the payment needs the customer to authenticate it with their bank"
        }
        Status::Cancelled => "the payment was cancelled",
        Status::Refunded => "the payment was refunded",
        Status::Ready | Status::Pending | Status::Settled => "the payment did not complete",
        Status::Failed => "the payment failed",
    }
    .to_owned()
}

fn intent_unsettled_reason(intent: &PaymentIntent) -> String {
    unsettled_reason(&intent.status, intent.next_action.is_some())
}

/// A charge the card networks accept and self-service may make: at least the currency's minimum
/// before tax, at most `MAX_SELF_SERVICE_CHARGE_MINOR` in all.
fn ensure_chargeable(quote: &TopUpWalletQuote) -> StoreResult<()> {
    let minimum = minimum_charge_minor(&quote.currency)?;
    if quote.net_minor < minimum {
        return Err(invalid(format!(
            "a top-up charges at least {} before tax",
            format_money(minimum, &quote.currency)
        )));
    }
    if quote.charge_minor > MAX_SELF_SERVICE_CHARGE_MINOR {
        return Err(invalid(format!(
            "a top-up charges at most {}",
            format_money(MAX_SELF_SERVICE_CHARGE_MINOR, &quote.currency)
        )));
    }
    Ok(())
}

impl Services {
    /// The price of a top-up of `amount_minor` into `account`. A credit unit wallet sells at
    /// `sale` when the top-up pinned one, otherwise at the customer's agreed price or the list
    /// price in the customer's currency,
    /// taxed now; money credit is taxed when it is spent, so its charge is the credit itself.
    async fn price_top_up_tx(
        &self,
        conn: &mut PgConn,
        account: &CustomerCreditAccountRow,
        amount_minor: i64,
        pinned: Option<&UnitSale>,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<(TopUpWalletQuote, Option<UnitSale>)> {
        let (quote, sale) = self
            .price_top_up_unchecked_tx(conn, account, amount_minor, pinned, unit_price)
            .await?;
        ensure_chargeable(&quote)?;
        Ok((quote, sale))
    }

    /// `price_top_up_tx` without the self-service bounds: what one top-up would charge.
    pub(crate) async fn price_top_up_unchecked_tx(
        &self,
        conn: &mut PgConn,
        account: &CustomerCreditAccountRow,
        amount_minor: i64,
        pinned: Option<&UnitSale>,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<(TopUpWalletQuote, Option<UnitSale>)> {
        if account.credit_unit_id.is_none() && unit_price.is_some() {
            return Err(invalid("a unit price applies to credit unit wallets only"));
        }
        if amount_minor <= 0 {
            return Err(invalid("a top-up adds a positive amount of credit"));
        }
        let (quote, sale) = if account.credit_unit_id.is_none() {
            let quote = TopUpWalletQuote {
                charge_minor: amount_minor,
                currency: normalized_currency(&account.currency),
                net_minor: amount_minor,
                tax_minor: 0,
            };
            (quote, None)
        } else {
            let customer: Customer =
                CustomerRow::find_by_id(conn, &account.customer_id, &account.tenant_id)
                    .await?
                    .try_into()?;
            let sale = match pinned {
                Some(sale) => sale.clone(),
                None => resolve_unit_sale(conn, account, &customer.currency, unit_price)
                    .await?
                    .ok_or_else(|| precondition("the wallet holds no credit unit"))?,
            };
            let quote = self
                .quote_unit_sale_tx(conn, &customer, &sale, amount_minor)
                .await?;
            (quote, Some(sale))
        };
        Ok((quote, sale))
    }

    /// `quote_wallet_top_up_at`, with the sale it priced for a credit unit wallet.
    pub(crate) async fn price_wallet_top_up(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        unit_price: Option<rust_decimal::Decimal>,
    ) -> StoreResult<(TopUpWalletQuote, Option<UnitSale>)> {
        let mut conn = self.store.get_conn().await?;
        let account = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        self.price_top_up_tx(&mut conn, &account, amount_minor, None, unit_price)
            .await
    }

    pub(crate) async fn top_up_wallet(
        &self,
        command: TopUpWallet,
        quote: Option<TopUpWalletQuote>,
    ) -> StoreResult<TopUpWalletReceipt> {
        if command.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(invalid("top-up credit expiry must be in the future"));
        }
        self.create_top_up_checkout(
            command.tenant_id,
            command.customer_id,
            command.wallet_id,
            command.amount_minor,
            command.idempotency_key.clone(),
            command.expires_at,
            command.unit_price,
            quote.clone(),
            None,
        )
        .await?;
        let key = command.idempotency_key.trim();
        if key.is_empty() || key.len() > 255 {
            return Err(invalid("a top-up idempotency key is 1 to 255 bytes"));
        }
        let mut relocks = 0;
        let armed = loop {
            let (services, command, quote) = (self.clone(), command.clone(), quote.clone());
            let attempt = self
                .store
                .transaction(move |conn| {
                    async move {
                        services
                            .begin_top_up_tx(conn, &command, quote.as_ref())
                            .await
                    }
                    .scope_boxed()
                })
                .await?;
            match attempt {
                SelfServiceAttempt::Replayed(receipt) => return Ok(*receipt),
                SelfServiceAttempt::Charge(armed) => break armed,
                SelfServiceAttempt::Relock if relocks < MAX_RELOCKS => relocks += 1,
                SelfServiceAttempt::Relock => {
                    return Err(Report::new(StoreError::Conflict(
                        "this top-up is being retried concurrently; try again".to_owned(),
                    )));
                }
            }
        };
        match self.charge_purchase(armed).await? {
            PurchaseCharge::Accepted(receipt) => Ok(*receipt),
            // The provider's reason stays for the operator; the customer reads only its class.
            PurchaseCharge::Declined(reason) => {
                Err(precondition(format!("the top-up payment failed: {reason}"))
                    .attach_opaque(CustomerFacingMessage(customer_safe_decline_reason(&reason))))
            }
            PurchaseCharge::InFlight(armed) => self.in_flight_receipt(armed).await,
        }
    }

    async fn self_service_top_up_state(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        idempotency_key: String,
    ) -> StoreResult<Option<SelfServiceTopUpState>> {
        let services = self.clone();
        self.store
            .transaction(move |conn| {
                async move {
                    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
                    let Some(operation) = CustomerCreditOperationRow::find_idempotent_for_update(
                        conn,
                        &lock,
                        MANUAL_TOP_UP_NAMESPACE,
                        idempotency_key.trim(),
                    )
                    .await?
                    .filter(|operation| operation.account_id == wallet_id) else {
                        return Ok(None);
                    };
                    let failed = |reason: &str| SelfServiceTopUpState::Failed {
                        reason: customer_safe_decline_reason(reason),
                    };
                    Ok(Some(match operation.state.as_str() {
                        state::SETTLED => SelfServiceTopUpState::Succeeded,
                        state::FAILED | state::CANCELLED => failed(
                            operation
                                .metadata
                                .get("last_error")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                        _ => match services.inspect_in_flight_tx(conn, &operation).await? {
                            InFlight::Dead(reason) => failed(&reason),
                            InFlight::Captured(_, _) => SelfServiceTopUpState::Succeeded,
                            InFlight::Pending(armed) => SelfServiceTopUpState::Pending {
                                requires_action: PaymentTransactionRow::get_by_id(
                                    conn,
                                    armed.payment_transaction_id,
                                    tenant_id,
                                )
                                .await?
                                .next_action
                                .is_some(),
                            },
                        },
                    }))
                }
                .scope_boxed()
            })
            .await
    }

    /// The sale a new top-up of a credit unit wallet pins, quoted with its tax, and its metadata
    /// entry; None for a wallet held in money.
    async fn pin_top_up_sale_tx(
        &self,
        conn: &mut PgConn,
        command: &TopUpWallet,
    ) -> StoreResult<Option<PinnedTopUp>> {
        let account =
            CustomerCreditAccountRow::find_by_id(conn, command.tenant_id, command.wallet_id)
                .await?
                .filter(|account| account.customer_id == command.customer_id)
                .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        if account.credit_unit_id.is_none() {
            if command.unit_price.is_some() || command.expected_unit_price.is_some() {
                return Err(invalid("a unit price applies to credit unit wallets only"));
            }
            return Ok(None);
        }
        let customer: Customer =
            CustomerRow::find_by_id(conn, &command.customer_id, &command.tenant_id)
                .await?
                .try_into()?;
        let Some(sale) =
            resolve_unit_sale(conn, &account, &customer.currency, command.unit_price).await?
        else {
            return Ok(None);
        };
        super::purchase::ensure_expected_unit_price(Some(&sale), command.expected_unit_price)?;
        let quote = self
            .quote_unit_sale_tx(conn, &customer, &sale, command.amount_minor)
            .await?;
        let pin = sale.pin(command.amount_minor, Some(&quote))?;
        Ok(Some(PinnedTopUp { sale, pin, quote }))
    }

    /// Locks in the credit order (`lock_customer_tx`): the key's payment, the customer, the
    /// anchor, its checkout session, the wallet.
    async fn begin_top_up_tx(
        &self,
        conn: &mut PgConn,
        command: &TopUpWallet,
        quote: Option<&TopUpWalletQuote>,
    ) -> StoreResult<SelfServiceAttempt> {
        let key = command.idempotency_key.trim();
        let fingerprint = request_fingerprint(command);
        let seen = CustomerCreditOperationRow::find_idempotent(
            conn,
            command.tenant_id,
            command.customer_id,
            MANUAL_TOP_UP_NAMESPACE,
            key,
        )
        .await?;
        let seen_payment = seen
            .as_ref()
            .and_then(|operation| operation.payment_transaction_id);
        let lock = lock_customer_and_payment_tx(
            conn,
            command.tenant_id,
            command.customer_id,
            seen_payment,
        )
        .await?;
        // A new top-up pins its price now; a replay keeps the one it pinned.
        let pinned = match seen {
            Some(_) => None,
            None => self.pin_top_up_sale_tx(conn, command).await?,
        };
        let inserted = new_top_up_anchor(command, &fingerprint, pinned)?
            .insert_conflict_do_nothing(conn)
            .await?;
        let operation = match inserted {
            Some(operation) => operation,
            None => {
                // Keys are scoped to the customer: another customer's key never resolves here.
                let existing = CustomerCreditOperationRow::find_idempotent_for_update(
                    conn,
                    &lock,
                    MANUAL_TOP_UP_NAMESPACE,
                    key,
                )
                .await?
                .ok_or_else(|| precondition("the top-up idempotency anchor is missing"))?;
                if existing.request_fingerprint != fingerprint {
                    return Err(Report::new(StoreError::Conflict(
                        "this idempotency key was used for a different top-up".to_owned(),
                    )));
                }
                if existing.payment_transaction_id != seen_payment {
                    return Ok(SelfServiceAttempt::Relock);
                }
                match self.resume_top_up_tx(conn, &lock, existing).await? {
                    ResumedTopUp::Receipt(receipt) => {
                        return Ok(SelfServiceAttempt::Replayed(receipt));
                    }
                    ResumedTopUp::Charge(armed) => return Ok(SelfServiceAttempt::Charge(armed)),
                    ResumedTopUp::Restart(operation) => *operation,
                }
            }
        };
        let session: crate::domain::CheckoutSession =
            diesel_models::checkout_sessions::CheckoutSessionRow::get_by_id_for_update(
                conn,
                command.tenant_id,
                super::top_up_checkout::session_id(command.tenant_id, command.customer_id, key),
            )
            .await?
            .1
            .into();
        if !session.can_complete() {
            return Err(precondition("top-up checkout has expired or completed"));
        }
        let opened = CustomerCreditOperationRow::count_since(
            conn,
            command.tenant_id,
            command.customer_id,
            MANUAL_TOP_UP_NAMESPACE,
            Utc::now() - Duration::hours(ROLLING_LIMIT_HOURS),
        )
        .await?;
        // Counted per charge attempt too: retrying a declined key must not test card after card.
        let attempted = CustomerCreditOperationRow::count_attempts_since(
            conn,
            command.tenant_id,
            command.customer_id,
            Utc::now() - Duration::hours(ROLLING_LIMIT_HOURS),
        )
        .await?;
        if opened > MAX_TOP_UPS_PER_ROLLING_DAY || attempted >= MAX_TOP_UPS_PER_ROLLING_DAY {
            let message = format!(
                "at most {MAX_TOP_UPS_PER_ROLLING_DAY} top-ups are allowed per {ROLLING_LIMIT_HOURS} hours"
            );
            return Err(
                precondition(message.clone()).attach_opaque(crate::errors::RateLimited(message))
            );
        }
        let account = CustomerCreditAccountRow::lock_by_id(conn, &lock, command.wallet_id)
            .await?
            .filter(|account| account.customer_id == command.customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        let sale = pinned_unit_sale(conn, &account, &operation.metadata).await?;
        if account.credit_unit_id.is_some() && sale.is_none() {
            return Err(precondition("the top-up has no pinned credit price"));
        }
        // The pinned charge is the price; a page that showed another price id was outdated.
        let priced = match (
            super::unit_wallets::pinned_quote(&operation.metadata),
            &sale,
        ) {
            (Some(pinned), Some(_)) => {
                ensure_chargeable(&pinned)?;
                pinned
            }
            _ => {
                self.price_top_up_tx(conn, &account, command.amount_minor, sale.as_ref(), None)
                    .await?
                    .0
            }
        };
        let shown_price = session.top_up()?.unit_sale.map(|shown| shown.price_id);
        if let Some(quote) = quote
            && *quote != priced
        {
            return Err(precondition(PRICE_CHANGED_REFRESH));
        }
        if let (Some(shown), Some(sale)) = (shown_price, &sale)
            && shown != sale.price_id
        {
            return Err(precondition(PRICE_CHANGED_REFRESH));
        }
        self.arm_purchase_tx(
            conn,
            &lock,
            PurchaseOrigin::SelfService,
            &account,
            &operation,
            command.payment_method_id,
        )
        .await
        .map(SelfServiceAttempt::Charge)
    }

    /// A replay of a key that already has an anchor: report a finished top-up, resume an attempt
    /// still in flight (same pinned payment, so no second charge), or bury a dead one.
    async fn resume_top_up_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        operation: CustomerCreditOperationRow,
    ) -> StoreResult<ResumedTopUp> {
        match operation.state.as_str() {
            state::SETTLED => Ok(ResumedTopUp::Receipt(Box::new(
                replay_receipt(conn, &operation).await?,
            ))),
            state::CANCELLED => Err(precondition("this top-up was cancelled")),
            state::FAILED => Ok(ResumedTopUp::Restart(Box::new(operation))),
            _ => match self.inspect_in_flight_tx(conn, &operation).await? {
                InFlight::Dead(reason) => {
                    let ended = self
                        .fail_purchase_tx(conn, lock, &operation, reason, None)
                        .await?;
                    Ok(if ended.state == state::FAILED {
                        ResumedTopUp::Restart(Box::new(ended))
                    } else {
                        ResumedTopUp::Receipt(Box::new(replay_receipt(conn, &ended).await?))
                    })
                }
                // Captured out of band: the invoice-paid path funds the wallet and settles it.
                InFlight::Captured(_, _) => Ok(ResumedTopUp::Receipt(Box::new(
                    replay_receipt(conn, &operation).await?,
                ))),
                InFlight::Pending(armed) => Ok(ResumedTopUp::Charge(armed)),
            },
        }
    }

    /// Resolves an attempt left in flight from its durable payment alone.
    async fn inspect_in_flight_tx(
        &self,
        conn: &mut PgConn,
        operation: &CustomerCreditOperationRow,
    ) -> StoreResult<InFlight> {
        let (Some(payment_transaction_id), Some(invoice_id)) =
            (operation.payment_transaction_id, operation.invoice_id)
        else {
            return Ok(InFlight::Dead(
                "the top-up attempt was never armed".to_owned(),
            ));
        };
        let payment =
            PaymentTransactionRow::get_by_id(conn, payment_transaction_id, operation.tenant_id)
                .await?;
        if payment.status == PaymentStatusEnum::Pending
            && payment.next_action.is_some()
            && let Some(id) = payment.checkout_session_id
        {
            let session: crate::domain::CheckoutSession =
                diesel_models::checkout_sessions::CheckoutSessionRow::get_by_id(
                    conn,
                    operation.tenant_id,
                    id,
                )
                .await?
                .into();
            if !session.can_complete() {
                return Ok(InFlight::Dead(
                    "the top-up checkout expired or was cancelled".to_owned(),
                ));
            }
        }
        let attempts = operation
            .metadata
            .get("attempts")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok(match payment.status {
            PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled => InFlight::Dead(
                payment
                    .error_type
                    .clone()
                    .unwrap_or_else(|| "the card charge was declined".to_owned()),
            ),
            PaymentStatusEnum::Settled | PaymentStatusEnum::Refunded => {
                let transaction: PaymentTransaction = payment.into();
                InFlight::Captured(transaction.into(), invoice_id)
            }
            PaymentStatusEnum::Pending
                if payment.next_action.is_some()
                    && payment.created_at
                        < Utc::now() - Duration::hours(ABANDONED_AUTHENTICATION_HOURS) =>
            {
                InFlight::Dead(
                    "the payment authentication was not completed within 24 hours".to_owned(),
                )
            }
            PaymentStatusEnum::Pending | PaymentStatusEnum::Ready
                if payment.provider_transaction_id.is_none()
                    && attempts >= MAX_UNACKNOWLEDGED_ATTEMPTS =>
            {
                InFlight::Dead("the payment provider never acknowledged the charge".to_owned())
            }
            _ => InFlight::Pending(armed_from_operation(
                operation,
                payment_transaction_id,
                invoice_id,
            )?),
        })
    }

    /// Drafts the purchase document and commits a Pending payment that pins the provider
    /// identity before any charge.
    async fn arm_purchase_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        origin: PurchaseOrigin,
        account: &CustomerCreditAccountRow,
        operation: &CustomerCreditOperationRow,
        payment_method_id: CustomerPaymentMethodId,
    ) -> StoreResult<ArmedPurchase> {
        let customer: Customer =
            CustomerRow::find_by_id(conn, &operation.customer_id, &operation.tenant_id)
                .await?
                .try_into()?;
        // A foreign or retired method reads back as not found, never confirming it exists.
        let method =
            CustomerPaymentMethodRow::get_by_id(conn, &operation.tenant_id, &payment_method_id)
                .await?;
        if method.customer_id != operation.customer_id || method.archived_at.is_some() {
            return Err(Report::new(StoreError::ValueNotFound(
                "payment method".to_owned(),
            )));
        }
        let sale = pinned_unit_sale(conn, account, &operation.metadata).await?;
        if account.credit_unit_id.is_some() && sale.is_none() {
            return Err(precondition("the top-up has no pinned credit price"));
        }
        let invoice = self
            .draft_purchase_invoice(
                conn,
                lock,
                origin,
                &customer,
                Utc::now().date_naive(),
                account,
                operation.amount_minor,
                sale.as_ref(),
                None,
                metadata_expires_at(&operation.metadata)?,
            )
            .await?;
        let charge_minor = purchase_charge(&invoice, operation.amount_minor, sale.as_ref())
            .filter(|charge| {
                pinned_total_minor(&operation.metadata).is_none_or(|pinned| pinned == *charge)
            })
            .ok_or_else(|| {
                precondition("the top-up purchase document does not match the requested charge")
            })?;
        let mut metadata = operation.metadata.clone();
        if sale.is_some() {
            pin_charge(&mut metadata, invoice.tax_amount, charge_minor)?;
        }
        let payment_transaction_id = PaymentTransactionId::new();
        PaymentTransactionRowNew {
            id: payment_transaction_id,
            tenant_id: operation.tenant_id,
            // Linked before the document is finalized, so it is never auto-charged or paid by credit.
            invoice_id: Some(invoice.id),
            provider_transaction_id: None,
            amount: charge_minor,
            currency: invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: origin.payment_type(),
            error_type: None,
            processed_at: None,
            checkout_session_id: origin.customer_initiated().then(|| {
                super::top_up_checkout::session_id(
                    operation.tenant_id,
                    operation.customer_id,
                    &operation.idempotency_key,
                )
            }),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: origin.customer_initiated().then_some(operation.customer_id),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(conn)
        .await?;

        let fields = metadata_object(&mut metadata)?;
        // The outcome of a previous attempt under the same key; this attempt starts clean.
        fields.remove("last_error");
        if let Some(previous) = operation.payment_transaction_id {
            let superseded = fields
                .entry(CustomerCreditOperationRow::SUPERSEDED_PAYMENTS_KEY.to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            match superseded {
                Value::Array(ids) => ids.push(Value::String(previous.to_string())),
                _ => return Err(precondition("the top-up superseded payments are malformed")),
            }
        }
        fields.insert(
            "payment_method_id".to_owned(),
            Value::String(payment_method_id.to_string()),
        );
        let operation = CustomerCreditOperationRow::start_attempt(
            conn,
            operation.id,
            payment_transaction_id,
            invoice.id,
            charge_minor,
            &invoice.currency,
            increment_attempts(metadata)?,
        )
        .await?;
        Ok(ArmedPurchase {
            origin,
            tenant_id: operation.tenant_id,
            customer_id: operation.customer_id,
            wallet_id: operation.account_id,
            operation_id: operation.id,
            payment_transaction_id,
            invoice_id: invoice.id,
            charge_minor,
            currency: invoice.currency,
            credit_minor: operation.amount_minor,
        })
    }

    /// Charges with no transaction open, then records the outcome. An error the provider object
    /// does not settle keeps the attempt in flight for the reclaimer.
    async fn charge_purchase(&self, armed: ArmedPurchase) -> StoreResult<PurchaseCharge> {
        let result = self
            .resume_prepared_wallet_payment(
                armed.tenant_id,
                armed.customer_id,
                armed.payment_transaction_id,
                armed.origin.customer_initiated(),
            )
            .await;
        let (reason, decline_kind) = match result {
            Ok(intent) if armed.origin.accepts(&intent) => {
                return self
                    .accept_purchase(armed, intent)
                    .await
                    .map(|receipt| PurchaseCharge::Accepted(Box::new(receipt)));
            }
            Ok(intent) => (
                intent
                    .last_payment_error
                    .clone()
                    .unwrap_or_else(|| intent_unsettled_reason(&intent)),
                intent.decline_kind,
            ),
            Err(error) => match preparation_rejected(&error) {
                Some(reason) => (reason, None),
                None => {
                    log::warn!("top-up {} remains in flight: {error:?}", armed.operation_id);
                    return Ok(PurchaseCharge::InFlight(armed));
                }
            },
        };
        let reason = truncate_error(reason);
        self.fail_armed_purchase(&armed, reason.clone(), decline_kind)
            .await?;
        Ok(PurchaseCharge::Declined(reason))
    }

    /// The charge was accepted: issue the purchase document and record the provider outcome. A
    /// captured charge funds the wallet through the invoice-paid path.
    async fn accept_purchase(
        &self,
        armed: ArmedPurchase,
        intent: PaymentIntent,
    ) -> StoreResult<TopUpWalletReceipt> {
        let services = self.clone();
        let next_action = intent.next_action.clone();
        let transaction = self
            .store
            .transaction(move |conn| {
                async move {
                    let lock = lock_customer_and_payment_tx(
                        conn,
                        armed.tenant_id,
                        armed.customer_id,
                        Some(armed.payment_transaction_id),
                    )
                    .await?;
                    let operation =
                        CustomerCreditOperationRow::lock_by_id(conn, &lock, armed.operation_id)
                            .await?;
                    if operation.payment_transaction_id != Some(armed.payment_transaction_id) {
                        return Err(precondition("the top-up attempt was superseded"));
                    }
                    if operation.state != state::PROCESSING && operation.state != state::SETTLED {
                        return Err(precondition("the top-up attempt is no longer in flight"));
                    }
                    let invoice =
                        InvoiceRow::find_by_id(conn, armed.tenant_id, armed.invoice_id).await?;
                    let actor = armed.origin.actor(armed.customer_id);
                    if invoice.status == InvoiceStatusEnum::Draft && (armed.origin == PurchaseOrigin::Automatic || intent.status == DomainPaymentStatus::Settled) {
                        services
                            .finalize_invoice_tx(
                                conn,
                                &actor,
                                armed.invoice_id,
                                armed.tenant_id,
                                false,
                                &None,
                            )
                            .await?;
                    }
                    if armed.origin == PurchaseOrigin::SelfService && operation.state == state::PROCESSING {
                        diesel_models::checkout_sessions::CheckoutSessionRow::mark_awaiting_payment(
                            conn, armed.tenant_id,
                            super::top_up_checkout::session_id(armed.tenant_id, armed.customer_id, &operation.idempotency_key), None,
                        ).await?;
                    }
                    let transaction = services
                        .store
                        .get_payment_tx_by_id_for_update(
                            conn,
                            armed.payment_transaction_id,
                            armed.tenant_id,
                        )
                        .await?;
                    services
                        .store
                        .consolidate_intent_and_transaction_tx(conn, &actor, transaction, intent)
                        .await
                }
                .scope_boxed()
            })
            .await?;
        Ok(TopUpWalletReceipt {
            transaction,
            next_action,
            invoice_id: armed.invoice_id,
            wallet_id: armed.wallet_id,
            charge_minor: armed.charge_minor,
            currency: armed.currency,
            credit_minor: armed.credit_minor,
        })
    }

    async fn fail_armed_purchase(
        &self,
        armed: &ArmedPurchase,
        reason: String,
        decline_kind: Option<DeclineKind>,
    ) -> StoreResult<()> {
        let services = self.clone();
        let (tenant_id, customer_id, operation_id, payment_transaction_id) = (
            armed.tenant_id,
            armed.customer_id,
            armed.operation_id,
            armed.payment_transaction_id,
        );
        self.store
            .transaction(move |conn| {
                async move {
                    let lock = lock_customer_and_payment_tx(
                        conn,
                        tenant_id,
                        customer_id,
                        Some(payment_transaction_id),
                    )
                    .await?;
                    let operation =
                        CustomerCreditOperationRow::lock_by_id(conn, &lock, operation_id).await?;
                    if operation.state != state::PROCESSING
                        || operation.payment_transaction_id != Some(payment_transaction_id)
                    {
                        return Ok(());
                    }
                    services
                        .fail_purchase_tx(conn, &lock, &operation, reason, decline_kind)
                        .await
                        .map(|_| ())
                }
                .scope_boxed()
            })
            .await
    }

    /// Ends an attempt that captured nothing; its charge is kept, detached and failed, so a late
    /// capture reissues its document. Advances an automatic top-up's backoff and publishes a
    /// decline the webhook did not. The caller holds the payment, customer and operation, in order.
    async fn fail_purchase_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        operation: &CustomerCreditOperationRow,
        reason: String,
        decline_kind: Option<DeclineKind>,
    ) -> StoreResult<CustomerCreditOperationRow> {
        self.fail_purchase_at_tx(
            conn,
            lock,
            operation,
            reason,
            decline_kind,
            Utc::now(),
            false,
        )
        .await
    }

    /// `fail_purchase_tx` as of `now`; a `replayed` decline happened then, so the customer is not
    /// told of it now.
    async fn fail_purchase_at_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        operation: &CustomerCreditOperationRow,
        reason: String,
        decline_kind: Option<DeclineKind>,
        now: DateTime<Utc>,
        replayed: bool,
    ) -> StoreResult<CustomerCreditOperationRow> {
        let origin = PurchaseOrigin::of(operation)
            .ok_or_else(|| precondition("the operation is not a top-up"))?;
        let reason = truncate_error(reason);
        let tenant_id = operation.tenant_id;
        let mut decline_kind = decline_kind;
        if let Some(payment_transaction_id) = operation.payment_transaction_id {
            let payment =
                PaymentTransactionRow::get_by_id_for_update(conn, lock, payment_transaction_id)
                    .await?;
            // Captured since the caller looked: the settlement path funds and settles it.
            if matches!(
                payment.status,
                PaymentStatusEnum::Settled | PaymentStatusEnum::Refunded
            ) {
                return Ok(operation.clone());
            }
            let open = matches!(
                payment.status,
                PaymentStatusEnum::Pending | PaymentStatusEnum::Ready
            );
            decline_kind = decline_kind.or(payment.decline_kind.map(DeclineKind::from));
            let declined = open && decline_kind.is_some();
            let updated = PaymentTransactionRowPatch {
                id: payment_transaction_id,
                invoice_id: Some(None),
                status: open.then_some(PaymentStatusEnum::Failed),
                error_type: open.then(|| Some(reason.clone())),
                decline_kind: declined.then(|| decline_kind.map(Into::into)),
                processed_at: (open && replayed).then_some(Some(now.naive_utc())),
                ..Default::default()
            }
            .patch(conn, tenant_id, payment_transaction_id)
            .await?;
            if declined {
                let transaction: PaymentTransaction = updated.into();
                publish_outbox_events_tx(
                    conn,
                    vec![OutboxEvent::payment_transaction_saved(transaction.into())],
                )
                .await?;
            }
        }
        if let Some(invoice_id) = operation.invoice_id {
            self.discard_purchase_document_tx(conn, lock, tenant_id, invoice_id)
                .await?;
        }
        let mut metadata = operation.metadata.clone();
        metadata_object(&mut metadata)?
            .insert("last_error".to_owned(), Value::String(reason.clone()));
        let failed =
            CustomerCreditOperationRow::finish(conn, operation.id, state::FAILED, metadata).await?;
        if origin == PurchaseOrigin::Automatic {
            self.store
                .hooks
                .credit
                .automatic_purchase_failed(
                    conn,
                    lock,
                    operation,
                    reason,
                    decline_kind,
                    now,
                    replayed,
                )
                .await?;
        }
        Ok(failed)
    }

    /// A document that captured nothing leaves no trace: a draft is deleted so no invoice number
    /// is consumed, a document finalized meanwhile is voided.
    async fn discard_purchase_document_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        credit_inputs::bump_customer(conn, lock).await?;
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id).await?;
        match invoice.status {
            InvoiceStatusEnum::Draft => InvoiceRow::delete(conn, invoice_id, tenant_id).await?,
            InvoiceStatusEnum::Finalized => {
                InvoiceRow::void(conn, invoice_id, tenant_id).await?;
                self.store
                    .internal
                    .end_reminders_best_effort(conn, tenant_id, invoice_id, Utc::now().naive_utc())
                    .await;
                let voided: Invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                    .await?
                    .try_into()?;
                self.store
                    .internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        &Actor::System,
                        vec![OutboxEvent::invoice_voided((&voided).into())],
                    )
                    .await?;
            }
            // A purchase document always carries its credit line, so it is never Closed as empty.
            InvoiceStatusEnum::Void
            | InvoiceStatusEnum::Uncollectible
            | InvoiceStatusEnum::Closed => {}
        }
        Ok(())
    }

    /// A failed or cancelled top-up payment ends its attempt now instead of waiting for the
    /// reclaimer. True when the payment belongs to a top-up, which owns its failure.
    pub(in crate::services) async fn on_wallet_purchase_payment_failed(
        &self,
        event: &PaymentTransactionEvent,
    ) -> StoreResult<bool> {
        if !matches!(
            event.payment_type,
            crate::domain::PaymentTypeEnum::TopUp | crate::domain::PaymentTypeEnum::AutoTopup
        ) {
            return Ok(false);
        }
        let reason = event
            .error_type
            .clone()
            .unwrap_or_else(|| unsettled_reason(&event.status, false));
        let services = self.clone();
        let (tenant_id, payment_transaction_id) = (event.tenant_id, event.payment_transaction_id);
        self.store
            .transaction(move |conn| {
                async move {
                    let Some((lock, payment, operation)) =
                        lock_top_up_by_payment_tx(conn, tenant_id, payment_transaction_id).await?
                    else {
                        return Ok(true);
                    };
                    // The event may be stale: a capture that landed since has settled the payment.
                    let ended = matches!(
                        payment.status,
                        PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled
                    );
                    if operation.state == state::PROCESSING && ended {
                        services
                            .fail_purchase_tx(conn, &lock, &operation, reason, None)
                            .await?;
                    }
                    Ok(true)
                }
                .scope_boxed()
            })
            .await
    }

    async fn in_flight_receipt(&self, armed: ArmedPurchase) -> StoreResult<TopUpWalletReceipt> {
        let mut conn = self.store.get_conn().await?;
        let transaction: PaymentTransaction = PaymentTransactionRow::get_by_id(
            &mut conn,
            armed.payment_transaction_id,
            armed.tenant_id,
        )
        .await?
        .into();
        Ok(TopUpWalletReceipt {
            transaction,
            next_action: None,
            invoice_id: armed.invoice_id,
            wallet_id: armed.wallet_id,
            charge_minor: armed.charge_minor,
            currency: armed.currency,
            credit_minor: armed.credit_minor,
        })
    }

    /// Arms and charges the automatic top-ups the usage controls queued.
    pub(in crate::services) async fn charge_queued_automatic_top_ups(&self) -> StoreResult<()> {
        for armed in self.arm_automatic_top_ups().await? {
            self.charge_wallet_purchase(armed).await;
        }
        Ok(())
    }

    async fn run_wallet_purchase_executor(&self) -> StoreResult<()> {
        for claimed in self.claim_stuck_purchases().await? {
            match claimed {
                StuckPurchase::Resend(armed) => self.charge_wallet_purchase(armed).await,
                StuckPurchase::Captured(event, invoice_id) => {
                    if let Err(error) = self.on_invoice_payment_settled(event, invoice_id).await {
                        log::error!("top-up reconciliation failed: {error:?}");
                    }
                }
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(
        tenant_id = %armed.tenant_id,
        customer_id = %armed.customer_id,
        invoice_id = %armed.invoice_id,
    ))]
    async fn charge_wallet_purchase(&self, armed: ArmedPurchase) {
        if let Err(error) = self.charge_purchase(armed).await {
            log::error!("wallet top-up charge failed unexpectedly: {error:?}");
        }
    }

    /// Each top-up is armed in its own transaction, so one that cannot be armed yet stays queued
    /// without holding back the others; one that can never be armed is failed like a declined
    /// charge, so it counts toward suspension and frees the wallet for a new top-up.
    async fn arm_automatic_top_ups(&self) -> StoreResult<Vec<ArmedPurchase>> {
        let candidates = {
            let mut conn = self.store.get_conn().await?;
            CustomerCreditOperationRow::list_to_arm(
                &mut conn,
                AUTOMATIC_TOP_UP_NAMESPACE,
                EXECUTOR_BATCH_SIZE,
            )
            .await?
        };
        let mut armed = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let services = self.clone();
            let queued = candidate.clone();
            let outcome = self
                .store
                .transaction(move |conn| {
                    async move { services.arm_automatic_top_up_tx(conn, &queued).await }
                        .scope_boxed()
                })
                .await;
            match outcome {
                Ok(Some(purchase)) => armed.push(purchase),
                Ok(None) => {}
                Err(error) => match preparation_failure(&error) {
                    Some(reason) => {
                        log::warn!(
                            "automatic top-up {} cannot be armed: {error:?}",
                            candidate.id
                        );
                        if let Err(error) = self.fail_queued_top_up(&candidate, reason).await {
                            log::error!(
                                "an unarmable automatic top-up could not be ended: {error:?}"
                            );
                        }
                    }
                    None => log::error!("an automatic top-up could not be armed: {error:?}"),
                },
            }
        }
        Ok(armed)
    }

    async fn fail_queued_top_up(
        &self,
        candidate: &CustomerCreditOperationRow,
        reason: String,
    ) -> StoreResult<()> {
        let services = self.clone();
        let (tenant_id, customer_id, id) =
            (candidate.tenant_id, candidate.customer_id, candidate.id);
        self.store
            .transaction(move |conn| {
                async move {
                    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
                    let operation = CustomerCreditOperationRow::lock_by_id(conn, &lock, id).await?;
                    if operation.state == state::CREATED {
                        services
                            .fail_purchase_tx(conn, &lock, &operation, reason, None)
                            .await?;
                    }
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    async fn arm_automatic_top_up_tx(
        &self,
        conn: &mut PgConn,
        candidate: &CustomerCreditOperationRow,
    ) -> StoreResult<Option<ArmedPurchase>> {
        let lock = lock_customer_tx(conn, candidate.tenant_id, candidate.customer_id).await?;
        let operation = CustomerCreditOperationRow::lock_by_id(conn, &lock, candidate.id).await?;
        if operation.state != state::CREATED {
            return Ok(None);
        }
        let account = CustomerCreditAccountRow::lock_by_id(conn, &lock, candidate.account_id)
            .await?
            .ok_or_else(|| precondition("the automatic top-up wallet is missing"))?;
        let payment_method_id = metadata_payment_method_id(&operation.metadata)?;
        self.arm_purchase_tx(
            conn,
            &lock,
            PurchaseOrigin::Automatic,
            &account,
            &operation,
            payment_method_id,
        )
        .await
        .map(Some)
    }

    /// Attempts left in flight: an unanswered charge is resent under the SAME pinned payment, one
    /// already captured is settled instead of authorized again, a dead one is ended.
    async fn claim_stuck_purchases(&self) -> StoreResult<Vec<StuckPurchase>> {
        let stuck_before = Utc::now() - Duration::minutes(STUCK_PROCESSING_MINUTES);
        let listed = {
            let mut conn = self.store.get_conn().await?;
            CustomerCreditOperationRow::list_stuck(
                &mut conn,
                &[MANUAL_TOP_UP_NAMESPACE, AUTOMATIC_TOP_UP_NAMESPACE],
                stuck_before,
                EXECUTOR_BATCH_SIZE,
            )
            .await?
        };
        let mut claimed = Vec::with_capacity(listed.len());
        for operation in listed {
            let services = self.clone();
            let claim = self
                .store
                .transaction(move |conn| {
                    async move {
                        services
                            .claim_stuck_purchase_tx(conn, &operation, stuck_before)
                            .await
                    }
                    .scope_boxed()
                })
                .await?;
            claimed.extend(claim);
        }
        Ok(claimed)
    }

    /// One listed attempt, locked behind its payment and customer. An attempt another reclaimer
    /// or the attempt itself moved on since it was listed is left alone.
    async fn claim_stuck_purchase_tx(
        &self,
        conn: &mut PgConn,
        listed: &CustomerCreditOperationRow,
        stuck_before: DateTime<Utc>,
    ) -> StoreResult<Option<StuckPurchase>> {
        let lock = lock_customer_and_payment_tx(
            conn,
            listed.tenant_id,
            listed.customer_id,
            listed.payment_transaction_id,
        )
        .await?;
        let operation = CustomerCreditOperationRow::lock_by_id(conn, &lock, listed.id).await?;
        if operation.state != state::PROCESSING
            || operation.updated_at > stuck_before
            || operation.payment_transaction_id != listed.payment_transaction_id
        {
            return Ok(None);
        }
        let in_flight = self.inspect_in_flight_tx(conn, &operation).await?;
        if let InFlight::Dead(reason) = in_flight {
            self.fail_purchase_tx(conn, &lock, &operation, reason, None)
                .await?;
            return Ok(None);
        }
        let metadata = increment_attempts(operation.metadata.clone())?;
        CustomerCreditOperationRow::touch(conn, operation.id, metadata).await?;
        Ok(Some(match in_flight {
            InFlight::Captured(event, invoice_id) => StuckPurchase::Captured(event, invoice_id),
            InFlight::Pending(armed) => StuckPurchase::Resend(armed),
            InFlight::Dead(_) => unreachable!("dead attempts are ended above"),
        }))
    }

    /// A capture that arrived after its attempt failed and discarded its document: the document is
    /// reissued at the recorded price, so the money becomes the credit it bought. Money that can no
    /// longer be matched to its purchase is reported as detached, for the caller to refund.
    pub(in crate::services) async fn reissue_captured_purchase(
        &self,
        event: &PaymentTransactionEvent,
    ) -> StoreResult<Option<ReissuedPurchase>> {
        if !matches!(
            event.payment_type,
            crate::domain::PaymentTypeEnum::TopUp | crate::domain::PaymentTypeEnum::AutoTopup
        ) {
            return Ok(None);
        }
        let services = self.clone();
        let event = event.clone();
        self.store
            .transaction(move |conn| {
                async move {
                    let Some((lock, payment, operation)) = lock_top_up_by_payment_tx(
                        conn,
                        event.tenant_id,
                        event.payment_transaction_id,
                    )
                    .await?
                    .filter(|(_, _, operation)| PurchaseOrigin::of(operation).is_some()) else {
                        return detect_superseded_capture_tx(conn, &event).await;
                    };
                    if let Some(invoice_id) = payment.invoice_id {
                        return Ok(Some(ReissuedPurchase::Document(invoice_id)));
                    }
                    if operation.state != state::FAILED {
                        return Err(precondition(
                            "a detached top-up payment belongs to an attempt still in flight",
                        ));
                    }
                    if payment.status != PaymentStatusEnum::Settled {
                        return Err(precondition("the top-up payment is not settled"));
                    }
                    match services
                        .reissue_purchase_document_tx(conn, &lock, &operation, &payment)
                        .await?
                    {
                        Some(invoice_id) => Ok(Some(ReissuedPurchase::Document(invoice_id))),
                        None => Ok(Some(ReissuedPurchase::Detached)),
                    }
                }
                .scope_boxed()
            })
            .await
    }

    async fn reissue_purchase_document_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        operation: &CustomerCreditOperationRow,
        payment: &PaymentTransactionRow,
    ) -> StoreResult<Option<InvoiceId>> {
        let origin = PurchaseOrigin::of(operation)
            .ok_or_else(|| precondition("the operation is not a top-up"))?;
        let Some(account) =
            CustomerCreditAccountRow::lock_by_id(conn, lock, operation.account_id).await?
        else {
            return Ok(None);
        };
        let sale = pinned_unit_sale(conn, &account, &operation.metadata).await?;
        let (currency, charged) = match &sale {
            Some(sale) => (
                sale.currency.as_str(),
                pinned_total_minor(&operation.metadata).or(operation.expected_consideration_minor),
            ),
            None => (account.currency.as_str(), Some(operation.amount_minor)),
        };
        if !currency.eq_ignore_ascii_case(&payment.currency) || charged != Some(payment.amount) {
            return Ok(None);
        }
        let customer: Customer =
            CustomerRow::find_by_id(conn, &operation.customer_id, &operation.tenant_id)
                .await?
                .try_into()?;
        let invoice_date = payment
            .processed_at
            .unwrap_or_else(|| Utc::now().naive_utc())
            .date();
        let invoice = self
            .draft_purchase_invoice(
                conn,
                lock,
                origin,
                &customer,
                invoice_date,
                &account,
                operation.amount_minor,
                sale.as_ref(),
                None,
                metadata_expires_at(&operation.metadata)?,
            )
            .await?;
        let invoice = if purchase_charge(&invoice, operation.amount_minor, sale.as_ref())
            == Some(payment.amount)
        {
            invoice
        } else {
            InvoiceRow::delete(conn, invoice.id, operation.tenant_id).await?;
            credit_inputs::bump_customer(conn, lock).await?;
            if sale.is_none() {
                return Ok(None);
            }
            // The tax changed since the charge was pinned (a VAT number added, say): the captured
            // money buys the credits it was quoted for, its tax carved out under today's rules.
            let invoice = self
                .draft_purchase_invoice(
                    conn,
                    lock,
                    origin,
                    &customer,
                    invoice_date,
                    &account,
                    operation.amount_minor,
                    sale.as_ref(),
                    Some(payment.amount),
                    metadata_expires_at(&operation.metadata)?,
                )
                .await?;
            if invoice.total != payment.amount || invoice.amount_due != payment.amount {
                InvoiceRow::delete(conn, invoice.id, operation.tenant_id).await?;
                credit_inputs::bump_customer(conn, lock).await?;
                return Ok(None);
            }
            invoice
        };
        PaymentTransactionRowPatch {
            id: payment.id,
            invoice_id: Some(Some(invoice.id)),
            ..Default::default()
        }
        .patch(conn, operation.tenant_id, payment.id)
        .await?;
        CustomerCreditOperationRow::start_attempt(
            conn,
            operation.id,
            payment.id,
            invoice.id,
            payment.amount,
            &payment.currency,
            operation.metadata.clone(),
        )
        .await?;
        Ok(Some(invoice.id))
    }

    /// One-off invoice with a single `Credit` line, finalized only once the charge is accepted so
    /// a declined attempt consumes no invoice number.
    async fn draft_purchase_invoice(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        origin: PurchaseOrigin,
        customer: &Customer,
        invoice_date: chrono::NaiveDate,
        account: &CustomerCreditAccountRow,
        credit_minor: i64,
        sale: Option<&UnitSale>,
        captured_total: Option<i64>,
        expires_at: Option<DateTime<Utc>>,
    ) -> StoreResult<Invoice> {
        credit_inputs::bump_customer(conn, lock).await?;
        let (currency, line) = match (sale, captured_total) {
            (Some(sale), Some(total)) => (
                sale.currency.clone(),
                sale.line_at_total(origin.document_name(), credit_minor, total, invoice_date)?,
            ),
            (Some(sale), None) => (
                sale.currency.clone(),
                sale.line(origin.document_name(), credit_minor, invoice_date, None)?,
            ),
            (None, _) => {
                let currency = normalized_currency(&account.currency);
                let precision = crate::constants::Currencies::resolve_currency_precision(&currency)
                    .ok_or_else(|| {
                        precondition(format!("unsupported top-up currency {currency}"))
                    })?;
                let line = crate::domain::LineItem {
                    name: origin.document_name().to_owned(),
                    description: Some(format!("Prepaid credit added to the {currency} wallet")),
                    ..credit_purchase_line(credit_minor, precision, invoice_date, None)
                };
                (currency, line)
            }
        };
        let invoice = self
            .create_oneoff_draft_invoice(
                conn,
                customer.tenant_id,
                invoice_date,
                vec![line],
                customer,
                currency,
                None,
                None,
                None,
                None,
                None,
                Some(origin.document_name().to_owned()),
                // Credit purchases are priced before tax; only a captured total is carved.
                captured_total.is_some(),
            )
            .await?
            .ok_or_else(|| precondition("the top-up purchase document was not created"))?;
        // Minted under the invoice's purchase row, like a sold credit pack: the only way this
        // code base carries an absolute (not a from-funding) credit expiry to the grant.
        if let Some(at) = expires_at {
            CreditPurchaseRow::insert(
                conn,
                &CreditPurchaseRowNew {
                    expires_at: Some(at),
                    id: uuid::Uuid::new_v4(),
                    created_at: None,
                    amount_cents: invoice.total - invoice.tax_amount,
                    amount_credits: sale.is_some().then_some(credit_minor),
                    negotiated_unit_price: sale
                        .filter(|sale| sale.source == UnitPriceSource::Override)
                        .map(|sale| sale.unit_price),
                    unit_price_source: sale.map(|sale| sale.source.as_db_str().to_owned()),
                    note: None,
                    invoice_id: invoice.id,
                    tenant_id: customer.tenant_id,
                    customer_id: customer.id,
                    created_by: None,
                },
            )
            .await?;
        }
        Ok(invoice)
    }
}

/// How a replayed automatic top-up ended: what the card's gateway answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayedTopUp {
    Settled {
        invoice_id: InvoiceId,
    },
    Declined {
        reason: String,
        decline_kind: Option<DeclineKind>,
    },
    /// The gateway has not answered for good; the executor's reclaimer follows it up.
    Pending {
        payment_transaction_id: PaymentTransactionId,
    },
}

/// A replayed top-up armed and committed, before its card is charged.
struct ArmedReplay {
    customer_id: CustomerId,
    operation_id: CreditOperationId,
    payment_transaction_id: PaymentTransactionId,
    invoice_id: InvoiceId,
    payment_method_id: CustomerPaymentMethodId,
    charge_minor: i64,
    currency: String,
}

enum ReplayStart {
    Recorded(ReplayedTopUp),
    Armed(ArmedReplay),
}

impl Services {
    /// A sandbox replay's automatic top-up at `at`, through the executor's anchor, document and
    /// payment, keyed by (wallet, time, amount) so a retry never charges again. The card is
    /// charged outside any transaction: whatever the gateway answers is recorded.
    pub(in crate::services) async fn replay_automatic_top_up(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
        amount_minor: i64,
        at: DateTime<Utc>,
        delivery: crate::services::InvoiceDelivery,
    ) -> StoreResult<ReplayedTopUp> {
        if at > Utc::now() {
            return Err(invalid("a top-up cannot be charged at a future moment"));
        }
        if amount_minor <= 0 {
            return Err(invalid("a top-up charges a positive amount"));
        }
        let services = self.clone();
        let armed = match self
            .store
            .transaction(move |conn| {
                async move {
                    services
                        .arm_replayed_top_up_tx(conn, tenant_id, wallet_id, amount_minor, at)
                        .await
                }
                .scope_boxed()
            })
            .await?
        {
            ReplayStart::Recorded(outcome) => return Ok(outcome),
            ReplayStart::Armed(armed) => armed,
        };
        let intent = {
            let mut conn = self.store.get_conn().await?;
            super::replay_top_up::charge_card_off_session(
                &self.store,
                &mut conn,
                tenant_id,
                armed.payment_method_id,
                armed.payment_transaction_id,
                armed.charge_minor,
                &armed.currency,
            )
            .await?
        };
        let intent = PaymentIntent {
            processed_at: (intent.status != DomainPaymentStatus::Pending).then_some(at.naive_utc()),
            ..intent
        };
        let services = self.clone();
        self.store
            .transaction(move |conn| {
                async move {
                    services
                        .record_replayed_top_up_tx(conn, tenant_id, &armed, intent, at, delivery)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    async fn arm_replayed_top_up_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        wallet_id: WalletId,
        amount_minor: i64,
        at: DateTime<Utc>,
    ) -> StoreResult<ReplayStart> {
        let account = CustomerCreditAccountRow::find_by_id(conn, tenant_id, wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        let lock = lock_customer_tx(conn, tenant_id, account.customer_id).await?;
        let account = CustomerCreditAccountRow::lock_by_id(conn, &lock, wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        if account.credit_unit_id.is_some() {
            return Err(precondition(
                "a past automatic top-up is replayed on a wallet held in money",
            ));
        }
        let idempotency_key = format!("replay:{wallet_id}:{}:{amount_minor}", at.timestamp());
        if let Some(operation) = CustomerCreditOperationRow::find_idempotent(
            conn,
            tenant_id,
            account.customer_id,
            AUTOMATIC_TOP_UP_NAMESPACE,
            &idempotency_key,
        )
        .await?
        {
            return recorded_replay(conn, &operation)
                .await
                .map(ReplayStart::Recorded);
        }
        let customer: Customer = CustomerRow::find_by_id(conn, &account.customer_id, &tenant_id)
            .await?
            .try_into()?;
        let rule = self
            .store
            .hooks
            .credit
            .automatic_top_up(conn, account.id)
            .await?
            .and_then(AutomaticTopUp::chargeable);
        if let Some(rule) = &rule {
            ensure_rule_amount(rule, amount_minor)?;
        }
        let payment_method_id = rule
            .as_ref()
            .and_then(|rule| rule.payment_method_id)
            .or(customer.current_payment_method_id)
            .ok_or_else(|| precondition("the automatic top-up has no card to charge"))?;
        let currency = normalized_currency(&account.currency);
        let operation = CustomerCreditOperationRowNew {
            id: CreditOperationId::new(),
            tenant_id,
            customer_id: account.customer_id,
            account_id: account.id,
            state: state::CREATED.to_owned(),
            namespace: AUTOMATIC_TOP_UP_NAMESPACE.to_owned(),
            request_fingerprint: Sha256::digest(
                format!("automatic-top-up:replay:{tenant_id}:{idempotency_key}").as_bytes(),
            )
            .to_vec(),
            idempotency_key,
            amount_minor,
            expected_consideration_minor: None,
            consideration_currency: Some(currency.clone()),
            reason: "Automatic top-up triggered by low balance".to_owned(),
            metadata: json!({
                "payment_method_id": payment_method_id.to_string(),
                "attempts": 0,
            }),
        }
        .insert_conflict_do_nothing(conn)
        .await?
        .ok_or_else(|| precondition("a top-up of this wallet is already in flight"))?;
        let invoice = self
            .draft_purchase_invoice(
                conn,
                &lock,
                PurchaseOrigin::Automatic,
                &customer,
                at.date_naive(),
                &account,
                amount_minor,
                None,
                None,
                None,
            )
            .await?;
        let charge_minor = purchase_charge(&invoice, amount_minor, None).ok_or_else(|| {
            precondition("the top-up purchase document does not match its charge")
        })?;
        let payment_transaction_id = PaymentTransactionId::new();
        PaymentTransactionRowNew {
            id: payment_transaction_id,
            tenant_id,
            invoice_id: Some(invoice.id),
            provider_transaction_id: None,
            amount: charge_minor,
            currency: invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PurchaseOrigin::Automatic.payment_type(),
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(conn)
        .await?;
        CustomerCreditOperationRow::start_attempt(
            conn,
            operation.id,
            payment_transaction_id,
            invoice.id,
            charge_minor,
            &invoice.currency,
            increment_attempts(operation.metadata.clone())?,
        )
        .await?;
        Ok(ReplayStart::Armed(ArmedReplay {
            customer_id: account.customer_id,
            operation_id: operation.id,
            payment_transaction_id,
            invoice_id: invoice.id,
            payment_method_id,
            charge_minor,
            currency: invoice.currency,
        }))
    }

    async fn record_replayed_top_up_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        armed: &ArmedReplay,
        intent: PaymentIntent,
        at: DateTime<Utc>,
        delivery: crate::services::InvoiceDelivery,
    ) -> StoreResult<ReplayedTopUp> {
        let lock = lock_customer_and_payment_tx(
            conn,
            tenant_id,
            armed.customer_id,
            Some(armed.payment_transaction_id),
        )
        .await?;
        let operation =
            CustomerCreditOperationRow::lock_by_id(conn, &lock, armed.operation_id).await?;
        match intent.status {
            DomainPaymentStatus::Settled => {
                delivery.apply_tx(conn, tenant_id, armed.invoice_id).await?;
                self.finalize_invoice_at_tx(
                    conn,
                    &Actor::System,
                    armed.invoice_id,
                    tenant_id,
                    false,
                    &None,
                    crate::services::IssueAt::At(at),
                )
                .await?;
                let transaction = self
                    .store
                    .get_payment_tx_by_id_for_update(conn, armed.payment_transaction_id, tenant_id)
                    .await?;
                let transaction = self
                    .store
                    .consolidate_intent_and_transaction_tx(
                        conn,
                        &Actor::System,
                        transaction,
                        intent,
                    )
                    .await?;
                let event: PaymentTransactionEvent = transaction.into();
                self.on_invoice_payment_settled_tx(conn, &event, armed.invoice_id, at)
                    .await?;
                Ok(ReplayedTopUp::Settled {
                    invoice_id: armed.invoice_id,
                })
            }
            DomainPaymentStatus::Pending => {
                let transaction = self
                    .store
                    .get_payment_tx_by_id_for_update(conn, armed.payment_transaction_id, tenant_id)
                    .await?;
                self.store
                    .consolidate_intent_and_transaction_tx(
                        conn,
                        &Actor::System,
                        transaction,
                        intent,
                    )
                    .await?;
                Ok(ReplayedTopUp::Pending {
                    payment_transaction_id: armed.payment_transaction_id,
                })
            }
            _ => {
                let reason = intent
                    .last_payment_error
                    .clone()
                    .unwrap_or_else(|| intent_unsettled_reason(&intent));
                self.fail_purchase_at_tx(
                    conn,
                    &lock,
                    &operation,
                    reason.clone(),
                    intent.decline_kind,
                    at,
                    true,
                )
                .await?;
                Ok(ReplayedTopUp::Declined {
                    reason: truncate_error(reason),
                    decline_kind: intent.decline_kind,
                })
            }
        }
    }
}

/// What a replayed top-up already recorded under its key.
async fn recorded_replay(
    conn: &mut PgConn,
    operation: &CustomerCreditOperationRow,
) -> StoreResult<ReplayedTopUp> {
    match operation.state.as_str() {
        state::SETTLED => Ok(ReplayedTopUp::Settled {
            invoice_id: operation
                .invoice_id
                .ok_or_else(|| precondition("a settled top-up has no purchase document"))?,
        }),
        state::FAILED => {
            let decline_kind = match operation.payment_transaction_id {
                Some(id) => PaymentTransactionRow::get_by_id(conn, id, operation.tenant_id)
                    .await?
                    .decline_kind
                    .map(DeclineKind::from),
                None => None,
            };
            Ok(ReplayedTopUp::Declined {
                reason: operation
                    .metadata
                    .get("last_error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                decline_kind,
            })
        }
        _ => Ok(ReplayedTopUp::Pending {
            payment_transaction_id: operation
                .payment_transaction_id
                .ok_or_else(|| precondition("a top-up in flight has no payment"))?,
        }),
    }
}

/// A replayed top-up charges what its rule would: the fixed amount, or within its bounds.
fn ensure_rule_amount(rule: &AutomaticTopUpConfig, amount_minor: i64) -> StoreResult<()> {
    let allowed = match rule.method {
        AutomaticTopUpMethod::Fixed => rule.amount_minor == Some(amount_minor),
        AutomaticTopUpMethod::ToTarget => {
            rule.target_minor
                .is_some_and(|target| amount_minor <= target)
                && rule.minimum_minor.is_none_or(|min| amount_minor >= min)
                && rule.maximum_minor.is_none_or(|max| amount_minor <= max)
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(invalid(format!(
            "the automatic top-up rule would not charge {amount_minor}"
        )))
    }
}

/// Settles a top-up once its payment is captured and its credit funded, in the invoice-paid
/// transaction, behind its invoice's (so its customer's) lock, as of `now`: the moment the
/// settlement is recorded at, which the caps and the cooldown read. Idempotent: an anchor that is
/// not in flight is left as it is.
pub(crate) async fn complete_purchase_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    payment_transaction_id: PaymentTransactionId,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    let Some(operation) = CustomerCreditOperationRow::find_by_payment_transaction_for_update(
        conn,
        lock,
        payment_transaction_id,
    )
    .await?
    else {
        return Ok(());
    };
    let Some(origin) = PurchaseOrigin::of(&operation) else {
        return Ok(());
    };
    if operation.state != state::PROCESSING {
        return Ok(());
    }
    let invoice_id = operation
        .invoice_id
        .ok_or_else(|| precondition("a settled top-up has no purchase document"))?;
    let funded = CustomerCreditGrantRow::lock_by_invoice(conn, lock, invoice_id)
        .await?
        .into_iter()
        .filter(|grant| grant.source_kind == source::CREDIT_PURCHASE)
        .try_fold(0_i64, |total, grant| {
            if grant.account_id != operation.account_id {
                return Err(precondition(
                    "the top-up funded a different wallet than it was bought for",
                ));
            }
            total
                .checked_add(grant.amount)
                .ok_or_else(|| invalid("the top-up credit exceeds the supported range"))
        })?;
    // A CreditToBalance note issued while the payment was in flight returned its share to the
    // balance already: the purchase funds only the rest, as minted on this payment.
    let invoice: Invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
        .await?
        .try_into()?;
    let (bought, net_of_notes) = super::mint::purchased_funding_tx(conn, &invoice).await?;
    if bought != operation.amount_minor || funded != net_of_notes {
        return Err(precondition(format!(
            "the top-up funded {funded} of {net_of_notes} due on {bought} bought, but {} was ordered",
            operation.amount_minor
        )));
    }
    CustomerCreditOperationRow::settle_at(conn, operation.id, operation.metadata.clone(), now)
        .await?;
    if origin == PurchaseOrigin::SelfService {
        diesel_models::checkout_sessions::CheckoutSessionRow::complete_top_up(
            conn,
            tenant_id,
            super::top_up_checkout::session_id(
                tenant_id,
                operation.customer_id,
                &operation.idempotency_key,
            ),
        )
        .await?;
    }
    // The PROCESSING -> SETTLED edge records the rule's success exactly once.
    if origin == PurchaseOrigin::Automatic {
        let account = CustomerCreditAccountRow::lock_by_id(conn, lock, operation.account_id)
            .await?
            .ok_or_else(|| precondition("the automatic top-up wallet is missing"))?;
        controls
            .automatic_purchase_settled(conn, &account, now)
            .await?;
    }
    Ok(())
}

/// Locks a top-up's customer, then its payment when it has one: what every top-up path holds
/// before its operation.
async fn lock_customer_and_payment_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment_transaction_id: Option<PaymentTransactionId>,
) -> StoreResult<CustomerLock> {
    let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
    if let Some(id) = payment_transaction_id {
        PaymentTransactionRow::get_by_id_for_update(conn, &lock, id).await?;
    }
    Ok(lock)
}

/// The customer, the payment and the top-up the payment currently funds, locked in that order;
/// None when no top-up tracks this payment (the payment is locked behind its customer then).
async fn lock_top_up_by_payment_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    payment_transaction_id: PaymentTransactionId,
) -> StoreResult<
    Option<(
        CustomerLock,
        PaymentTransactionRow,
        CustomerCreditOperationRow,
    )>,
> {
    let Some(seen) = CustomerCreditOperationRow::find_by_payment_transaction(
        conn,
        tenant_id,
        payment_transaction_id,
    )
    .await?
    else {
        PaymentTransactionRow::lock_with_customer(conn, tenant_id, payment_transaction_id).await?;
        return Ok(None);
    };
    let lock = lock_customer_tx(conn, tenant_id, seen.customer_id).await?;
    let payment =
        PaymentTransactionRow::get_by_id_for_update(conn, &lock, payment_transaction_id).await?;
    Ok(
        CustomerCreditOperationRow::find_by_payment_transaction_for_update(
            conn,
            &lock,
            payment_transaction_id,
        )
        .await?
        .map(|operation| (lock, payment, operation)),
    )
}

/// A capture of a charge whose key was re-armed since: the anchor tracks a newer attempt, so this
/// money must not be funded a second time.
async fn detect_superseded_capture_tx(
    conn: &mut PgConn,
    event: &PaymentTransactionEvent,
) -> StoreResult<Option<ReissuedPurchase>> {
    let superseded = CustomerCreditOperationRow::find_by_superseded_payment(
        conn,
        event.tenant_id,
        event.payment_transaction_id,
    )
    .await?
    .filter(|operation| PurchaseOrigin::of(operation).is_some());
    Ok(superseded.map(|_| ReissuedPurchase::Detached))
}

/// A top-up that cannot be prepared for a lasting reason (retired card, missing customer, invalid
/// terms); a database or connectivity failure is retried as it is.
fn preparation_failure(error: &Report<StoreError>) -> Option<String> {
    match error.current_context() {
        StoreError::ValueNotFound(what) => Some(format!(
            "no payment method or customer usable for the automatic top-up ({what} not found)"
        )),
        StoreError::InvalidArgument(message)
        | StoreError::FailedPrecondition(message)
        | StoreError::Forbidden(message) => Some(format!(
            "the automatic top-up could not be prepared: {message}"
        )),
        _ => None,
    }
}

/// A self-service top-up of a credit unit wallet, priced and pinned when its key is first used.
struct PinnedTopUp {
    sale: UnitSale,
    pin: Value,
    quote: TopUpWalletQuote,
}

fn new_top_up_anchor(
    command: &TopUpWallet,
    fingerprint: &[u8],
    pinned: Option<PinnedTopUp>,
) -> StoreResult<CustomerCreditOperationRowNew> {
    let operation_id = CreditOperationId::new();
    let mut metadata = json!({
        "payment_method_id": command.payment_method_id.to_string(),
        "expires_at": command.expires_at.map(|at| at.to_rfc3339()),
        "attempts": 0,
    });
    let (consideration, consideration_currency) = match pinned {
        Some(PinnedTopUp { sale, pin, quote }) => {
            metadata_object(&mut metadata)?.insert(UNIT_SALE_KEY.to_owned(), pin);
            (Some(quote.charge_minor), Some(sale.currency))
        }
        None => (None, None),
    };
    Ok(CustomerCreditOperationRowNew {
        id: operation_id,
        tenant_id: command.tenant_id,
        customer_id: command.customer_id,
        account_id: command.wallet_id,
        state: state::CREATED.to_owned(),
        namespace: MANUAL_TOP_UP_NAMESPACE.to_owned(),
        idempotency_key: command.idempotency_key.trim().to_owned(),
        request_fingerprint: fingerprint.to_vec(),
        amount_minor: command.amount_minor,
        expected_consideration_minor: consideration,
        consideration_currency,
        reason: "Self-service wallet top-up".to_owned(),
        metadata,
    })
}

/// What the purchase document charges: money credit is untaxed and charges its own amount, a
/// unit sale its pinned price plus tax. None when the document does not match the purchase.
fn purchase_charge(invoice: &Invoice, credit_minor: i64, sale: Option<&UnitSale>) -> Option<i64> {
    let matches = match sale {
        None => {
            invoice.total == credit_minor
                && invoice.amount_due == credit_minor
                && invoice.tax_amount == 0
        }
        Some(sale) => {
            sale.net_minor(credit_minor).ok() == Some(invoice.subtotal)
                && invoice.amount_due == invoice.total
                && invoice.total > 0
        }
    };
    matches.then_some(invoice.total)
}

fn armed_from_operation(
    operation: &CustomerCreditOperationRow,
    payment_transaction_id: PaymentTransactionId,
    invoice_id: InvoiceId,
) -> StoreResult<ArmedPurchase> {
    Ok(ArmedPurchase {
        origin: PurchaseOrigin::of(operation)
            .ok_or_else(|| precondition("the operation is not a top-up"))?,
        tenant_id: operation.tenant_id,
        customer_id: operation.customer_id,
        wallet_id: operation.account_id,
        operation_id: operation.id,
        payment_transaction_id,
        invoice_id,
        charge_minor: operation
            .expected_consideration_minor
            .ok_or_else(|| precondition("the top-up attempt has no charge amount"))?,
        currency: operation
            .consideration_currency
            .clone()
            .ok_or_else(|| precondition("the top-up attempt has no charge currency"))?,
        credit_minor: operation.amount_minor,
    })
}

/// The purchase stays fixed; a declined checkout may retry with another card. A negotiated unit
/// price is appended only when set, so a key armed before it existed still matches.
fn request_fingerprint(command: &TopUpWallet) -> Vec<u8> {
    let mut request = format!(
        "manual-top-up:v3:{}:{}:{}:{}:{}",
        command.tenant_id,
        command.customer_id,
        command.wallet_id,
        command.amount_minor,
        command
            .expires_at
            .map(|at| at.to_rfc3339())
            .unwrap_or_default(),
    );
    if let Some(price) = command.unit_price {
        request.push_str(&format!(":unit-price:{}", price.normalize()));
    }
    Sha256::digest(request.as_bytes()).to_vec()
}

/// Rebuilds the receipt of a recorded top-up, so a retry returns the original charge.
async fn replay_receipt(
    conn: &mut PgConn,
    operation: &CustomerCreditOperationRow,
) -> StoreResult<TopUpWalletReceipt> {
    let payment_transaction_id = operation
        .payment_transaction_id
        .ok_or_else(|| precondition("the recorded top-up has no payment"))?;
    let invoice_id = operation
        .invoice_id
        .ok_or_else(|| precondition("the recorded top-up has no purchase document"))?;
    let armed = armed_from_operation(operation, payment_transaction_id, invoice_id)?;
    let transaction: PaymentTransaction =
        PaymentTransactionRow::get_by_id(conn, payment_transaction_id, operation.tenant_id)
            .await?
            .into();
    Ok(TopUpWalletReceipt {
        transaction,
        next_action: None,
        invoice_id,
        wallet_id: armed.wallet_id,
        charge_minor: armed.charge_minor,
        currency: armed.currency,
        credit_minor: armed.credit_minor,
    })
}

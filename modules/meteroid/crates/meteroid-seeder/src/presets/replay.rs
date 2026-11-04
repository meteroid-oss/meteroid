//! The credit history replay: every customer's `History::events`, merged into one time order,
//! run through the engine's time-explicit entry points while the tenant's lifecycle is held, so
//! each balance comes from the invoices, payments and grants the history records. Expired credit
//! is swept before each event; wallet rules are configured once the history is in place, and the
//! books are then checked against `History::expected`.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Months, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, ConnectorId, CreditGrantId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, EntitlementEntityId, InvoiceId, SubscriptionId, TenantId, WalletId,
};
use common_utils::decimals::ToSubunit;
use meteroid_store::domain::credit::{
    AutoTopUpChange, AutomaticTopUp, AutomaticTopUpConfig, AutomaticTopUpMethod,
    ConfigureCreditWallet, CreditGrantSource, CustomerCreditGrant, GrantClassification,
};
use meteroid_store::domain::credit_units::CreditUnit;
use meteroid_store::domain::entitlements::{EntitlementValue, OverageBehavior, ResetPeriod};
use meteroid_store::domain::enums::{
    DeclineKind, InvoicePaymentStatus, InvoiceStatusEnum, PaymentMethodTypeEnum, PaymentStatusEnum,
    SubscriptionActivationCondition,
};
use meteroid_store::domain::{
    CreateSubscription, CustomerConnection, CustomerPatch, CustomerPaymentMethod,
    CustomerPaymentMethodNew, EntitlementNew, FullPlan, Invoice, LineItemKind,
    PaymentMethodsConfig, SubscriptionNew, SubscriptionPatch,
};
use meteroid_store::errors::StoreError;
use meteroid_store::hooks::SeededBudget;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::{
    CustomerCreditWalletsInterface, EntitlementsInterface, InvoiceInterface, SubscriptionInterface,
};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::services::{InvoiceDelivery, ReplayedTopUp, SellCustomerCredit};
use meteroid_store::{Services, Store, StoreResult};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::presets::credits::HistoryUsage;
use crate::presets::scenarios::domain::{
    AutoTopUpAmount, AutoTopUpSpec, BudgetSpec, CardSpec, Collection, Customer,
    EntitlementOverrideSpec, FeeType, Plan, Scenario,
};
use crate::presets::scenarios::timeline::{
    Action, CardHandle, ChargeOutcome, Clock, DeclineReason, GrantRef, History, InvoiceExpectation,
    Payable, resolve_events,
};

/// Everything the replay needs once the catalog, customers, gateway and usage are in place.
pub(super) struct HistoryReplay<'a> {
    pub store: &'a Store,
    pub services: &'a Services,
    pub tenant_id: TenantId,
    pub scenario: &'a Scenario,
    pub plans: &'a HashMap<String, FullPlan>,
    pub credit_units: &'a HashMap<String, CreditUnit>,
    pub connector_id: ConnectorId,
    pub customers: &'a HashMap<String, CustomerId>,
    /// Each customer's clock (its `as_of` and billing day) and ingested usage windows.
    pub usage: &'a [HistoryUsage],
}

const BANK_TRANSFER_REFERENCE: &str = "Bank transfer";

/// Replays every credit customer's history. The lifecycle worker and automatic top-ups leave the
/// tenant alone while its seed is in progress. A book that does not reconcile fails the seed; each
/// mismatch is logged.
pub(super) async fn replay_history(replay: HistoryReplay<'_>) -> StoreResult<()> {
    let started = std::time::Instant::now();
    let outcome = match Replayer::new(&replay) {
        Ok(replayer) => Box::pin(replayer.run()).await,
        Err(error) => Err(error),
    };
    log::info!(
        "sandbox credit history of tenant {} replayed in {:?}",
        replay.tenant_id,
        started.elapsed()
    );
    outcome
}

/// One customer as the replay goes: what it subscribed to, its cards, wallets and documents.
struct Account<'a> {
    customer: &'a Customer,
    history: &'a History,
    customer_id: CustomerId,
    clock: Clock,
    plan: Option<&'a Plan>,
    subscription: Option<SubscriptionId>,
    /// The first period's invoice, issued as the subscription started, until a bill names it.
    first_bill: Option<(NaiveDateTime, Option<Invoice>)>,
    cards: HashMap<CardHandle, CustomerPaymentMethod>,
    default_card: Option<CustomerPaymentMethodId>,
    wallets: HashMap<&'static str, WalletId>,
    payables: HashMap<Payable, InvoiceId>,
    bills: Vec<Payable>,
    /// Applied once the history is in place, so no rule acts on a balance replayed out of time.
    wallet_rules: Vec<Action>,
    declined_top_ups: Vec<(NaiveDateTime, Payable, &'static str, Decimal, DeclineReason)>,
    /// When the latest automatic top-up of the history settled, which its rule carries.
    last_top_up: Option<NaiveDateTime>,
    auto_top_up: Option<AutomaticTopUp>,
}

impl Account<'_> {
    fn name(&self) -> &str {
        &self.customer.name
    }

    fn payable(&self, handle: Payable) -> StoreResult<InvoiceId> {
        self.payables.get(handle).copied().ok_or_else(|| {
            StoreError::InvalidArgument(format!("{} has no document {handle}", self.name())).into()
        })
    }

    fn handle_of(&self, invoice_id: InvoiceId) -> Option<Payable> {
        self.payables
            .iter()
            .find(|(_, id)| **id == invoice_id)
            .map(|(handle, _)| *handle)
    }

    fn default_card(&self) -> StoreResult<CustomerPaymentMethodId> {
        self.default_card.ok_or_else(|| {
            StoreError::InvalidArgument(format!("{} has no default card", self.name())).into()
        })
    }
}

/// A grant's origin as the history names it, owned so the engine's grants can be compared to it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    PlanCredit { invoice: String, component: String },
    PlanBonus { invoice: String, component: String },
    Rollover { invoice: String, component: String },
    Purchase(String),
    PromoCode(String),
    Added(String),
    Unknown(String),
}

impl From<&GrantRef> for Source {
    fn from(grant: &GrantRef) -> Self {
        match grant {
            GrantRef::PlanCredit { invoice, component } => Source::PlanCredit {
                invoice: (*invoice).to_string(),
                component: (*component).to_string(),
            },
            GrantRef::PlanBonus { invoice, component } => Source::PlanBonus {
                invoice: (*invoice).to_string(),
                component: (*component).to_string(),
            },
            GrantRef::Rollover { invoice, component } => Source::Rollover {
                invoice: (*invoice).to_string(),
                component: (*component).to_string(),
            },
            GrantRef::Purchase(handle) => Source::Purchase((*handle).to_string()),
            GrantRef::PromoCode(code) => Source::PromoCode((*code).to_string()),
            GrantRef::Added(reason) => Source::Added((*reason).to_string()),
        }
    }
}

type GrantLine = (Source, Decimal, Option<NaiveDateTime>);

struct Replayer<'r, 'a> {
    replay: &'r HistoryReplay<'a>,
    accounts: Vec<Account<'r>>,
    mismatches: Vec<String>,
}

impl<'r, 'a> Replayer<'r, 'a> {
    fn new(replay: &'r HistoryReplay<'a>) -> StoreResult<Self> {
        let mut accounts = Vec::new();
        for customer in &replay.scenario.customers {
            let Some(history) = &customer.history else {
                continue;
            };
            let customer_id = *replay.customers.get(&customer.name).ok_or_else(|| {
                StoreError::ValueNotFound(format!("Customer was not found {}", customer.name))
            })?;
            let clock = replay
                .usage
                .iter()
                .find(|usage| usage.customer_id == customer_id)
                .map(|usage| usage.clock)
                .ok_or_else(|| {
                    StoreError::ValueNotFound(format!("no usage history for {}", customer.name))
                })?;
            accounts.push(Account {
                customer,
                history,
                customer_id,
                clock,
                plan: None,
                subscription: None,
                first_bill: None,
                cards: HashMap::new(),
                default_card: None,
                wallets: HashMap::new(),
                payables: HashMap::new(),
                bills: Vec::new(),
                wallet_rules: Vec::new(),
                declined_top_ups: Vec::new(),
                last_top_up: None,
                auto_top_up: None,
            });
        }
        Ok(Replayer {
            replay,
            accounts,
            mismatches: Vec::new(),
        })
    }

    fn store(&self) -> &'r Store {
        self.replay.store
    }

    fn services(&self) -> &'r Services {
        self.replay.services
    }

    fn tenant_id(&self) -> TenantId {
        self.replay.tenant_id
    }

    fn mismatch(&mut self, account: usize, what: String) {
        let message = format!("{}: {what}", self.accounts[account].name());
        log::error!(
            "sandbox credit replay of tenant {}: {message}",
            self.tenant_id()
        );
        self.mismatches.push(message);
    }

    /// One time order across customers, so invoice numbers follow business time and two replays
    /// of the same scenario number every document alike.
    fn timeline(&self) -> Vec<(NaiveDateTime, usize, Action)> {
        let mut events: Vec<(NaiveDateTime, usize, usize, Action)> = self
            .accounts
            .iter()
            .enumerate()
            .flat_map(|(index, account)| {
                resolve_events(account.history, &account.clock)
                    .into_iter()
                    .enumerate()
                    .map(move |(position, (at, action))| (at, index, position, action))
            })
            .collect();
        events.sort_by_key(|(at, index, position, _)| (*at, *index, *position));
        events
            .into_iter()
            .map(|(at, index, _, action)| (at, index, action))
            .collect()
    }

    async fn run(mut self) -> StoreResult<()> {
        let timeline = self.timeline();
        // The catalog's promotion codes exist from the start of the history.
        if let Some((first, _, _)) = timeline.first() {
            crate::presets::credits::create_promo_codes(
                self.services(),
                self.tenant_id(),
                &self.replay.scenario.promo_codes,
                *first,
            )
            .await?;
        }
        for (at, index, action) in timeline {
            let label = format!(
                "{} at {at}: {}",
                self.accounts[index].name(),
                describe(&action)
            );
            log::debug!(
                "sandbox credit replay of tenant {}: {label}",
                self.tenant_id()
            );
            self.sweep(index, at)
                .await
                .map_err(|e| e.attach(label.clone()))?;
            Box::pin(self.apply(index, at, action))
                .await
                .map_err(|e| e.attach(label))?;
        }
        let as_of = self.accounts.first().map(|account| account.clock.as_of);
        for index in 0..self.accounts.len() {
            let name = self.accounts[index].name().to_string();
            if let Some(as_of) = as_of {
                self.sweep(index, as_of).await?;
            }
            self.configure_wallets(index)
                .await
                .map_err(|e| e.attach(format!("{name}: wallet rules")))?;
            self.charge_automatically(index)
                .await
                .map_err(|e| e.attach(format!("{name}: automatic charging")))?;
        }
        for index in 0..self.accounts.len() {
            self.check_books(index).await?;
        }
        if self.mismatches.is_empty() {
            return Ok(());
        }
        Err(StoreError::FailedPrecondition(format!(
            "the replayed credit history of tenant {} does not reconcile with its books ({} \
             mismatches): {}",
            self.tenant_id(),
            self.mismatches.len(),
            self.mismatches.join("; ")
        ))
        .into())
    }

    async fn sweep(&self, index: usize, at: NaiveDateTime) -> StoreResult<()> {
        self.store()
            .expire_customer_credit_for(
                self.tenant_id(),
                self.accounts[index].customer_id,
                at.and_utc(),
            )
            .await
            .map(|_| ())
    }

    async fn wallet(&mut self, index: usize, denomination: &'static str) -> StoreResult<WalletId> {
        if let Some(wallet) = self.accounts[index].wallets.get(denomination) {
            return Ok(*wallet);
        }
        let wallet = self
            .store()
            .create_customer_credit_wallet(
                self.tenant_id(),
                self.accounts[index].customer_id,
                denomination,
            )
            .await?
            .id;
        self.accounts[index].wallets.insert(denomination, wallet);
        Ok(wallet)
    }

    async fn apply(&mut self, index: usize, at: NaiveDateTime, action: Action) -> StoreResult<()> {
        let tenant_id = self.tenant_id();
        let customer_id = self.accounts[index].customer_id;
        let business_time = at.and_utc();
        match action {
            Action::Subscribe {
                plan,
                ends,
                billing_day_anchor,
            } => {
                let (created, invoice) = self
                    .subscribe(index, at, plan, ends, billing_day_anchor)
                    .await?;
                let account = &mut self.accounts[index];
                account.subscription = Some(created);
                account.first_bill = Some((at, invoice));
            }
            Action::AddCard {
                card,
                spec,
                make_default,
            } => {
                let method = attach_card(
                    self.store(),
                    tenant_id,
                    self.replay.connector_id,
                    customer_id,
                    &spec,
                    self.accounts[index].clock.as_of,
                )
                .await?;
                if make_default {
                    set_default_payment_method(self.store(), tenant_id, customer_id, method.id)
                        .await?;
                    self.accounts[index].default_card = Some(method.id);
                }
                self.accounts[index].cards.insert(card, method);
            }
            Action::RemoveCard { card } => {
                let method = self.accounts[index]
                    .cards
                    .remove(card)
                    .ok_or_else(|| StoreError::InvalidArgument(format!("unknown card {card}")))?;
                self.store()
                    .detach_payment_method_by_external_id(
                        tenant_id,
                        &method.external_payment_method_id,
                    )
                    .await?;
                if self.accounts[index].default_card == Some(method.id) {
                    self.accounts[index].default_card = None;
                }
            }
            Action::SetDefaultCard { card } => {
                let method = self.accounts[index]
                    .cards
                    .get(card)
                    .map(|m| m.id)
                    .ok_or_else(|| StoreError::InvalidArgument(format!("unknown card {card}")))?;
                set_default_payment_method(self.store(), tenant_id, customer_id, method).await?;
                self.accounts[index].default_card = Some(method);
            }
            Action::BillCycle { invoice, expected } => {
                let issued = match self.accounts[index].first_bill.take() {
                    Some((started_at, first)) if started_at == at => first,
                    _ => {
                        let subscription = self.accounts[index].subscription.ok_or_else(|| {
                            StoreError::InvalidArgument("a bill before any subscription".into())
                        })?;
                        self.services()
                            .bill_subscription_period_at(
                                tenant_id,
                                subscription,
                                business_time,
                                InvoiceDelivery::Suppressed,
                            )
                            .await?
                            .map(|detailed| detailed.invoice)
                    }
                };
                if let Some(issued) = &issued {
                    self.accounts[index].payables.insert(invoice, issued.id);
                    self.accounts[index].bills.push(invoice);
                }
                self.check_invoice(index, invoice, at, issued.as_ref(), &expected)
                    .await?;
            }
            Action::Purchase {
                purchase,
                denomination,
                amount,
                expires_in_months,
            } => {
                self.sell(index, at, purchase, denomination, amount, expires_in_months)
                    .await?;
            }
            Action::AutoTopUp {
                purchase,
                denomination,
                amount,
                expires_in_months,
                outcome,
            } => match outcome {
                ChargeOutcome::Paid => {
                    if expires_in_months.is_some() {
                        return Err(StoreError::InvalidArgument(format!(
                            "automatic top-up {purchase} expires, but automatic top-ups add \
                             credit that never does"
                        ))
                        .into());
                    }
                    let wallet_id = self.wallet(index, denomination).await?;
                    let outcome = self
                        .services()
                        .replay_automatic_top_up_at(
                            tenant_id,
                            wallet_id,
                            minor(amount, denomination, self.replay.credit_units)?,
                            business_time,
                            InvoiceDelivery::Suppressed,
                        )
                        .await?;
                    match outcome {
                        ReplayedTopUp::Settled { invoice_id } => {
                            let account = &mut self.accounts[index];
                            account.payables.insert(purchase, invoice_id);
                            account.last_top_up = Some(at);
                        }
                        other => self.mismatch(
                            index,
                            format!("automatic top-up {purchase} at {at} was not paid: {other:?}"),
                        ),
                    }
                }
                ChargeOutcome::Declined(reason) => {
                    self.accounts[index].declined_top_ups.push((
                        at,
                        purchase,
                        denomination,
                        amount,
                        reason,
                    ));
                }
            },
            Action::Charge { payable, outcome } => {
                self.charge(index, at, payable, outcome).await?;
            }
            Action::BankTransfer { payable } => {
                let invoice_id = self.accounts[index].payable(payable)?;
                self.services()
                    .settle_invoice_payment_at(
                        Actor::System,
                        tenant_id,
                        invoice_id,
                        business_time,
                        Some(BANK_TRANSFER_REFERENCE.to_string()),
                        InvoiceDelivery::Suppressed,
                    )
                    .await?;
            }
            Action::RedeemPromoCode { code } => {
                self.services()
                    .redeem_coupon_credit_code_at(tenant_id, customer_id, code, business_time)
                    .await?;
            }
            Action::AddCredit {
                denomination,
                amount,
                reason,
                promotional,
                expires,
            } => {
                let wallet_id = self.wallet(index, denomination).await?;
                let clock = self.accounts[index].clock;
                self.store()
                    .fund_customer_credit_wallet_at(
                        FundCustomerCreditWallet {
                            tenant_id,
                            customer_id,
                            wallet_id,
                            amount_minor: minor(amount, denomination, self.replay.credit_units)?,
                            classification: if promotional {
                                GrantClassification::Promotional
                            } else {
                                GrantClassification::Paid
                            },
                            priority: DEFAULT_PRIORITY,
                            expires_at: expires.map(|when| clock.resolve(when).and_utc()),
                            idempotency_key: Uuid::now_v7(),
                            reason: reason.to_string(),
                            reference: None,
                            actor_id: None,
                            cost: None,
                        },
                        business_time,
                    )
                    .await?;
            }
            rule @ Action::ConfigureWallet { .. } => {
                self.accounts[index].wallet_rules.push(rule);
            }
            Action::SetEntitlementOverride(spec) => {
                let feature = self
                    .store()
                    .get_feature_by_id_or_code(AliasOr::Alias(spec.feature_code.clone()), tenant_id)
                    .await?;
                self.store()
                    .create_entitlement(EntitlementNew {
                        tenant_id,
                        feature_id: feature.id,
                        entity: EntitlementEntityId::Customer(customer_id),
                        value: entitlement_override(&spec),
                    })
                    .await?;
            }
            Action::SetBudget(spec) => {
                let currency = self.accounts[index].customer.currency.clone();
                self.store()
                    .hooks
                    .credit
                    .seed_budget(
                        self.services(),
                        budget(
                            tenant_id,
                            customer_id,
                            &currency,
                            &spec,
                            self.replay.credit_units,
                        )?,
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn subscribe(
        &mut self,
        index: usize,
        at: NaiveDateTime,
        plan_name: &str,
        ends: Option<crate::presets::scenarios::timeline::When>,
        billing_day_anchor: Option<u16>,
    ) -> StoreResult<(SubscriptionId, Option<Invoice>)> {
        let plan = self.replay.plans.get(plan_name).ok_or_else(|| {
            StoreError::ValueNotFound(format!("Plan was not found : {plan_name}"))
        })?;
        let spec = self
            .replay
            .scenario
            .plans
            .iter()
            .find(|p| p.name == plan_name)
            .ok_or_else(|| {
                StoreError::ValueNotFound(format!("Plan was not found : {plan_name}"))
            })?;
        let clock = self.accounts[index].clock;
        let payment_methods_config = match spec.payment.collection {
            Collection::ChargeAutomatically => None,
            Collection::SendInvoice {
                bank_transfer: true,
            } => Some(PaymentMethodsConfig::bank_transfer()),
            Collection::SendInvoice {
                bank_transfer: false,
            } => Some(PaymentMethodsConfig::external()),
        };
        let subscription = CreateSubscription {
            subscription: SubscriptionNew {
                customer_id: self.accounts[index].customer_id,
                plan_version_id: plan.version.id,
                net_terms: None,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: at.date(),
                end_date: ends.map(|when| clock.resolve(when).date()),
                billing_start_date: None,
                activation_condition: SubscriptionActivationCondition::OnStart,
                trial_duration: None,
                billing_day_anchor,
                payment_methods_config,
                auto_advance_invoices: true,
                // Turned on once the history is replayed: its charges are the history's own.
                charge_automatically: false,
                purchase_order: None,
                backdate_invoices: true,
                skip_checkout_session: false,
                skip_past_invoices: false,
                usage_filter: None,
                custom_properties: serde_json::Value::Null,
            },
            price_components: None,
            add_ons: None,
            coupons: None,
            entitlements: vec![],
            minimum_override: None,
            drop_minimum: false,
        };
        let (created, invoice) = self
            .services()
            .insert_subscription_started_at(
                Actor::System,
                subscription,
                self.tenant_id(),
                at.and_utc(),
                InvoiceDelivery::Suppressed,
            )
            .await?;
        self.accounts[index].plan = Some(spec);
        Ok((created.id, invoice.map(|detailed| detailed.invoice)))
    }

    async fn sell(
        &mut self,
        index: usize,
        at: NaiveDateTime,
        purchase: Payable,
        denomination: &'static str,
        amount: Decimal,
        expires_in_months: Option<u32>,
    ) -> StoreResult<()> {
        let wallet_id = self.wallet(index, denomination).await?;
        let sold = self
            .services()
            .sell_customer_credit_at(
                Uuid::now_v7(),
                SellCustomerCredit {
                    expires_at: None,
                    tenant_id: self.tenant_id(),
                    customer_id: self.accounts[index].customer_id,
                    wallet_id,
                    amount_minor: minor(amount, denomination, self.replay.credit_units)?,
                    expires_in_months,
                    note: None,
                    actor: Actor::System,
                    unit_price: None,
                    expected_unit_price: None,
                },
                at.and_utc(),
                InvoiceDelivery::Suppressed,
            )
            .await?;
        self.accounts[index]
            .payables
            .insert(purchase, sold.invoice.invoice.id);
        Ok(())
    }

    /// Charges the default card through the gateway, whose answer must be the history's.
    async fn charge(
        &mut self,
        index: usize,
        at: NaiveDateTime,
        payable: Payable,
        outcome: ChargeOutcome,
    ) -> StoreResult<()> {
        let invoice_id = self.accounts[index].payable(payable)?;
        let card = self.accounts[index].default_card()?;
        let transaction = self
            .services()
            .charge_invoice_at(
                self.tenant_id(),
                invoice_id,
                card,
                at.and_utc(),
                InvoiceDelivery::Suppressed,
            )
            .await?;
        let charged = match transaction.status {
            PaymentStatusEnum::Settled => Some(ChargeOutcome::Paid),
            PaymentStatusEnum::Failed => match transaction.decline_kind {
                Some(DeclineKind::CardExpired) => {
                    Some(ChargeOutcome::Declined(DeclineReason::CardExpired))
                }
                Some(kind) if kind.is_hard() => {
                    Some(ChargeOutcome::Declined(DeclineReason::HardDecline))
                }
                _ => None,
            },
            _ => None,
        };
        if charged != Some(outcome) {
            self.mismatch(
                index,
                format!(
                    "charging {payable} at {at}: expected {outcome:?}, the gateway answered {:?} \
                     ({:?})",
                    transaction.status, transaction.decline_kind
                ),
            );
        }
        Ok(())
    }

    async fn configure_wallets(&mut self, index: usize) -> StoreResult<()> {
        let tenant_id = self.tenant_id();
        let customer_id = self.accounts[index].customer_id;
        let rules = std::mem::take(&mut self.accounts[index].wallet_rules);
        // Wallet rules are usage controls: an edition without them seeds none.
        let rules = if self.store().hooks.credit.available() {
            rules
        } else {
            Vec::new()
        };
        for rule in rules {
            let Action::ConfigureWallet {
                denomination,
                low_balance_thresholds,
                usage_at_zero,
                auto_top_up,
            } = rule
            else {
                continue;
            };
            let wallet_id = self.wallet(index, denomination).await?;
            let mut command = configure_wallet(
                tenant_id,
                customer_id,
                wallet_id,
                denomination,
                &low_balance_thresholds,
                usage_at_zero,
                auto_top_up.as_ref(),
                self.replay.credit_units,
            )?;
            // Configured once the history is in place, the rule still carries its last success.
            if let AutoTopUpChange::Set(config) = &mut command.auto_top_up {
                config.last_succeeded_at = self.accounts[index].last_top_up.map(|at| at.and_utc());
            }
            let (_, stored) = self.services().configure_credit_wallet(command).await?;
            self.accounts[index].auto_top_up = stored;
        }
        let declined = std::mem::take(&mut self.accounts[index].declined_top_ups);
        for (at, purchase, denomination, amount, reason) in declined {
            let wallet_id = self.wallet(index, denomination).await?;
            let outcome = self
                .services()
                .replay_automatic_top_up_at(
                    tenant_id,
                    wallet_id,
                    minor(amount, denomination, self.replay.credit_units)?,
                    at.and_utc(),
                    InvoiceDelivery::Suppressed,
                )
                .await?;
            let as_expected = match (&outcome, reason) {
                (
                    ReplayedTopUp::Declined {
                        decline_kind: Some(DeclineKind::CardExpired),
                        ..
                    },
                    DeclineReason::CardExpired,
                ) => true,
                (
                    ReplayedTopUp::Declined {
                        decline_kind: Some(kind),
                        ..
                    },
                    DeclineReason::HardDecline,
                ) => kind.is_hard() && *kind != DeclineKind::CardExpired,
                _ => false,
            };
            if !as_expected {
                self.mismatch(
                    index,
                    format!(
                        "automatic top-up {purchase} at {at}: expected {reason:?}, got {outcome:?}"
                    ),
                );
            }
            self.accounts[index].auto_top_up = self
                .services()
                .credit_wallet_auto_top_up(tenant_id, customer_id, wallet_id)
                .await?;
        }
        Ok(())
    }

    async fn charge_automatically(&self, index: usize) -> StoreResult<()> {
        let account = &self.accounts[index];
        let (Some(subscription), Some(plan)) = (account.subscription, account.plan) else {
            return Ok(());
        };
        if plan.payment.collection != Collection::ChargeAutomatically {
            return Ok(());
        }
        self.store()
            .patch_subscription(
                self.tenant_id(),
                SubscriptionPatch {
                    id: subscription,
                    charge_automatically: Some(true),
                    auto_advance_invoices: None,
                    net_terms: None,
                    invoice_memo: None,
                    purchase_order: None,
                    payment_methods_config: None,
                    usage_filter: None,
                    custom_properties: None,
                },
                Actor::System,
            )
            .await?;
        Ok(())
    }

    fn precision(&self, denomination: &str) -> StoreResult<u32> {
        match self.replay.credit_units.get(denomination) {
            Some(unit) => Ok(u32::from(unit.precision)),
            None => meteroid_money::iso::find(denomination)
                .map(|c| c.exponent)
                .ok_or_else(|| {
                    StoreError::InvalidArgument(format!("unknown denomination {denomination}"))
                        .into()
                }),
        }
    }

    fn major(&self, amount_minor: i64, denomination: &str) -> StoreResult<Decimal> {
        Ok(Decimal::new(amount_minor, self.precision(denomination)?))
    }

    async fn grants(&self, index: usize) -> StoreResult<Vec<CustomerCreditGrant>> {
        let (grants, total) = self
            .store()
            .customer_credit_grants(
                self.tenant_id(),
                self.accounts[index].customer_id,
                None,
                None,
                None,
                1_000,
                0,
            )
            .await?;
        if total > grants.len() as u64 {
            return Err(StoreError::FailedPrecondition(format!(
                "{} holds more grants than the replay reads",
                self.accounts[index].name()
            ))
            .into());
        }
        Ok(grants)
    }

    /// The history's name for `grant`'s origin.
    fn source_of(
        &self,
        index: usize,
        grant: &CustomerCreditGrant,
        all: &[CustomerCreditGrant],
    ) -> Source {
        let account = &self.accounts[index];
        let unknown = || {
            Source::Unknown(format!(
                "{:?} grant {} ({})",
                grant.grant.source, grant.grant.id, grant.grant.reason
            ))
        };
        let document = grant.grant.invoice_id.and_then(|id| account.handle_of(id));
        match grant.grant.source {
            CreditGrantSource::Purchase | CreditGrantSource::Included => {
                let Some(handle) = document else {
                    return unknown();
                };
                if !account.bills.contains(&handle) {
                    return Source::Purchase(handle.to_string());
                }
                let included = grant.grant.source == CreditGrantSource::Included;
                let bonus =
                    !included && grant.grant.classification == GrantClassification::Promotional;
                match pack_component(account.plan, included, &grant.currency) {
                    Some(component) if bonus => Source::PlanBonus {
                        invoice: handle.to_string(),
                        component,
                    },
                    Some(component) => Source::PlanCredit {
                        invoice: handle.to_string(),
                        component,
                    },
                    None => unknown(),
                }
            }
            CreditGrantSource::Rollover => {
                let origin = all
                    .iter()
                    .find(|other| other.grant.id.as_uuid() == grant.grant.source_id);
                match origin.map(|origin| self.source_of(index, origin, all)) {
                    Some(Source::PlanCredit { invoice, component }) => {
                        Source::Rollover { invoice, component }
                    }
                    _ => unknown(),
                }
            }
            CreditGrantSource::Coupon => match &grant.grant.reference {
                Some(code) => Source::PromoCode(code.clone()),
                None => unknown(),
            },
            CreditGrantSource::Manual => Source::Added(grant.grant.reason.clone()),
            _ => unknown(),
        }
    }

    async fn check_invoice(
        &mut self,
        index: usize,
        handle: Payable,
        at: NaiveDateTime,
        invoice: Option<&Invoice>,
        expected: &InvoiceExpectation,
    ) -> StoreResult<()> {
        let Some(invoice) = invoice else {
            if *expected != InvoiceExpectation::empty() {
                self.mismatch(
                    index,
                    format!("{handle} at {at}: no invoice was issued, {expected:?} was expected"),
                );
            }
            return Ok(());
        };
        let currency = invoice.currency.clone();
        let money = |amount: i64| self.major(amount, &currency);
        let mut advance = Decimal::ZERO;
        let mut usage = Decimal::ZERO;
        let mut overage = Decimal::ZERO;
        let mut shortfall = Decimal::ZERO;
        for line in &invoice.line_items {
            if line.kind == LineItemKind::MinimumShortfall {
                shortfall += money(line.amount_subtotal)?;
            } else if line.metric_id.is_none() {
                advance += money(line.amount_subtotal)?;
            } else if let Some(unit) = &line.credit_unit {
                usage += Decimal::new(unit.consumed, u32::from(unit.precision));
                overage += money(line.amount_subtotal)?;
            } else {
                usage += money(line.amount_subtotal)?;
            }
        }
        let tax = money(invoice.tax_amount)?;
        let due = money(invoice.amount_due)?;

        let grants = self.grants(index).await?;
        let mut applied: Vec<(Source, Decimal)> = Vec::new();
        for (grant_id, amount) in self
            .store()
            .invoice_credit_by_grant(self.tenant_id(), invoice.id)
            .await?
        {
            let line = self.applied_line(index, grant_id, amount, &grants)?;
            applied.push(line);
        }
        applied.sort();
        let mut wanted: Vec<(Source, Decimal)> = expected
            .applied
            .iter()
            .map(|(grant, amount)| (Source::from(grant), *amount))
            .collect();
        wanted.sort();

        let mut differences = Vec::new();
        let mut compare = |what: &str, billed: Decimal, wanted: Decimal| {
            if billed != wanted {
                differences.push(format!("{what} {billed} instead of {wanted}"));
            }
        };
        compare("advance", advance, expected.advance);
        compare("usage", usage, expected.usage);
        compare("overage", overage, expected.overage);
        compare("minimum shortfall", shortfall, expected.minimum_shortfall);
        compare("tax", tax, expected.tax);
        compare("amount due", due, expected.amount_due + expected.tax);
        if applied != wanted {
            differences.push(format!("credit applied {applied:?} instead of {wanted:?}"));
        }
        if invoice.finalized_at != Some(at) {
            differences.push(format!("issued at {:?}", invoice.finalized_at));
        }
        if !differences.is_empty() {
            self.mismatch(
                index,
                format!(
                    "{handle} (invoice {}) at {at}: {}",
                    invoice.invoice_number,
                    differences.join(", ")
                ),
            );
        }
        Ok(())
    }

    fn applied_line(
        &self,
        index: usize,
        grant_id: CreditGrantId,
        amount: i64,
        grants: &[CustomerCreditGrant],
    ) -> StoreResult<(Source, Decimal)> {
        let grant = grants
            .iter()
            .find(|g| g.grant.id == grant_id)
            .ok_or_else(|| StoreError::ValueNotFound(format!("credit grant {grant_id}")))?;
        Ok((
            self.source_of(index, grant, grants),
            self.major(amount, &grant.currency)?,
        ))
    }

    async fn check_books(&mut self, index: usize) -> StoreResult<()> {
        let tenant_id = self.tenant_id();
        let grants = self.grants(index).await?;
        let mut live: BTreeMap<String, Vec<GrantLine>> = BTreeMap::new();
        for grant in grants.iter().filter(|g| g.grant.remaining_minor > 0) {
            let line = (
                self.source_of(index, grant, &grants),
                self.major(grant.grant.remaining_minor, &grant.currency)?,
                grant.grant.expires_at.map(|at| at.naive_utc()),
            );
            live.entry(grant.currency.clone()).or_default().push(line);
        }
        let account = &self.accounts[index];
        let expected = &account.history.expected;
        let clock = account.clock;
        let mut wanted: BTreeMap<String, Vec<GrantLine>> = BTreeMap::new();
        let mut on_account: Vec<(String, Decimal)> = Vec::new();
        for wallet in &expected.wallets {
            on_account.push((wallet.denomination.to_string(), wallet.on_account));
            wanted.insert(
                wallet.denomination.to_string(),
                wallet
                    .grants
                    .iter()
                    .map(|g| {
                        (
                            Source::from(&g.source),
                            g.remaining,
                            g.expires.and_then(|e| e.resolve(&clock)),
                        )
                    })
                    .collect(),
            );
        }
        for lines in live.values_mut().chain(wanted.values_mut()) {
            lines.sort();
        }
        let mut differences = Vec::new();
        if live != wanted {
            differences.push(format!("credit grants {live:?} instead of {wanted:?}"));
        }
        for (denomination, amount) in on_account {
            let held: Decimal = live
                .get(&denomination)
                .map(|lines| lines.iter().map(|(_, remaining, _)| *remaining).sum())
                .unwrap_or_default();
            if held != amount {
                differences.push(format!(
                    "{held} {denomination} on account instead of {amount}"
                ));
            }
        }
        let suspended = account
            .auto_top_up
            .as_ref()
            .is_some_and(|rule| rule.config.suspended_at.is_some());
        if suspended != expected.auto_top_up_suspended {
            differences.push(format!(
                "automatic top-up suspended: {suspended} instead of {}",
                expected.auto_top_up_suspended
            ));
        }
        if expected.usage_paused_at.is_some() {
            let blocks = self
                .store()
                .customer_credit_wallets(tenant_id, account.customer_id)
                .await?
                .iter()
                .any(|wallet| wallet.pauses_at_zero());
            if !blocks {
                differences.push("usage is not stopped once credit runs out".to_string());
            }
        }
        let mut open = Vec::new();
        let mut handles: Vec<(&Payable, &InvoiceId)> = account.payables.iter().collect();
        handles.sort_by_key(|(handle, _)| **handle);
        for (handle, invoice_id) in handles {
            let invoice = self
                .store()
                .get_detailed_invoice_by_id(tenant_id, *invoice_id)
                .await?
                .invoice;
            if invoice.status == InvoiceStatusEnum::Finalized
                && invoice.payment_status != InvoicePaymentStatus::Paid
                && invoice.amount_due > 0
            {
                open.push((handle.to_string(), invoice.due_at));
            }
        }
        let mut wanted_open: Vec<(String, Option<NaiveDateTime>)> = expected
            .open_invoices
            .iter()
            .map(|(handle, due)| (handle.to_string(), Some(clock.resolve(*due))))
            .collect();
        wanted_open.sort();
        if open != wanted_open {
            differences.push(format!("open invoices {open:?} instead of {wanted_open:?}"));
        }
        for difference in differences {
            self.mismatch(index, difference);
        }
        Ok(())
    }
}

/// The plan's credit component a pack grant comes from: paid packs are bought, included ones are
/// given, each in its own denomination.
fn pack_component(plan: Option<&Plan>, included: bool, denomination: &str) -> Option<String> {
    let plan = plan?;
    let mut candidates = plan.components.iter().filter(|component| {
        let FeeType::Credit {
            unit_price,
            credit_unit,
            ..
        } = &component.fee
        else {
            return false;
        };
        unit_price.is_zero() == included
            && credit_unit.as_deref().unwrap_or(plan.currency.as_str()) == denomination
    });
    let component = candidates.next()?;
    candidates.next().is_none().then(|| component.name.clone())
}

fn describe(action: &Action) -> String {
    match action {
        Action::Subscribe { plan, .. } => format!("subscribe to {plan}"),
        Action::AddCard { card, .. } => format!("add card {card}"),
        Action::RemoveCard { card } => format!("remove card {card}"),
        Action::SetDefaultCard { card } => format!("make {card} the default card"),
        Action::BillCycle { invoice, .. } => format!("bill {invoice}"),
        Action::Purchase { purchase, .. } => format!("purchase {purchase}"),
        Action::AutoTopUp { purchase, .. } => format!("automatic top-up {purchase}"),
        Action::Charge { payable, .. } => format!("charge {payable}"),
        Action::BankTransfer { payable } => format!("bank transfer for {payable}"),
        Action::RedeemPromoCode { code } => format!("redeem {code}"),
        Action::AddCredit { reason, .. } => format!("add credit: {reason}"),
        Action::ConfigureWallet { denomination, .. } => {
            format!("configure the {denomination} wallet")
        }
        Action::SetEntitlementOverride(spec) => format!("{} limit", spec.feature_code),
        Action::SetBudget(_) => "budget".to_string(),
    }
}

/// `amount` in minor units of a currency, or of a credit unit when `denomination` is one.
pub(super) fn minor(
    amount: Decimal,
    denomination: &str,
    units: &HashMap<String, CreditUnit>,
) -> StoreResult<i64> {
    let precision = match units.get(denomination) {
        Some(unit) => unit.precision,
        None => meteroid_money::iso::find(denomination)
            .map(|c| c.exponent as u8)
            .ok_or_else(|| {
                StoreError::InvalidArgument(format!("unknown denomination {denomination}"))
            })?,
    };
    amount
        .to_subunit_opt(precision)
        .ok_or_else(|| StoreError::InvalidDecimal.into())
}

/// Saves `card` on the mock gateway, where its id carries the outcome of every charge.
pub(super) async fn attach_card(
    store: &Store,
    tenant_id: TenantId,
    connector_id: ConnectorId,
    customer_id: CustomerId,
    card: &CardSpec,
    as_of: NaiveDateTime,
) -> StoreResult<CustomerPaymentMethod> {
    let connection = store
        .upsert_customer_connection(
            &tenant_id,
            CustomerConnection {
                id: CustomerConnectionId::new(),
                customer_id,
                connector_id,
                supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
                external_customer_id: format!("mock_cus_{}", customer_id.as_base62()),
            },
        )
        .await?;
    let (exp_month, exp_year) = card_expiry(as_of, card.expires_in_months);
    store
        .upsert_payment_method(CustomerPaymentMethodNew {
            id: CustomerPaymentMethodId::new(),
            tenant_id,
            customer_id,
            connection_id: connection.id,
            external_payment_method_id: format!(
                "mock_pm_{}_card_{}-{}_{}",
                card.behavior.gateway_code(),
                card.brand,
                card.last4,
                &Uuid::new_v4().simple().to_string()[..8]
            ),
            payment_method_type: PaymentMethodTypeEnum::Card,
            account_number_hint: None,
            card_brand: Some(card.brand.to_string()),
            card_last4: Some(card.last4.to_string()),
            card_exp_month: Some(exp_month),
            card_exp_year: Some(exp_year),
            fingerprint: None,
            saved_for_payment: false,
        })
        .await
}

fn card_expiry(as_of: NaiveDateTime, months: i32) -> (i32, i32) {
    let month = as_of.date().with_day(1).expect("the first of a month");
    let at = if months >= 0 {
        month + Months::new(months.unsigned_abs())
    } else {
        month - Months::new(months.unsigned_abs())
    };
    (at.month() as i32, at.year())
}

pub(super) async fn set_default_payment_method(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    method: CustomerPaymentMethodId,
) -> StoreResult<()> {
    store
        .patch_customer(
            Actor::System,
            tenant_id,
            CustomerPatch {
                id: customer_id,
                name: None,
                alias: None,
                billing_email: None,
                invoicing_emails: None,
                phone: None,
                currency: None,
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: None,
                current_payment_method_id: Some(Some(method)),
                tax_status: None,
                exemption_reason: None,
                connected_account_id: None,
                custom_properties: None,
                preferred_locales: None,
                customer_type: None,
                first_name: None,
                last_name: None,
            },
        )
        .await?;
    Ok(())
}

/// The automatic top-up rule as authored; `None` charges the customer's default card, so a later
/// default-card switch moves the rule to the new card.
pub(super) fn auto_top_up_config(
    spec: &AutoTopUpSpec,
    denomination: &str,
    units: &HashMap<String, CreditUnit>,
) -> StoreResult<AutomaticTopUpConfig> {
    let (method, amount, target) = match spec.amount {
        AutoTopUpAmount::Fixed(amount) => (
            AutomaticTopUpMethod::Fixed,
            Some(minor(amount, denomination, units)?),
            None,
        ),
        AutoTopUpAmount::ToTarget(target) => (
            AutomaticTopUpMethod::ToTarget,
            None,
            Some(minor(target, denomination, units)?),
        ),
    };
    let mut config = AutomaticTopUpConfig::new(
        method,
        minor(spec.threshold, denomination, units)?,
        amount,
        target,
        None,
    );
    config.daily_cap_minor = spec
        .daily_cap
        .map(|cap| minor(cap, denomination, units))
        .transpose()?;
    config.monthly_cap_minor = spec
        .monthly_cap
        .map(|cap| minor(cap, denomination, units))
        .transpose()?;
    Ok(config)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn configure_wallet(
    tenant_id: TenantId,
    customer_id: CustomerId,
    wallet_id: WalletId,
    denomination: &str,
    low_balance_thresholds: &[Decimal],
    usage_at_zero: meteroid_store::domain::credit_exhausted::UsageAtZero,
    auto_top_up: Option<&AutoTopUpSpec>,
    units: &HashMap<String, CreditUnit>,
) -> StoreResult<ConfigureCreditWallet> {
    Ok(ConfigureCreditWallet {
        tenant_id,
        customer_id,
        wallet_id,
        auto_top_up: match auto_top_up {
            Some(spec) => AutoTopUpChange::Set(auto_top_up_config(spec, denomination, units)?),
            None => AutoTopUpChange::Remove,
        },
        low_balance_thresholds_minor: Some(
            low_balance_thresholds
                .iter()
                .map(|t| minor(*t, denomination, units))
                .collect::<StoreResult<Vec<_>>>()?,
        ),
        usage_at_zero: Some(usage_at_zero),
        unit_price: meteroid_store::domain::credit::UnitPriceChange::Keep,
        actor_id: None,
        // The demo scenario stands for a customer who agreed to the automatic charges.
        consent_attested_by: Some(common_domain::actor::Actor::System),
    })
}

pub(super) fn entitlement_override(spec: &EntitlementOverrideSpec) -> EntitlementValue {
    EntitlementValue::Metered {
        limit: Some(spec.limit),
        reset_period: ResetPeriod::BillingCycle,
        overage_behavior: match &spec.block {
            Some(block) => OverageBehavior::Block {
                grace_period_pct: block.grace_pct,
            },
            None => OverageBehavior::Allow,
        },
        thresholds: spec.thresholds.clone(),
        enabled: true,
        group_by: spec.group_by.clone(),
    }
}

pub(super) fn budget(
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    spec: &BudgetSpec,
    units: &HashMap<String, CreditUnit>,
) -> StoreResult<SeededBudget> {
    Ok(SeededBudget {
        tenant_id,
        customer_id,
        currency: currency.to_string(),
        amount_minor: minor(spec.amount, currency, units)?,
        hard: spec.hard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn card_expiry_counts_months_from_the_seed() {
        let as_of = NaiveDate::from_ymd_opt(2026, 10, 8)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        assert_eq!(card_expiry(as_of, -3), (7, 2026));
        assert_eq!(card_expiry(as_of, 36), (10, 2029));
    }
}

//! Drives the real engine through the services and store entry points the scenario kit uses,
//! in business time, and reads back what the model is compared with.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, PaymentTransactionId, SubscriptionId, WalletId,
};
use diesel_models::customer_credit::{CustomerCreditEntryRow, CustomerCreditGrantRow};
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::PaymentTransactionRowNew;
use http::HeaderMap;
use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount, CouponNew};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{Invoice, PaginationRequest, UsageFilter};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::services::{InvoiceDelivery, OfflineRefundCommand, SellCustomerCredit};
use rust_decimal_macros::dec;

use super::reference::{
    Action, DOCUMENTS, GRACE, How, Late, LateUse, Model, Plan, Token, midnight,
};
use crate::scenarios::kit::{Halyard, LITE, M, MAX, STANDARD, Tax, at as day_at};

pub const CODE: &str = "MODEL100";

#[derive(Clone, Debug)]
enum Paid {
    Manual(String),
    Provider(String),
}

pub struct Engine {
    pub h: Halyard,
    pub customer: CustomerId,
    pub subs: Vec<SubscriptionId>,
    pub invoices: Vec<InvoiceId>,
    payments: Vec<Vec<Paid>>,
    wallet: WalletId,
    serial: u32,
}

/// What the model is compared with, in the model's terms.
#[derive(Debug, PartialEq, Eq)]
pub struct Observed {
    /// Mapped invoices: status, subtotal, discount, promotional credit, tax, total, applied, due.
    pub invoices: Vec<(InvoiceStatusEnum, [i64; 7])>,
    /// Invoices the engine issued that no action of the model mapped.
    pub unmapped: Vec<String>,
    pub notes: Vec<Vec<(String, i64)>>,
    pub grants: BTreeMap<String, i64>,
}

/// Everything a second pass of a worker could change.
#[derive(Debug, PartialEq, Eq)]
pub struct Fingerprint {
    invoices: Vec<String>,
    notes: Vec<String>,
    grants: Vec<(String, i64)>,
    entries: usize,
}

fn err<E: std::fmt::Debug>(e: E) -> String {
    let text = format!("{e:?}");
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

pub fn token_name(token: Token) -> &'static str {
    match token {
        Token::Lite => LITE,
        Token::Standard => STANDARD,
        Token::Max => MAX,
    }
}

pub fn sub_name(sub: usize) -> &'static str {
    ["A", "B"][sub]
}

fn key(sub: usize) -> String {
    format!("key-{}", sub_name(sub))
}

impl Engine {
    pub async fn new() -> Self {
        let h = Halyard::new().await;
        h.env
            .skip_credit_invariants("the model runner checks them after every step");
        h.skip_corpus("a model case's documents follow from its seed");
        let customer = h.customer("Model", Tax::Untaxed).await;
        let wallet = h
            .store()
            .create_customer_credit_wallet(h.tenant, customer, "USD")
            .await
            .expect("USD wallet")
            .id;
        h.services()
            .create_coupon_at(
                Actor::System,
                CouponNew {
                    code: CODE.to_owned(),
                    description: "Model credit".to_owned(),
                    tenant_id: h.tenant,
                    discount: CouponDiscount::Credit {
                        currency: "USD".to_owned(),
                        amount: dec!(100),
                        expires_in_days: Some(25),
                        scope: CouponCreditScope::Customer,
                    },
                    expires_at: None,
                    redemption_limit: None,
                    recurring_value: None,
                    reusable: false,
                    plan_ids: vec![],
                },
                day_at(1, 1),
            )
            .await
            .expect("the promotion code");
        Self {
            h,
            customer,
            subs: vec![],
            invoices: vec![],
            payments: vec![],
            wallet,
            serial: 0,
        }
    }

    fn plan(&self, plan: Plan) -> common_domain::ids::PlanVersionId {
        match plan {
            Plan::Payg => self.h.payg,
            Plan::Scale => self.h.scale,
        }
    }

    pub async fn sweep(&self, at: DateTime<Utc>) -> Result<(), String> {
        self.h
            .store()
            .expire_customer_credit_for(self.h.tenant, self.customer, at)
            .await
            .map(|_| ())
            .map_err(err)
    }

    async fn due_events(&self) -> Result<(), String> {
        for _ in 0..4 {
            let processed = self
                .h
                .services()
                .get_and_process_due_events()
                .await
                .map_err(err)?;
            if processed == 0 {
                break;
            }
        }
        Ok(())
    }

    fn record(&self, sub: usize, dated: DateTime<Utc>, token: Token, mtok: i64) {
        let tokens = u64::try_from(mtok).expect("positive") * M;
        self.h
            .keyed_tokens(self.customer, dated, token_name(token), tokens, &key(sub));
    }

    fn late(&self, sub: usize, day: chrono::NaiveDate, late: LateUse) {
        self.record(
            sub,
            midnight(day) - chrono::Duration::hours(1),
            late.token,
            late.mtok,
        );
    }

    /// Runs the action as the model does; `model` is the state before it. A refused action still
    /// ends its step: the clock moved, so the sweep and the workers run as after any other.
    pub async fn apply(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        let outcome = self.run(action, model).await;
        if !matches!(
            action,
            Action::Renew { .. } | Action::ChangeNow { .. } | Action::CancelNow { .. }
        ) {
            self.sweep(action.done()).await?;
        }
        self.drain(2).await;
        self.sync().await;
        outcome
    }

    async fn run(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        let h = &self.h;
        let tenant = h.tenant;
        match action {
            Action::Start { day, plan } => {
                let sub = self.subs.len();
                let mut request = h.request(self.customer, self.plan(*plan), *day);
                request.subscription.usage_filter =
                    Some(UsageFilter::new("api_key".to_owned(), key(sub)).map_err(err)?);
                let (created, _) = h
                    .services()
                    .insert_subscription_started_at(
                        Actor::System,
                        request,
                        tenant,
                        midnight(*day),
                        InvoiceDelivery::Suppressed,
                    )
                    .await
                    .map_err(err)?;
                self.subs.push(created.id);
            }
            Action::Usage {
                sub,
                dated,
                token,
                mtok,
                ..
            } => self.record(*sub, *dated, *token, *mtok),
            Action::Renew { at, subs } => {
                for &s in subs {
                    h.services()
                        .bill_subscription_period_at(
                            tenant,
                            self.subs[s],
                            *at,
                            InvoiceDelivery::Suppressed,
                        )
                        .await
                        .map_err(err)?;
                }
                self.due_events().await?;
                self.sweep(*at + DOCUMENTS).await?;
            }
            Action::TopUp { at, amount, pay } => {
                let sold = h
                    .services()
                    .sell_customer_credit_at(
                        uuid::Uuid::now_v7(),
                        SellCustomerCredit {
                            expires_at: None,
                            tenant_id: tenant,
                            customer_id: self.customer,
                            wallet_id: self.wallet,
                            amount_minor: *amount,
                            expires_in_months: None,
                            note: None,
                            actor: Actor::System,
                            unit_price: None,
                            expected_unit_price: None,
                        },
                        *at,
                        InvoiceDelivery::Suppressed,
                    )
                    .await
                    .map_err(err)?;
                self.sync().await;
                if *pay {
                    let invoice = self
                        .invoices
                        .iter()
                        .position(|id| *id == sold.invoice.invoice.id)
                        .ok_or("the purchase invoice is not mapped")?;
                    self.pay_manual(invoice, *at).await?;
                }
            }
            Action::Pay { at, invoice, how } => match how {
                How::Manual => self.pay_manual(*invoice, *at).await?,
                How::Provider(extra) => {
                    let amount = model.invoices[*invoice].due() + extra;
                    let external = self.collect(*invoice, amount, *at, true).await?;
                    self.payments[*invoice].push(Paid::Provider(external));
                }
            },
            Action::Fail { at, invoice } => {
                let amount = model.invoices[*invoice].due();
                self.collect(*invoice, amount, *at, false).await?;
            }
            Action::Chargeback {
                at,
                invoice,
                payment,
                disputed,
            } => {
                let Paid::Provider(external) = &self.payments[*invoice][*payment] else {
                    return Err("a manual payment cannot be charged back here".to_owned());
                };
                let kind = if *disputed {
                    "payment_charged_back"
                } else {
                    "payment_late_failure"
                };
                let payload = format!(
                    r#"{{"id":"evt_cb_{external}","kind":"{kind}","external_id":"{external}","occurred_at":"{}"}}"#,
                    at.to_rfc3339()
                );
                self.deliver(payload).await?;
            }
            Action::RefundNote {
                at,
                invoice,
                payment,
                amount,
            } => {
                let id = self.invoices[*invoice];
                let purchase = h.invoice(id).await;
                let note = h
                    .store()
                    .create_and_finalize_credit_note(
                        Actor::System,
                        tenant,
                        CreateCreditNoteParams {
                            invoice_id: id,
                            line_items: vec![CreditLineItem::Amount {
                                local_id: purchase.line_items[0].local_id.clone(),
                                amount_cents: *amount,
                            }],
                            reason: Some("Refund".into()),
                            memo: None,
                            credit_type: CreditType::Refund,
                            custom_properties: None,
                        },
                    )
                    .await
                    .map_err(err)?;
                let parent = self.transaction(*invoice, *payment).await?;
                h.services()
                    .record_offline_refund(
                        Actor::System,
                        tenant,
                        OfflineRefundCommand {
                            parent_transaction_id: parent,
                            amount: -note.total,
                            credit_note_id: Some(note.id),
                            refund_date: at.naive_utc(),
                            reference: Some(format!("refund-{}", note.id)),
                            reason: None,
                        },
                    )
                    .await
                    .map_err(err)?;
            }
            Action::RefundNoNote {
                at,
                invoice,
                payment,
                amount,
            } => {
                let parent = self.transaction(*invoice, *payment).await?;
                self.serial += 1;
                h.services()
                    .record_offline_refund(
                        Actor::System,
                        tenant,
                        OfflineRefundCommand {
                            parent_transaction_id: parent,
                            amount: *amount,
                            credit_note_id: None,
                            refund_date: at.naive_utc(),
                            reference: Some(format!("refund-{}", self.serial)),
                            reason: None,
                        },
                    )
                    .await
                    .map_err(err)?;
            }
            Action::Void { invoice, at } => {
                h.store()
                    .void_invoice_at(Actor::System, self.invoices[*invoice], tenant, *at)
                    .await
                    .map_err(err)?;
            }
            Action::Redeem { at } => {
                h.services()
                    .redeem_coupon_credit_code_at(tenant, self.customer, CODE, *at)
                    .await
                    .map_err(err)?;
            }
            Action::ChangeNow { sub, day, to, late } => {
                h.services()
                    .apply_plan_change_immediate_at(
                        Actor::System,
                        self.subs[*sub],
                        tenant,
                        self.plan(*to),
                        vec![],
                        *day,
                    )
                    .await
                    .map_err(err)?;
                self.after_ending(*sub, *day, *late, false).await?;
            }
            Action::ChangeAtEnd { sub, to, .. } => {
                h.services()
                    .schedule_plan_change(
                        Actor::System,
                        self.subs[*sub],
                        tenant,
                        self.plan(*to),
                        vec![],
                    )
                    .await
                    .map_err(err)?;
            }
            Action::CancelNow { sub, day, late } => {
                h.services()
                    .cancel_subscription(
                        Actor::System,
                        self.subs[*sub],
                        tenant,
                        None,
                        CancellationEffectiveAt::Date(*day),
                    )
                    .await
                    .map_err(err)?;
                self.after_ending(*sub, *day, *late, true).await?;
            }
            Action::CancelAtEnd { sub, end, .. } => {
                h.services()
                    .cancel_subscription(
                        Actor::System,
                        self.subs[*sub],
                        tenant,
                        None,
                        CancellationEffectiveAt::Date(*end),
                    )
                    .await
                    .map_err(err)?;
            }
        }
        Ok(())
    }

    /// Mirrors the model: a late event within the grace before the sweep that lapses the ending
    /// credit, one after it before the final invoice is issued.
    async fn after_ending(
        &self,
        sub: usize,
        day: chrono::NaiveDate,
        late: Option<LateUse>,
        cancelled: bool,
    ) -> Result<(), String> {
        let bound = midnight(day);
        if cancelled {
            self.due_events().await?;
        }
        if let Some(late) = late.filter(|l| l.when == Late::WithinGrace) {
            self.late(sub, day, late);
        }
        self.sweep(bound + GRACE).await?;
        if let Some(late) = late.filter(|l| l.when == Late::AfterGrace) {
            self.late(sub, day, late);
        }
        if cancelled {
            self.due_events().await?;
        }
        self.sweep(bound + DOCUMENTS).await
    }

    async fn pay_manual(&mut self, invoice: usize, at: DateTime<Utc>) -> Result<(), String> {
        self.serial += 1;
        let reference = format!("manual-{}", self.serial);
        self.h
            .services()
            .settle_invoice_payment_at(
                Actor::System,
                self.h.tenant,
                self.invoices[invoice],
                at,
                Some(reference.clone()),
                InvoiceDelivery::Suppressed,
            )
            .await
            .map_err(err)?;
        self.payments[invoice].push(Paid::Manual(reference));
        Ok(())
    }

    async fn deliver(&self, payload: String) -> Result<(), String> {
        let h = &self.h;
        let connector = h
            .store()
            .get_connector_with_data_by_alias("sandbox-gateway".to_owned(), h.tenant)
            .await
            .map_err(err)?;
        let provider = initialize_payment_connector(&connector).map_err(err)?;
        let event = provider
            .parse_event(payload.as_bytes(), &HeaderMap::new())
            .map_err(err)?
            .ok_or("no event")?;
        handle_normalized_event(
            event,
            &connector,
            provider.as_ref(),
            h.store().clone(),
            h.services(),
        )
        .await
        .map_err(err)?;
        self.drain(6).await;
        Ok(())
    }

    /// The outbox consumers read ten messages a pass: run after every step and drained around
    /// provider events, so a settlement never waits behind the events of earlier steps.
    async fn drain(&self, passes: usize) {
        for _ in 0..passes {
            self.h.env.run_outbox_and_orchestration().await;
        }
    }

    /// A provider payment of `amount` collected (or failed) at `at`; returns its external id.
    async fn collect(
        &mut self,
        invoice: usize,
        amount: i64,
        at: DateTime<Utc>,
        succeeds: bool,
    ) -> Result<String, String> {
        let payment = PaymentTransactionId::new();
        let external = format!("pi_{}", payment.as_base62());
        PaymentTransactionRowNew {
            id: payment,
            tenant_id: self.h.tenant,
            invoice_id: Some(self.invoices[invoice]),
            provider_transaction_id: Some(external.clone()),
            amount,
            currency: "USD".to_owned(),
            payment_method_id: None,
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
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
        .insert(&mut self.h.env.conn().await)
        .await
        .map_err(err)?;
        let payload = if succeeds {
            format!(
                r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{external}","amount":{amount},"currency":"USD","occurred_at":"{at}"}}"#,
                tx = payment.as_base62(),
                at = at.to_rfc3339(),
            )
        } else {
            format!(
                r#"{{"id":"evt_ko_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"{external}","occurred_at":"{at}"}}"#,
                tx = payment.as_base62(),
                at = at.to_rfc3339(),
            )
        };
        self.deliver(payload).await?;
        Ok(external)
    }

    async fn transaction(
        &self,
        invoice: usize,
        payment: usize,
    ) -> Result<PaymentTransactionId, String> {
        let reference = match &self.payments[invoice][payment] {
            Paid::Manual(r) | Paid::Provider(r) => r.clone(),
        };
        self.h
            .store()
            .list_payment_tx_by_invoice_id(self.h.tenant, self.invoices[invoice])
            .await
            .map_err(err)?
            .into_iter()
            .find(|tx| {
                tx.transaction.provider_transaction_id.as_deref() == Some(reference.as_str())
            })
            .map(|tx| tx.transaction.id)
            .ok_or_else(|| format!("no payment {reference}"))
    }

    async fn all_invoices(&self) -> Vec<Invoice> {
        let mut invoices: Vec<Invoice> = self
            .h
            .store()
            .list_invoices(
                self.h.tenant,
                Some(self.customer),
                None,
                None,
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_string()),
                PaginationRequest {
                    page: 0,
                    per_page: Some(1_000),
                },
            )
            .await
            .expect("invoices")
            .items
            .into_iter()
            .map(|i| i.invoice)
            .collect();
        invoices.sort_by_key(|i| (i.created_at, i.id.as_uuid()));
        invoices
    }

    /// Maps the invoices issued since, in creation order; closed periods with nothing billed
    /// are not documents of the model.
    pub async fn sync(&mut self) {
        for invoice in self.all_invoices().await {
            if invoice.status == InvoiceStatusEnum::Closed
                || invoice.status == InvoiceStatusEnum::Draft
                || self.invoices.contains(&invoice.id)
            {
                continue;
            }
            self.invoices.push(invoice.id);
            self.payments.push(vec![]);
        }
    }

    fn grant_key(&self, grant: &CustomerCreditGrantRow) -> String {
        let sub = grant
            .subscription_id
            .and_then(|s| self.subs.iter().position(|id| *id == s));
        let kind = match (
            grant.source_kind.as_str(),
            grant.classification.as_str(),
            sub.is_some(),
        ) {
            ("CREDIT_PURCHASE", "PAID", true) => "Pack",
            ("CREDIT_PURCHASE", "PROMOTIONAL", true) => "Bonus",
            ("CREDIT_PURCHASE", "PAID", false) => "TopUp",
            ("INCLUDED_CREDIT", _, _) => "Welcome",
            ("COUPON", _, _) => "Promo",
            ("CREDIT_NOTE", _, _) => "Note",
            ("OVERPAYMENT", _, _) => "Overpay",
            (other, classification, _) => {
                return format!("{other}/{classification}");
            }
        };
        match sub {
            Some(s) => format!("{kind}/{}", sub_name(s)),
            None => kind.to_owned(),
        }
    }

    pub async fn observe(&self) -> Observed {
        let all = self.all_invoices().await;
        let mut invoices = vec![];
        let mut notes = vec![];
        for id in &self.invoices {
            let invoice = all.iter().find(|i| i.id == *id).expect("a mapped invoice");
            invoices.push((
                invoice.status,
                [
                    invoice.subtotal,
                    invoice.discount,
                    invoice.promotional_credits,
                    invoice.tax_amount,
                    invoice.total,
                    invoice.applied_credits,
                    invoice.amount_due,
                ],
            ));
            let mut of_invoice: Vec<(String, i64)> = self
                .h
                .store()
                .list_credit_notes_by_invoice_id(self.h.tenant, *id)
                .await
                .expect("notes")
                .into_iter()
                .filter(|n| n.status == meteroid_store::domain::enums::CreditNoteStatus::Finalized)
                .map(|n| {
                    let kind = match n.credit_type {
                        CreditType::CreditToBalance => "Balance",
                        CreditType::Refund => "Refund",
                        CreditType::DebtCancellation => "Debt",
                    };
                    (kind.to_owned(), n.total)
                })
                .collect();
            of_invoice.sort();
            notes.push(of_invoice);
        }
        let unmapped = all
            .iter()
            .filter(|i| !self.invoices.contains(&i.id) && i.status != InvoiceStatusEnum::Closed)
            .map(|i| format!("{:?} {} due {}", i.status, i.total, i.amount_due))
            .collect();
        let mut grants = BTreeMap::new();
        for grant in self.h.grants(self.customer).await {
            if grant.remaining != 0 {
                *grants.entry(self.grant_key(&grant)).or_insert(0) += grant.remaining;
            }
        }
        Observed {
            invoices,
            unmapped,
            notes,
            grants,
        }
    }

    pub async fn fingerprint(&self) -> Fingerprint {
        let all = self.all_invoices().await;
        let mut notes = vec![];
        for invoice in &all {
            for note in self
                .h
                .store()
                .list_credit_notes_by_invoice_id(self.h.tenant, invoice.id)
                .await
                .expect("notes")
            {
                notes.push(format!("{} {:?} {}", note.id, note.status, note.total));
            }
        }
        let grants = self.h.grants(self.customer).await;
        let mut entries = 0;
        let mut conn = self.h.env.conn().await;
        for grant in &grants {
            entries += CustomerCreditEntryRow::list_by_grant(&mut conn, self.h.tenant, grant.id)
                .await
                .expect("entries")
                .len();
        }
        Fingerprint {
            invoices: all
                .iter()
                .map(|i| {
                    format!(
                        "{} {:?} {:?} {} {} {} {}",
                        i.id,
                        i.status,
                        i.payment_status,
                        i.total,
                        i.promotional_credits,
                        i.applied_credits,
                        i.amount_due
                    )
                })
                .collect(),
            notes,
            grants: grants
                .iter()
                .map(|g| (g.id.to_string(), g.remaining))
                .collect(),
            entries,
        }
    }

    /// Every sweep and worker a second time, at the same business instant: none may write.
    pub async fn rerun_workers(&self, action: &Action) -> Result<(), String> {
        let h = &self.h;
        self.sweep(action.done()).await?;
        h.services()
            .settle_missed_plan_change_endings(100)
            .await
            .map_err(err)?;
        h.store()
            .end_credit_of_ended_subscriptions(100)
            .await
            .map_err(err)?;
        h.store()
            .reconcile_reversed_credit_purchases(100)
            .await
            .map_err(err)?;
        self.due_events().await?;
        if let Action::Renew { at, subs } = action {
            for &s in subs {
                if let Ok(Some(invoice)) = h
                    .services()
                    .bill_subscription_period_at(
                        h.tenant,
                        self.subs[s],
                        *at,
                        InvoiceDelivery::Suppressed,
                    )
                    .await
                {
                    return Err(format!(
                        "renewing {} again at {at} returned invoice {}",
                        sub_name(s),
                        invoice.invoice.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// The ledger as stored, for a failure report: each grant, its source invoice and movements.
    pub async fn dump(&self) -> Vec<String> {
        let mut conn = self.h.env.conn().await;
        let mut lines = vec![];
        for grant in self.h.grants(self.customer).await {
            let source = grant
                .invoice_id
                .and_then(|id| self.invoices.iter().position(|i| *i == id))
                .map_or(String::new(), |i| format!(" from INV-{}", i + 1));
            let movements: Vec<String> =
                CustomerCreditEntryRow::list_by_grant(&mut conn, self.h.tenant, grant.id)
                    .await
                    .expect("entries")
                    .into_iter()
                    .map(|e| {
                        let invoice = e
                            .invoice_id
                            .and_then(|id| self.invoices.iter().position(|i| *i == id))
                            .map_or(String::new(), |i| format!(" INV-{}", i + 1));
                        format!(
                            "{} {}{invoice} @{}",
                            e.kind,
                            e.amount,
                            e.occurred_at.format("%m-%d %H:%M")
                        )
                    })
                    .collect();
            lines.push(format!(
                "{} {} {}{source} eff {} lapse {:?}: {}",
                self.grant_key(&grant),
                grant.amount,
                grant.remaining,
                grant.effective_at.format("%m-%d %H:%M"),
                grant.lapses_at.map(|l| l.format("%m-%d %H:%M").to_string()),
                movements.join(", ")
            ));
        }
        lines
    }

    pub async fn invariants(&self) -> Vec<String> {
        let mut conn = self.h.env.conn().await;
        crate::helpers::invariants::credit_invariant_violations(&mut conn, self.h.tenant).await
    }
}

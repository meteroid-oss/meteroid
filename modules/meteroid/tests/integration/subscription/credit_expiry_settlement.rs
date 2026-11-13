//! Credit that expires mid-period pays the usage it covered, once, and the invoice of the period
//! takes those settlements over instead of drawing again.
use std::sync::Arc;

use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId, InvoiceId, SubscriptionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::customer_credit::CreditUsageSettlementRow;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::UsagePeriod;
use meteroid_store::domain::credit::CreditGrantSource;
use meteroid_store::domain::credit::{CreditLedgerEntry, LedgerEntryKind};
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

use super::usage::{PLAN_VERSION_USAGE_ONLY_ID, seed_usage_only_plan};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::helpers::credit::{
    Grant, assert_ledger_balances, available, expire_grant, expire_grant_hours_ago, fund_grant,
    wallets,
};
use crate::meteroid_it::container::SeedLevel;

const GRANT: i64 = 5_000;

fn start_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

/// 0.10 EUR per unit: `units` units are `units * 10` cents.
fn set_period_usage(usage: &MutableMockUsageClient, units: i64) {
    let start = start_date().and_time(NaiveTime::MIN);
    let end = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: if units == 0 {
                vec![]
            } else {
                vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: std::collections::HashMap::new(),
                }]
            },
            period: UsagePeriod { start, end },
        },
    );
}

struct Scenario {
    env: TestEnv,
    usage: MutableMockUsageClient,
    subscription_id: SubscriptionId,
    grant: CreditGrantId,
}

/// A usage-only subscription whose first period is closed with an open draft, and a promotional
/// grant that expired hours ago, before the settlement grace, with the period's usage read.
async fn scenario(units: i64) -> Scenario {
    scenario_with(units, false).await
}

/// `usage_at_draft` reads the period's usage when its draft is created, so the draft carries a
/// usage line an operator can pin a price on.
async fn scenario_with(units: i64, usage_at_draft: bool) -> Scenario {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    if usage_at_draft {
        set_period_usage(&usage, units);
    }
    env.process_cycle_transitions().await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;
    expire_grant(env.store(), grant).await;
    set_period_usage(&usage, units);
    Scenario {
        env,
        usage,
        subscription_id,
        grant,
    }
}

impl Scenario {
    async fn sweep(&self) -> usize {
        self.env
            .store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep")
    }

    async fn settlements(&self) -> Vec<CreditUsageSettlementRow> {
        let mut conn = self.env.conn().await;
        CreditUsageSettlementRow::list_for_period(
            &mut conn,
            TENANT_ID,
            self.subscription_id,
            start_date(),
            "EUR",
        )
        .await
        .expect("settlements of the period")
    }

    async fn statement(&self) -> Vec<CreditLedgerEntry> {
        let wallet = wallets(self.env.store(), TENANT_ID, CUST_UBER_ID)
            .await
            .remove(0);
        let (mut entries, _) = self
            .env
            .store()
            .customer_credit_ledger(TENANT_ID, wallet.id, 100, 0)
            .await
            .expect("statement");
        entries.sort_by_key(|entry| entry.sequence);
        entries
    }

    async fn given_back_settlements(&self) -> Vec<CreditUsageSettlementRow> {
        use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::credit_usage_settlement::dsl as c;
        let mut conn = self.env.conn().await;
        c::credit_usage_settlement
            .filter(c::subscription_id.eq(self.subscription_id))
            .filter(c::returned_at.is_not_null())
            .select(CreditUsageSettlementRow::as_select())
            .load(&mut conn)
            .await
            .expect("settlements given back")
    }

    async fn entries(&self) -> Vec<CreditLedgerEntry> {
        self.entries_of(self.grant).await
    }

    async fn entries_of(&self, grant: CreditGrantId) -> Vec<CreditLedgerEntry> {
        let mut entries = self.statement().await;
        entries.retain(|entry| entry.grant_id == grant);
        entries
    }

    async fn amounts(&self, kind: LedgerEntryKind) -> Vec<i64> {
        self.amounts_of(self.grant, kind).await
    }

    async fn amounts_of(&self, grant: CreditGrantId, kind: LedgerEntryKind) -> Vec<i64> {
        self.entries_of(grant)
            .await
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.amount_minor)
            .collect()
    }

    async fn total(&self, kind: LedgerEntryKind) -> i64 {
        self.statement()
            .await
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.amount_minor)
            .sum()
    }

    async fn execute(&self, sql: &str) {
        use diesel_async::SimpleAsyncConnection;
        self.env
            .conn()
            .await
            .batch_execute(sql)
            .await
            .expect("test statement");
    }

    /// Prices the draft's usage lines at `price` per unit, as an operator editing the draft does.
    async fn pin_draft_price(&self, price: &str) {
        self.execute(&format!(
            "UPDATE invoice SET line_items = COALESCE((SELECT jsonb_agg(
                 CASE WHEN e->>'metric_id' IS NOT NULL
                      THEN jsonb_set(e, '{{unit_price}}', '\"{price}\"') ELSE e END)
                 FROM jsonb_array_elements(line_items) e), '[]'::jsonb)
             WHERE subscription_id = '{}' AND status = 'DRAFT'",
            self.subscription_id.as_uuid()
        ))
        .await;
    }

    async fn funded_sources(&self, grant: CreditGrantId) -> Vec<(CreditGrantSource, i64)> {
        self.env
            .store()
            .pgmq_read(
                PgmqQueue::WebhookOut,
                MessageReadQty(1000),
                MessageReadVtSec(0),
            )
            .await
            .expect("webhook queue")
            .into_iter()
            .filter_map(|message| message.message)
            .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
            .filter_map(|event| match event {
                OutboxEvent::WalletFunded(event) if event.credit_grant_id == grant => {
                    Some((event.source, event.amount_minor))
                }
                _ => None,
            })
            .collect()
    }

    fn age_settlements(&self, days: i64) {
        self.env.advance(chrono::Duration::days(days));
    }

    async fn finalize(&self) -> meteroid_store::domain::Invoice {
        self.env.process_due_events().await;
        let invoices = self.env.get_invoices(self.subscription_id).await;
        assert_eq!(invoices.len(), 1);
        invoices.into_iter().next().unwrap()
    }
}

#[tokio::test]
async fn a_trial_credit_expiring_mid_period_pays_the_usage_before_it_lapses() {
    let s = scenario(100).await;

    assert_eq!(s.sweep().await, 1);
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 1_000, "100 units at 0.10 EUR");
    assert_eq!(settlements[0].invoice_id, None, "not billed yet");
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-4_000],
        "only what the usage did not consume lapses"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.total, 1_000);
    assert_eq!(invoice.applied_credits, 1_000);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(
        s.amounts(LedgerEntryKind::Draw).await,
        vec![-1_000],
        "finalization takes the settlement over, it does not draw again"
    );
    assert_eq!(s.settlements().await[0].invoice_id, Some(invoice.id));
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A promotional grant and a non-expiring paid grant are both available when the promo expires
/// mid-period. The promo (expiring sooner, so first in the waterfall regardless of classification)
/// pays only the usage up to its own expiry; usage whose event time is after the expiry is not
/// covered by it and, at finalization, is paid from the paid grant that remains.
#[tokio::test]
async fn a_promotional_grant_pays_only_its_pre_expiry_usage_and_the_paid_grant_covers_the_rest() {
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::domain::credit::GrantClassification;
    use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
    use meteroid_store::repositories::customers::CustomersInterfaceAuto;

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;

    let promo = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;

    // January 2024 is closed with a draft still in its grace; the promo's term is moved strictly
    // inside it (absolute, unlike `expire_grant`). Grant terms are immutable, so the trigger is
    // bypassed for this one write, as the shared `expire_grant` helper does.
    let period_start = start_date().and_time(NaiveTime::MIN);
    let expiry = NaiveDate::from_ymd_opt(2024, 1, 15)
        .unwrap()
        .and_time(NaiveTime::MIN);
    let period_end = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            expiry.and_utc().to_rfc3339(),
            promo.as_uuid()
        ))
        .await
        .expect("move the promo's expiry inside the open period");

    let currency = env
        .store()
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .expect("customer")
        .currency;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &currency)
        .await
        .expect("open customer credit wallet");
    let paid = env
        .store()
        .fund_customer_credit_wallet(FundCustomerCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            wallet_id: wallet.id,
            amount_minor: 9_000,
            classification: GrantClassification::Paid,
            priority: 100,
            expires_at: None,
            idempotency_key: uuid::Uuid::now_v7(),
            reason: "Integration-test paid credit".to_owned(),
            reference: None,
            actor_id: None,
            cost: None,
        })
        .await
        .expect("fund paid credit")
        .grant_id;
    crate::helpers::credit::effective_from_fixture_start(env.store(), paid).await;

    let group = |value: i64| {
        vec![meteroid_store::clients::usage::GroupedUsageData {
            value: Decimal::from(value),
            dimensions: std::collections::HashMap::new(),
        }]
    };
    // 100 units (1000 cents) before the expiry, another 50 (500 cents) after it: the full period
    // totals 150 units, which is what finalization bills.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start,
            period_end: expiry,
        },
        UsageData {
            data: group(100),
            period: UsagePeriod {
                start: period_start,
                end: expiry,
            },
        },
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start,
            period_end,
        },
        UsageData {
            data: group(150),
            period: UsagePeriod {
                start: period_start,
                end: period_end,
            },
        },
    );

    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        1
    );
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        period_start.date(),
        "EUR",
    )
    .await
    .expect("settlements of the period");
    assert_eq!(settlements.len(), 1);
    assert_eq!(
        settlements[0].amount, 1_000,
        "the promo settles only the 100 pre-expiry units"
    );

    let wallet_row = wallets(env.store(), TENANT_ID, CUST_UBER_ID)
        .await
        .remove(0);
    let (entries, _) = env
        .store()
        .customer_credit_ledger(TENANT_ID, wallet_row.id, 100, 0)
        .await
        .expect("statement");
    let draws_of = |grant| {
        entries
            .iter()
            .filter(|e| e.grant_id == grant && e.kind == LedgerEntryKind::Draw)
            .map(|e| e.amount_minor)
            .collect::<Vec<_>>()
    };
    let expiries_of = |grant| {
        entries
            .iter()
            .filter(|e| e.grant_id == grant && e.kind == LedgerEntryKind::Expiry)
            .map(|e| e.amount_minor)
            .collect::<Vec<_>>()
    };
    assert_eq!(draws_of(promo), vec![-1_000]);
    assert_eq!(
        expiries_of(promo),
        vec![-4_000],
        "only what the usage did not consume lapses from the promo"
    );
    assert!(
        draws_of(paid).is_empty(),
        "the paid grant is untouched by the sweep: it only paid usage the promo could not"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = invoices.into_iter().next().unwrap();
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        invoice.total, 1_500,
        "150 units at 0.10 EUR for the whole period"
    );
    assert_eq!(invoice.applied_credits, 1_500);
    assert_eq!(
        invoice.amount_due, 0,
        "the post-expiry 50 units are paid from the paid grant, not left unpaid"
    );

    let (entries, _) = env
        .store()
        .customer_credit_ledger(TENANT_ID, wallet_row.id, 100, 0)
        .await
        .expect("statement");
    let draws_of = |grant| {
        entries
            .iter()
            .filter(|e| e.grant_id == grant && e.kind == LedgerEntryKind::Draw)
            .map(|e| e.amount_minor)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        draws_of(promo),
        vec![-1_000],
        "finalization takes the settlement over, it does not draw the expired promo again"
    );
    assert_eq!(
        draws_of(paid),
        vec![-500],
        "the paid grant pays exactly the usage whose event time is after the promo's expiry"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn a_retried_sweep_settles_and_lapses_nothing_twice() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);
    let entries = s.entries().await.len();

    assert_eq!(s.sweep().await, 0);
    assert_eq!(s.settlements().await.len(), 1);
    assert_eq!(s.entries().await.len(), entries);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn an_invoice_below_the_settled_amount_returns_the_excess_and_lapses_it() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);

    // The period ends up billing 40 units: 600 of the 1000 settled were not owed.
    set_period_usage(&s.usage, 40);
    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 400);
    assert_eq!(invoice.applied_credits, 400);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![600]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-4_000, -600],
        "the grant had expired, so the returned value lapses again"
    );
    let wallet = wallets(s.env.store(), TENANT_ID, CUST_UBER_ID)
        .await
        .remove(0);
    assert_eq!(wallet.available_minor, 0);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Events reaching the metering store more than 30 minutes after the expiry are not covered by
/// the expired credit: they are billed and paid from the credit valid at finalization.
#[tokio::test]
async fn usage_arriving_after_the_settlement_is_not_covered_by_the_expired_credit() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);

    set_period_usage(&s.usage, 150);
    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_500);
    assert_eq!(invoice.applied_credits, 1_000);
    assert_eq!(invoice.amount_due, 500);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// OQ15: the settlement grace is the invoicing entity's grace (here 24 hours), never under 30
/// minutes. Expired 13 hours ago, the credit waits; January's late events (50 units dated before
/// the expiry) arrive meanwhile, and once the grace has passed the sweep pays all 150 units from
/// it: 1500 settled, 3500 lapse, the invoice takes the settlement over and nothing is due.
#[tokio::test]
async fn a_late_event_inside_the_entity_grace_is_paid_by_the_expired_credit() {
    let s = scenario(100).await;
    s.env.set_consolidate_and_grace(false, 24).await;

    assert_eq!(
        s.sweep().await,
        0,
        "expired 13 hours ago, inside the entity's 24-hour grace"
    );
    assert!(s.settlements().await.is_empty());

    set_period_usage(&s.usage, 150);
    expire_grant_hours_ago(s.env.store(), s.grant, 25).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_500]);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-3_500]);

    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_500);
    assert_eq!(invoice.applied_credits, 1_500);
    assert_eq!(invoice.amount_due, 0);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A note inside the settlement grace gives value back to the expired credit, and the sweep pays
/// the period's late usage from it. 5000 of credit all paid a one-off invoice before it expired
/// (13 hours ago, inside the 24-hour grace); a note gives 2000 back, which waits. Past the grace,
/// January's 300 units (3000) settle the 2000 and the draft bills the other 1000.
#[tokio::test]
async fn a_note_inside_the_grace_leaves_its_value_for_the_late_usage() {
    use crate::test_customer_credit::{draft, line};

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 24).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;
    let one_off = draft(&env.setup, vec![line("Support", GRANT, false)])
        .await
        .expect("one-off draft");
    let one_off = env
        .services()
        .finalize_invoice(Actor::System, one_off.id, TENANT_ID)
        .await
        .expect("finalized")
        .invoice;
    assert_eq!((one_off.applied_credits, one_off.amount_due), (GRANT, 0));
    let s = Scenario {
        env,
        usage,
        subscription_id,
        grant,
    };
    expire_grant(s.env.store(), s.grant).await;

    s.env
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: one_off.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: one_off.line_items[0].local_id.clone(),
                    amount_cents: 2_000,
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .expect("credit note");
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![2_000]);
    assert!(
        s.recorded_expiries().await.is_empty(),
        "inside the grace, the value given back waits for the sweep"
    );

    set_period_usage(&s.usage, 300);
    expire_grant_hours_ago(s.env.store(), s.grant, 25).await;
    // The grace has passed: the credit was funded when it took effect, and the one-off drew it
    // while it was live, before it expired.
    s.execute(&format!(
        "SET session_replication_role = replica;
         UPDATE customer_credit_entry SET occurred_at = now() - interval '27 hours'
         WHERE grant_id = '{grant}' AND kind = 'FUND';
         UPDATE customer_credit_entry SET occurred_at = now() - interval '26 hours'
         WHERE grant_id = '{grant}' AND kind = 'DRAW' AND invoice_id = '{}';
         SET session_replication_role = origin;",
        one_off.id.as_uuid(),
        grant = s.grant.as_uuid(),
    ))
    .await;
    s.sweep().await;
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 2_000, "all the value given back");
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-GRANT, -2_000]);

    let invoice = s.finalize().await;
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (3_000, 2_000, 1_000)
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// OQ9: a promotional credit lapsing during a free trial lapses whole: the trial's usage is never
/// invoiced, so there is nothing for it to settle.
#[tokio::test]
async fn a_promo_lapsing_during_a_free_trial_settles_no_trial_usage() {
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{ProductRef, UsagePricingModel};
    use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
    use rust_decimal_macros::dec;

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    super::credit_plan_change::add_component(
        &env,
        "Bandwidth",
        PLAN_VERSION_PAID_FREE_TRIAL_ID,
        ProductRef::New {
            name: "Bandwidth".to_owned(),
            fee_type: FeeTypeEnum::Usage,
            fee_structure: FeeStructure::Usage {
                metric_id: METRIC_BANDWIDTH,
                model: UsageModel::PerUnit,
            },
        },
        Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
    )
    .await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date())
        .on_start()
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(subscription_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::TrialActive
    );
    let promo = crate::helpers::credit::fund_promotional_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '2024-01-10T00:00:00Z' WHERE id = '{}';
             SET session_replication_role = origin;",
            promo.as_uuid()
        ))
        .await
        .expect("the promo lapses on Jan 10, inside the trial");
    let (from, until) = (
        start_date().and_time(NaiveTime::MIN),
        NaiveDate::from_ymd_opt(2024, 1, 10)
            .unwrap()
            .and_time(NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: from,
            period_end: until,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: from,
                end: until,
            },
        },
    );

    env.store()
        .expire_customer_credit_for(
            TENANT_ID,
            CUST_UBER_ID,
            until.and_utc() + Duration::hours(1),
        )
        .await
        .expect("expiry sweep");
    let mut conn = env.conn().await;
    let movements: Vec<(String, i64)> =
        diesel_models::customer_credit::CustomerCreditEntryRow::list_by_grant(
            &mut conn, TENANT_ID, promo,
        )
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| (entry.kind, entry.amount))
        .collect();
    assert_eq!(
        movements,
        vec![("FUND".to_owned(), GRANT), ("EXPIRY".to_owned(), -GRANT)],
        "trial usage is never invoiced: the promo settles none of it"
    );
    assert!(
        CreditUsageSettlementRow::list_for_period(
            &mut conn,
            TENANT_ID,
            subscription_id,
            start_date(),
            "EUR",
        )
        .await
        .expect("settlements")
        .is_empty()
    );
}

/// A consolidating entity issues its invoices an hour at the earliest after the period closes, so
/// its settlement grace is an hour even with no grace of its own: expired 45 minutes ago, the
/// credit waits; two hours ago, it settles.
#[tokio::test]
async fn a_consolidating_entity_settles_after_an_hour_at_the_earliest() {
    use diesel_async::SimpleAsyncConnection;
    let s = consolidated(100, 2).await;
    s.env
        .conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = now() - interval '3 hours',
                 expires_at = now() - interval '45 minutes' WHERE id = '{}';
             SET session_replication_role = origin;",
            s.grant.as_uuid()
        ))
        .await
        .expect("expired 45 minutes ago");
    assert_eq!(s.sweep().await, 0, "inside the one-hour floor");
    expire_grant_hours_ago(s.env.store(), s.grant, 2).await;
    assert_eq!(s.sweep().await, 1);
}

/// Credit whose settlement keeps failing lapses a week after its settlement grace, not after its
/// expiry: with a 48-hour grace, still held eight days after it expired, lapsed after ten.
#[tokio::test]
async fn expired_credit_lapses_unsettled_a_week_after_its_settlement_grace() {
    let s = scenario(100).await;
    s.env.set_consolidate_and_grace(false, 48).await;
    s.execute(
        "CREATE FUNCTION test_refuse_draw() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'draws are refused'; END $$;
         CREATE TRIGGER test_refuse_draw BEFORE INSERT ON customer_credit_entry
             FOR EACH ROW WHEN (NEW.kind = 'DRAW') EXECUTE FUNCTION test_refuse_draw();",
    )
    .await;
    expire_grant_hours_ago(s.env.store(), s.grant, 8 * 24).await;
    assert_eq!(s.sweep().await, 0, "a week has not passed since the grace");
    expire_grant_hours_ago(s.env.store(), s.grant, 10 * 24).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-5_000]);
}

/// A void gives settled credit back to a grant that expired two hours ago: inside the entity's
/// 24-hour settlement grace, the void leaves the lapse to the sweep.
#[tokio::test]
async fn a_void_inside_the_entity_grace_leaves_the_lapse_to_the_sweep() {
    let s = voidable_scenario(120).await;
    s.env.set_consolidate_and_grace(false, 24).await;
    s.void_settled_period().await;
    assert!(
        s.recorded_expiries().await.is_empty(),
        "two hours into a 24-hour grace"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn voiding_a_settled_invoice_gives_the_settled_credit_back_once() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);
    let invoice = s.finalize().await;
    let invoice_id: InvoiceId = invoice.id;

    s.env
        .try_void_invoice(invoice_id)
        .await
        .expect("void the settled invoice");
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert!(
        s.env.try_void_invoice(invoice_id).await.is_err(),
        "a voided invoice cannot be voided again"
    );
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The sweep had already lapsed the grant when the invoice that took its settlement over is
/// voided: the void lapses the 1000 it gives back in its own transaction, so the ledger shows the
/// expiry at once (the old code left it to the next sweep) and that sweep writes nothing more.
#[tokio::test]
async fn voiding_an_invoice_lapses_what_it_gives_back_to_expired_credit_at_once() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-4_000]);
    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_000);
    assert_eq!(invoice.applied_credits, 1_000);

    s.env
        .try_void_invoice(invoice.id)
        .await
        .expect("void the settled invoice");
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-4_000, -1_000],
        "the value given back to the expired grant lapses with the void"
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    let statement = s.statement().await;
    assert_eq!(s.sweep().await, 0, "nothing is left for the sweep to lapse");
    assert_eq!(s.statement().await.len(), statement.len());
    assert_eq!(
        statement.iter().map(|e| e.amount_minor).sum::<i64>(),
        0,
        "fund 5000, draw 1000, give back 1000, lapse 4000 + 1000"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// January bills 150 units (1500) against a 1000 grant, effective since January, that expired
/// `minutes` ago: its invoice keeps 500 due, so it stays unpaid and can be voided.
async fn voidable_scenario(minutes: i64) -> Scenario {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(1_000)
        },
    )
    .await;
    set_period_usage(&usage, 150);
    let s = Scenario {
        env,
        usage,
        subscription_id,
        grant,
    };
    // Effective since January: only credit effective before its lines end pays their usage.
    s.execute(&format!(
        "SET session_replication_role = replica;
         UPDATE customer_credit_grant SET effective_at = '{}',
             expires_at = now() - interval '{minutes} minutes' WHERE id = '{}';
         SET session_replication_role = origin;",
        crate::helpers::credit::FIXTURE_EFFECTIVE_AT,
        grant.as_uuid()
    ))
    .await;
    s
}

impl Scenario {
    /// Voids the period's invoice, which took the settlement over and keeps 500 due.
    async fn void_settled_period(&self) -> InvoiceId {
        let invoice = self.finalize().await;
        assert_eq!(invoice.total, 1_500);
        assert_eq!(invoice.applied_credits, 1_000, "the settlement taken over");
        assert_eq!(invoice.amount_due, 500);
        let settlements = self.settlements().await;
        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0].amount, 1_000);
        assert_eq!(settlements[0].invoice_id, Some(invoice.id));
        self.env
            .try_void_invoice(invoice.id)
            .await
            .expect("void the unpaid settled invoice");
        invoice.id
    }

    /// Cancels effective on the voided invoice's period start, February 1.
    async fn cancel_on_the_voided_period(&self) {
        self.env
            .services()
            .cancel_subscription(
                Actor::System,
                self.subscription_id,
                TENANT_ID,
                Some("cancelled on a voided period".to_owned()),
                CancellationEffectiveAt::Date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
            )
            .await
            .expect("schedule the cancellation");
        self.env.process_due_events().await;
        self.env.process_due_events().await;
    }

    /// Cancelled on the voided invoice's date, January is billed again only when the operator
    /// regenerates the voided invoice: a new one is created and finalized.
    async fn bill_voided_period_again(&self, voided: InvoiceId) -> meteroid_store::domain::Invoice {
        self.cancel_on_the_voided_period().await;
        assert_eq!(self.env.get_invoices(self.subscription_id).await.len(), 1);
        self.env
            .services()
            .regenerate_invoice(Actor::System, TENANT_ID, voided)
            .await
            .expect("regenerate the voided invoice");
        self.env.process_due_events().await;
        let invoices = self.env.get_invoices(self.subscription_id).await;
        assert_eq!(invoices.len(), 2, "the voided invoice and its re-bill");
        let voided_invoice = invoices.iter().find(|i| i.id == voided).unwrap();
        assert_eq!(voided_invoice.status, InvoiceStatusEnum::Void);
        let rebill = invoices.into_iter().find(|i| i.id != voided).unwrap();
        assert_eq!(rebill.status, InvoiceStatusEnum::Finalized);
        assert_eq!(
            rebill.billing_period_start,
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        );
        rebill
    }

    /// The lapses the ledger recorded; the statement also lists expired value not lapsed yet.
    async fn recorded_expiries(&self) -> Vec<i64> {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_credit_entry::dsl as e;
        let mut conn = self.env.conn().await;
        e::customer_credit_entry
            .filter(e::grant_id.eq(self.grant))
            .filter(e::kind.eq("EXPIRY"))
            .order(e::created_at.asc())
            .select(e::amount)
            .load(&mut conn)
            .await
            .expect("recorded expiries")
    }

    /// The voided invoice's settlement stays its own: never taken over, re-settled or returned.
    async fn assert_settlement_left_with(&self, voided: InvoiceId) {
        let settlements = self.settlements().await;
        assert_eq!(settlements.len(), 1, "no second settlement of January");
        assert_eq!(settlements[0].amount, 1_000);
        assert_eq!(settlements[0].invoice_id, Some(voided));
        assert!(self.given_back_settlements().await.is_empty());
    }
}

/// FOLLOWUPS 140: a void is terminal. Cancelling effective on the voided invoice's period start
/// neither bills that period again nor draws credit for it (Stripe; Lago and Chargebee regenerate
/// explicitly only).
#[tokio::test]
async fn cancelling_on_a_voided_period_does_not_bill_it_again() {
    let s = voidable_scenario(120).await;
    s.sweep().await;
    let voided = s.void_settled_period().await;
    let live = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(5_000),
    )
    .await;
    s.cancel_on_the_voided_period().await;

    let invoices = s.env.get_invoices(s.subscription_id).await;
    assert_eq!(
        invoices
            .iter()
            .map(|i| (i.id, i.status))
            .collect::<Vec<_>>(),
        vec![(voided, InvoiceStatusEnum::Void)],
        "the voided period is not billed again"
    );
    assert!(s.amounts_of(live, LedgerEntryKind::Draw).await.is_empty());
    s.assert_settlement_left_with(voided).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        5_000
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// E-m6, through an explicit regeneration: a voided period billed again neither reuses the
/// settlement its void gave back nor settles it again. The void lapsed the 1000 at once; the
/// re-bill draws all 1500 from live credit.
#[tokio::test]
async fn a_voided_period_billed_again_draws_its_usage_once_from_live_credit() {
    let s = voidable_scenario(120).await;
    s.sweep().await;
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 1_000, "the whole grant pays usage");
    assert_eq!(settlements[0].invoice_id, None);

    let voided = s.void_settled_period().await;
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-1_000],
        "given back to expired credit, it lapses with the void"
    );
    assert_eq!(s.recorded_expiries().await, vec![-1_000]);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    let live = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(5_000),
    )
    .await;
    let rebill = s.bill_voided_period_again(voided).await;
    assert_eq!(rebill.total, 1_500);
    assert_eq!(rebill.applied_credits, 1_500);
    assert_eq!(rebill.amount_due, 0);
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::Draw).await,
        vec![-1_500]
    );
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_eq!(s.recorded_expiries().await, vec![-1_000]);
    s.assert_settlement_left_with(voided).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        3_500
    );
    assert_eq!(
        s.statement()
            .await
            .iter()
            .map(|e| e.amount_minor)
            .sum::<i64>(),
        3_500,
        "fund 1000 + 5000, draw 1000, give back 1000, lapse 1000, draw 1500"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// E-m6, the void inside the settlement grace: the value it gives back is not lapsed yet when
/// January is billed again, and the stale settlement keeps it from paying January a second time;
/// the next sweep lapses it, as the void would have.
#[tokio::test]
async fn a_voided_period_billed_again_within_the_grace_does_not_settle_twice() {
    let s = voidable_scenario(10).await;
    let voided = s.void_settled_period().await;
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert!(
        s.recorded_expiries().await.is_empty(),
        "inside the grace, the void leaves the lapse to the sweep"
    );
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-1_000],
        "the statement already shows the value given back as lapsing"
    );

    let live = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(5_000),
    )
    .await;
    let rebill = s.bill_voided_period_again(voided).await;
    assert_eq!(rebill.total, 1_500);
    assert_eq!(rebill.applied_credits, 1_500);
    assert_eq!(rebill.amount_due, 0);
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::Draw).await,
        vec![-1_500]
    );
    assert_eq!(
        s.amounts(LedgerEntryKind::Draw).await,
        vec![-1_000],
        "the expired grant pays January once"
    );
    assert!(s.recorded_expiries().await.is_empty());
    s.assert_settlement_left_with(voided).await;

    expire_grant(s.env.store(), s.grant).await;
    s.sweep().await;
    assert_eq!(s.recorded_expiries().await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-1_000]);
    s.assert_settlement_left_with(voided).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        3_500
    );
    assert_eq!(
        s.statement()
            .await
            .iter()
            .map(|e| e.amount_minor)
            .sum::<i64>(),
        3_500
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Finalization and the sweep race for the same expired credit: whichever comes second finds
/// the usage already paid.
#[tokio::test]
async fn an_invoice_finalizing_while_the_sweep_runs_pays_the_usage_once() {
    let s = scenario(100).await;

    let (_, invoice) = tokio::join!(s.sweep(), s.finalize());
    s.sweep().await;

    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.total, 1_000);
    assert_eq!(invoice.applied_credits, 1_000);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-4_000]);
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 1_000);
    assert_eq!(settlements[0].invoice_id, Some(invoice.id));
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The invoice of the period finalizes before the sweep has looked at the expired credit: it
/// pays the usage that credit covered itself, and the sweep then lapses only the rest.
#[tokio::test]
async fn an_invoice_finalizing_before_the_sweep_pays_the_usage_the_expired_credit_covered() {
    let s = scenario(100).await;

    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.total, 1_000);
    assert_eq!(invoice.applied_credits, 1_000);
    assert_eq!(invoice.amount_due, 0);
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 1_000);
    assert_eq!(settlements[0].invoice_id, Some(invoice.id));
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);

    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-4_000]);
    assert_eq!(s.settlements().await.len(), 1);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A price an operator pinned on the draft applies to the usage the expired credit pays too,
/// so the settlement and the invoice agree.
#[tokio::test]
async fn a_settlement_uses_the_price_pinned_on_the_draft() {
    let s = scenario_with(100, true).await;
    s.pin_draft_price("0.20").await;
    let draft = s.env.get_invoices(s.subscription_id).await.remove(0);
    let pinned = draft
        .line_items
        .iter()
        .find(|line| line.metric_id.is_some())
        .expect("the draft has a usage line");
    assert_eq!(pinned.unit_price, Some(Decimal::new(20, 2)));

    assert_eq!(s.sweep().await, 1);
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(
        settlements[0].amount, 2_000,
        "100 units at the pinned 0.20 EUR"
    );
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-2_000]);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-3_000]);

    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 2_000);
    assert_eq!(invoice.applied_credits, 2_000);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-2_000]);
    assert_eq!(s.settlements().await[0].invoice_id, Some(invoice.id));
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The period ends up billing nothing, so its invoice closes without taking the settlement over:
/// the settlement is given back once no invoice can bill its period, and the credit lapses again.
#[tokio::test]
async fn a_settlement_whose_period_bills_nothing_is_given_back() {
    let s = scenario(100).await;
    assert_eq!(s.sweep().await, 1);

    set_period_usage(&s.usage, 0);
    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Closed);
    let open = s.settlements().await;
    assert_eq!(open.len(), 1);
    assert_eq!((open[0].invoice_id, open[0].returned_at), (None, None));

    s.sweep().await;
    assert_eq!(
        s.settlements().await[0].returned_at,
        None,
        "the invoice of the period may still be on its way"
    );

    s.age_settlements(2);
    s.sweep().await;
    assert!(
        s.settlements().await.is_empty(),
        "a returned settlement no longer stands"
    );
    let returned = s.given_back_settlements().await;
    assert_eq!(returned.len(), 1);
    assert_eq!((returned[0].amount, returned[0].invoice_id), (1_000, None));
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    let funded = s.funded_sources(s.grant).await;
    assert!(
        funded.contains(&(CreditGrantSource::UsageSettlementReturn, 1_000)),
        "the given-back credit is announced as a settlement return: {funded:?}"
    );
    assert!(
        funded
            .iter()
            .all(|(source, _)| *source != CreditGrantSource::InvoiceVoid),
        "credit given back by a settlement is not an invoice void"
    );
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-4_000, -1_000],
        "the grant had expired, so the returned value lapses"
    );

    let entries = s.entries().await.len();
    s.sweep().await;
    assert_eq!(
        s.entries().await.len(),
        entries,
        "a repeated pass returns nothing twice"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A customer whose settlement keeps failing does not hold expired value forever: after a week
/// the credit lapses without it.
#[tokio::test]
async fn expired_credit_lapses_after_a_week_even_when_its_settlement_keeps_failing() {
    let s = scenario(100).await;
    s.execute(
        "CREATE FUNCTION test_refuse_draw() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'draws are refused'; END $$;
         CREATE TRIGGER test_refuse_draw BEFORE INSERT ON customer_credit_entry
             FOR EACH ROW WHEN (NEW.kind = 'DRAW') EXECUTE FUNCTION test_refuse_draw();",
    )
    .await;

    assert_eq!(
        s.sweep().await,
        0,
        "it expired two hours ago: settlement is still retried"
    );
    assert!(s.settlements().await.is_empty());
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );

    expire_grant_hours_ago(s.env.store(), s.grant, 8 * 24).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-5_000]);
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_credit_entry::dsl as e;
        let mut conn = s.env.conn().await;
        let reasons = e::customer_credit_entry
            .filter(e::grant_id.eq(s.grant))
            .filter(e::kind.eq("EXPIRY"))
            .select(e::reason)
            .load::<Option<String>>(&mut conn)
            .await
            .expect("expiry reasons");
        assert_eq!(
            reasons,
            vec![Some(
                "Lapsed without settling the usage it covered".to_owned()
            )]
        );
    }
    assert!(s.amounts(LedgerEntryKind::Draw).await.is_empty());
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Two grants of a customer expire within the one period: the usage is paid once, not once per
/// expiry.
#[tokio::test]
async fn two_expiries_in_one_period_pay_the_usage_once() {
    let s = scenario(100).await;
    let second = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(300)
        },
    )
    .await;
    expire_grant_hours_ago(s.env.store(), second, 1).await;

    assert_eq!(s.sweep().await, 2);
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -1_000);
    assert_eq!(s.total(LedgerEntryKind::Expiry).await, -4_300);
    let settled = s.settlements().await;
    assert_eq!(settled.iter().map(|row| row.amount).sum::<i64>(), 1_000);

    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_000);
    assert_eq!(invoice.applied_credits, 1_000);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -1_000);
    assert_eq!(s.total(LedgerEntryKind::DrawReversal).await, 0);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// An invoice paid partly with expired-credit settlement and partly with live credit, and a live
/// grant that covers the rest.
async fn settled_and_live(s: &Scenario) -> (meteroid_store::domain::Invoice, CreditGrantId) {
    assert_eq!(s.sweep().await, 1);
    let live = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(5_000),
    )
    .await;
    set_period_usage(&s.usage, 150);
    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_500);
    assert_eq!(invoice.applied_credits, 1_500);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts_of(live, LedgerEntryKind::Draw).await, vec![-500]);
    s.env.run_outbox_and_orchestration().await;
    let invoice = s.env.get_invoices(s.subscription_id).await.remove(0);
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    (invoice, live)
}

#[tokio::test]
async fn a_credit_note_on_a_settled_invoice_gives_back_live_credit_and_its_void_takes_it_again() {
    let s = scenario(100).await;
    let (invoice, live) = settled_and_live(&s).await;
    let line = invoice
        .line_items
        .iter()
        .find(|line| line.metric_id.is_some())
        .expect("usage line")
        .local_id
        .clone();

    let note = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Line {
                    local_id: line,
                    quantity: Decimal::from(50),
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("credit note");
    s.env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("finalize the credit note");
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::DrawReversal).await,
        vec![500]
    );
    assert!(s.amounts(LedgerEntryKind::DrawReversal).await.is_empty());
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        5_000
    );

    s.env
        .store()
        .void_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("void the credit note");
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::Draw).await,
        vec![-500, -500]
    );
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::DrawReversal).await,
        vec![500]
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        4_500
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A credit note over the whole invoice gives back what the settlement drew too, to its own grant;
/// the value lapses again, so the note can no longer be voided.
#[tokio::test]
async fn a_credit_note_over_a_settled_invoice_returns_the_settled_credit_where_it_came_from() {
    let s = scenario(100).await;
    let (invoice, live) = settled_and_live(&s).await;

    let note = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![],
                reason: None,
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("credit note");
    s.env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("finalize the credit note");
    assert_eq!(
        s.amounts_of(live, LedgerEntryKind::DrawReversal).await,
        vec![500]
    );
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    // What came back to the expired grant lapsed as it came back: the sweep finds nothing.
    assert_eq!(s.sweep().await, 0);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-4_000, -1_000],
        "the expired grant does not keep what came back to it"
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        5_000
    );

    let statement = s.statement().await.len();
    let refused = s
        .env
        .store()
        .void_credit_note(Actor::System, TENANT_ID, note.id)
        .await;
    assert!(refused.is_err(), "the credit that came back has lapsed");
    assert_eq!(s.statement().await.len(), statement);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        5_000
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// G1 (300) expired with 1,000 of usage: its settlement drew 300 from it and 700 from live G2. The
/// invoice kept that and drew 500 more from G2. A note returning 600 gives back the latest drawn
/// first, 500 then 100 of G2; voided, it draws them again, and the invoice still voids: G1 gets
/// its 300 back and lapses them at once, G2 gets 1,200.
#[tokio::test]
async fn an_invoice_whose_note_re_drew_settled_credit_still_voids() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let g1 = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(300)
        },
    )
    .await;
    expire_grant(env.store(), g1).await;
    use meteroid_store::domain::credit::GrantClassification;
    use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
    // Live since before G1 expired, so the settlement draws on it too.
    let wallet = wallets(env.store(), TENANT_ID, CUST_UBER_ID)
        .await
        .remove(0);
    let g2 = env
        .store()
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 5_000,
                classification: GrantClassification::Paid,
                priority: 100,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Integration-test customer credit".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            Utc::now() - Duration::hours(14),
        )
        .await
        .expect("fund the live grant")
        .grant_id;
    crate::helpers::credit::effective_from_fixture_start(env.store(), g2).await;
    set_period_usage(&usage, 100);
    let s = Scenario {
        env,
        usage,
        subscription_id,
        grant: g1,
    };
    // Settled whole, G1 leaves nothing to lapse.
    assert_eq!(s.sweep().await, 0);
    assert_eq!(s.amounts_of(g1, LedgerEntryKind::Draw).await, vec![-300]);
    assert_eq!(s.amounts_of(g2, LedgerEntryKind::Draw).await, vec![-700]);

    set_period_usage(&s.usage, 150);
    let invoice = s.finalize().await;
    assert_eq!(invoice.total, 1_500);
    assert_eq!(invoice.applied_credits, 1_500);
    assert_eq!(
        s.amounts_of(g2, LedgerEntryKind::Draw).await,
        vec![-700, -500]
    );
    s.env.run_outbox_and_orchestration().await;

    let line = invoice
        .line_items
        .iter()
        .find(|line| line.metric_id.is_some())
        .expect("usage line")
        .local_id
        .clone();
    let note = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Line {
                    local_id: line,
                    quantity: Decimal::from(60),
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .expect("credit note");
    s.env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("finalize the credit note");
    assert_eq!(
        s.amounts_of(g2, LedgerEntryKind::DrawReversal).await,
        vec![500, 100]
    );
    assert!(
        s.amounts_of(g1, LedgerEntryKind::DrawReversal)
            .await
            .is_empty()
    );

    let voided = s
        .env
        .store()
        .void_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("void the credit note");
    assert!(voided.voided_at.is_some(), "the void records when");
    assert_eq!(
        s.amounts_of(g2, LedgerEntryKind::Draw).await,
        vec![-700, -500, -500, -100]
    );

    // Credit settled the invoice in full, and a paid invoice cannot be voided: the give-back of
    // the void is what is under test, so the payment status is cleared by hand.
    s.execute(&format!(
        "UPDATE invoice SET payment_status = 'UNPAID' WHERE id = '{}'",
        invoice.id.as_uuid()
    ))
    .await;
    s.env
        .try_void_invoice(invoice.id)
        .await
        .expect("void the invoice the voided note re-drew credit for");
    assert_eq!(
        s.amounts_of(g1, LedgerEntryKind::DrawReversal).await,
        vec![300]
    );
    assert_eq!(s.amounts_of(g1, LedgerEntryKind::Expiry).await, vec![-300]);
    assert_eq!(
        s.amounts_of(g2, LedgerEntryKind::DrawReversal)
            .await
            .iter()
            .sum::<i64>(),
        600 + 1_200
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        5_000
    );
    for grant in [g1, g2] {
        assert_eq!(
            s.entries_of(grant)
                .await
                .iter()
                .map(|entry| entry.amount_minor)
                .sum::<i64>(),
            if grant == g1 { 0 } else { 5_000 },
            "the ledger sums to what the grant holds"
        );
    }
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Two usage-only subscriptions of the customer whose first periods close the same day and are
/// consolidated into one invoice, and a promotional grant that expired `expired_hours_ago`.
struct Consolidated {
    env: TestEnv,
    usage: MutableMockUsageClient,
    subscriptions: Vec<SubscriptionId>,
    grant: CreditGrantId,
}

async fn consolidated(units: i64, expired_hours_ago: i64) -> Consolidated {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(true, 0).await;
    let mut subscriptions = Vec::new();
    for _ in 0..2 {
        subscriptions.push(
            subscription()
                .customer(CUST_UBER_ID)
                .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
                .start_date(start_date())
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    env.process_cycle_transitions().await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;
    expire_grant_hours_ago(env.store(), grant, expired_hours_ago).await;
    set_period_usage(&usage, units);
    Consolidated {
        env,
        usage,
        subscriptions,
        grant,
    }
}

impl Consolidated {
    async fn sweep(&self) -> usize {
        self.env
            .store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep")
    }

    async fn settlements(&self) -> Vec<CreditUsageSettlementRow> {
        let mut conn = self.env.conn().await;
        let mut all = Vec::new();
        for subscription_id in &self.subscriptions {
            all.extend(
                CreditUsageSettlementRow::list_for_period(
                    &mut conn,
                    TENANT_ID,
                    *subscription_id,
                    start_date(),
                    "EUR",
                )
                .await
                .expect("settlements of the period"),
            );
        }
        all
    }

    async fn open_settlements(&self) -> Vec<CreditUsageSettlementRow> {
        let mut conn = self.env.conn().await;
        CreditUsageSettlementRow::list_open_for_customers(&mut conn, TENANT_ID, &[CUST_UBER_ID])
            .await
            .expect("open settlements")
    }

    async fn total(&self, kind: LedgerEntryKind) -> i64 {
        let wallet = wallets(self.env.store(), TENANT_ID, CUST_UBER_ID)
            .await
            .remove(0);
        let (entries, _) = self
            .env
            .store()
            .customer_credit_ledger(TENANT_ID, wallet.id, 100, 0)
            .await
            .expect("statement");
        entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.amount_minor)
            .sum()
    }

    /// Fires the grace-period finalization, which consolidates both drafts into one parent.
    async fn finalize(&self) -> meteroid_store::domain::Invoice {
        self.env.process_due_events().await;
        let parents = self
            .env
            .get_customer_invoices(CUST_UBER_ID)
            .await
            .into_iter()
            .filter(|invoice| {
                invoice.subscription_id.is_none()
                    && invoice.invoice_type == meteroid_store::domain::enums::InvoiceType::Recurring
            })
            .collect::<Vec<_>>();
        assert_eq!(parents.len(), 1, "both drafts are consolidated");
        let parent = parents.into_iter().next().unwrap();
        let children = self.env.get_consolidated_children(parent.id).await;
        assert_eq!(children.len(), 2);
        parent
    }

    async fn age_settlements_and_sweep(&self) {
        self.env.advance(chrono::Duration::days(2));
        self.sweep().await;
    }

    async fn assert_settled_by(&self, parent: &meteroid_store::domain::Invoice) {
        let settlements = self.settlements().await;
        assert_eq!(settlements.len(), 2, "one settlement per subscription");
        assert!(
            settlements
                .iter()
                .all(|row| row.amount == 1_000 && row.invoice_id == Some(parent.id)),
            "the consolidated invoice takes both periods' settlements over: {settlements:?}"
        );
        assert!(self.open_settlements().await.is_empty());
    }
}

/// The parent of consolidated drafts bills both subscriptions' periods: it takes their
/// settlements over instead of leaving them open and charging the usage again.
#[tokio::test]
async fn a_consolidated_invoice_takes_over_the_settlements_of_every_merged_period() {
    let s = consolidated(100, 2).await;

    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.settlements().await.len(), 2);
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -2_000);
    assert_eq!(s.total(LedgerEntryKind::Expiry).await, -3_000);

    let parent = s.finalize().await;
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        parent.total, 2_000,
        "100 units at 0.10 EUR on each subscription"
    );
    assert_eq!(parent.applied_credits, 2_000);
    assert_eq!(parent.amount_due, 0);
    s.assert_settled_by(&parent).await;
    assert_eq!(
        s.total(LedgerEntryKind::Draw).await,
        -2_000,
        "finalization takes the settlements over, it does not draw again"
    );
    assert_eq!(s.total(LedgerEntryKind::DrawReversal).await, 0);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );

    s.age_settlements_and_sweep().await;
    s.assert_settled_by(&parent).await;
    assert_eq!(s.total(LedgerEntryKind::DrawReversal).await, 0);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The credit expired within the settlement grace, so the sweep has not looked at it yet: the
/// consolidated invoice settles every merged period itself, and the sweep then lapses the rest.
#[tokio::test]
async fn a_consolidated_invoice_settles_an_expiry_the_sweep_has_not_reached() {
    let s = consolidated(100, 0).await;

    assert_eq!(
        s.sweep().await,
        0,
        "the expiry is within the settlement grace"
    );
    let parent = s.finalize().await;
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    assert_eq!(parent.total, 2_000);
    assert_eq!(parent.applied_credits, 2_000);
    assert_eq!(parent.amount_due, 0);
    s.assert_settled_by(&parent).await;
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -2_000);

    expire_grant(s.env.store(), s.grant).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -2_000);
    assert_eq!(s.total(LedgerEntryKind::Expiry).await, -3_000);
    s.assert_settled_by(&parent).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Every merged period ends up billing nothing, so the consolidated invoice closes: the drafts it
/// merged no longer hold their periods open, and the settlements are given back.
#[tokio::test]
async fn the_settlements_of_a_consolidated_invoice_that_closes_are_given_back() {
    let s = consolidated(100, 2).await;
    assert_eq!(s.sweep().await, 1);

    set_period_usage(&s.usage, 0);
    let parent = s.finalize().await;
    assert_eq!(parent.status, InvoiceStatusEnum::Closed);
    assert_eq!(s.open_settlements().await.len(), 2);

    s.age_settlements_and_sweep().await;
    assert!(
        s.open_settlements().await.is_empty(),
        "no invoice can bill the merged periods any more"
    );
    assert!(s.settlements().await.is_empty());
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -2_000);
    assert_eq!(s.total(LedgerEntryKind::DrawReversal).await, 2_000);
    assert_eq!(
        s.total(LedgerEntryKind::Expiry).await,
        -5_000,
        "the grant had expired, so the returned value lapses"
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Voiding the consolidated parent must undo exactly what it took over: both merged periods'
/// settlements, reversed in one draw-back per grant (the parent is one invoice), not per period.
/// Added for commit 8d5acc030 (consolidation netting), which had no void coverage: the void path
/// (`restore_invoice_credit_tx` -> `reverse_invoice_draws_tx`) finds every settlement attached to
/// the parent via `CreditUsageSettlementRow::list_by_invoice`, regardless of how many subscription
/// periods it merged, so it generalizes to the consolidated case without change.
#[tokio::test]
async fn voiding_a_consolidated_invoice_gives_back_the_settled_credit_of_every_merged_period() {
    let s = consolidated(100, 2).await;
    assert_eq!(s.sweep().await, 1);
    let parent = s.finalize().await;
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    assert_eq!(parent.applied_credits, 2_000);
    assert_eq!(s.total(LedgerEntryKind::Draw).await, -2_000);
    assert_eq!(s.total(LedgerEntryKind::DrawReversal).await, 0);

    s.env
        .try_void_invoice(parent.id)
        .await
        .expect("void the consolidated parent");
    assert_eq!(
        s.total(LedgerEntryKind::DrawReversal).await,
        2_000,
        "both merged periods' settlement draws are given back, exactly as voiding a single \
         invoice gives back what it drew"
    );
    assert!(
        s.env.try_void_invoice(parent.id).await.is_err(),
        "a voided invoice cannot be voided again"
    );
    assert_eq!(
        s.total(LedgerEntryKind::DrawReversal).await,
        2_000,
        "a repeated void gives nothing back twice"
    );
    s.assert_settled_by(&parent).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        0,
        "the grant that paid both periods had already expired before the void; value given back \
         to it cannot be spent"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

// ---- Capped rollover of included credit (PR 3 S2) ----

/// `scenario(units)` whose expiring grant rolls over: up to `max_minor`, for one month.
async fn rollover_scenario(units: i64, max_minor: Option<i64>) -> Scenario {
    use meteroid_store::domain::credit::GrantRollover;
    use meteroid_store::domain::price_components::CreditExpiry;
    let s = scenario(units).await;
    let rollover = GrantRollover {
        max_minor,
        expiry: CreditExpiry::Month(1),
    }
    .to_json()
    .expect("rollover json");
    s.execute(&format!(
        "SET session_replication_role = replica;
         UPDATE customer_credit_grant SET rollover = '{rollover}'::jsonb WHERE id = '{}';
         SET session_replication_role = origin;",
        s.grant.as_uuid()
    ))
    .await;
    s
}

impl Scenario {
    async fn rolled_grants(&self) -> Vec<meteroid_store::domain::credit::CreditGrant> {
        wallets(self.env.store(), TENANT_ID, CUST_UBER_ID)
            .await
            .into_iter()
            .flat_map(|wallet| wallet.grants)
            .filter(|grant| grant.source == CreditGrantSource::Rollover)
            .collect()
    }

    async fn grant_row(
        &self,
        grant: CreditGrantId,
    ) -> diesel_models::customer_credit::CustomerCreditGrantRow {
        use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
        use diesel_async::RunQueryDsl;
        use diesel_models::customer_credit::CustomerCreditGrantRow;
        use diesel_models::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::id.eq(grant))
            .select(CustomerCreditGrantRow::as_select())
            .first(&mut self.env.conn().await)
            .await
            .expect("grant row")
    }
}

#[tokio::test]
async fn a_capped_rollover_pays_the_usage_before_the_expiry_then_carries_the_cap() {
    use meteroid_store::domain::credit::GrantClassification;
    let s = rollover_scenario(100, Some(2_000)).await;
    let expired = s.grant_row(s.grant).await;

    assert_eq!(s.sweep().await, 1);
    assert_eq!(
        s.settlements().await[0].amount,
        1_000,
        "usage before the expiry paid first"
    );
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-1_000]);
    assert_eq!(s.amounts(LedgerEntryKind::Rollover).await, vec![-2_000]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-2_000],
        "what the cap leaves behind lapses"
    );

    let rolled = s.rolled_grants().await;
    assert_eq!(rolled.len(), 1);
    let rolled = &rolled[0];
    assert_eq!(rolled.amount_minor, 2_000);
    assert_eq!(rolled.remaining_minor, 2_000);
    assert_eq!(rolled.source_id, s.grant.as_uuid());
    assert_eq!(
        rolled.priority,
        expired.priority - 1,
        "burns before fresh credit"
    );
    assert_eq!(
        rolled.classification,
        GrantClassification::from_db_str(&expired.classification).expect("classification")
    );
    let expired_at = expired.expires_at.unwrap();
    assert_eq!(rolled.effective_at, expired_at);
    assert_eq!(
        rolled.expires_at,
        expired_at.checked_add_months(chrono::Months::new(1))
    );
    assert_eq!(
        s.grant_row(rolled.id).await.rollover,
        None,
        "rolled credit never rolls again"
    );
    assert_eq!(
        s.funded_sources(rolled.id).await,
        vec![(CreditGrantSource::Rollover, 2_000)]
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    let invoice = s.finalize().await;
    assert_eq!(
        invoice.applied_credits, 1_000,
        "the settlement pays the period"
    );
    assert_eq!(
        s.rolled_grants().await[0].remaining_minor,
        2_000,
        "the rolled credit is not touched by usage before its own start"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// January's usage up to `until`: `units` units.
fn set_usage_until(usage: &MutableMockUsageClient, until: NaiveDate, units: i64) {
    let start = start_date().and_time(NaiveTime::MIN);
    let end = until.and_time(NaiveTime::MIN);
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

/// The invoice of January finalizes after both of its credits expired, the sweep run on time
/// (`swept_first`) or not at all yet: G1 (3.00, customer-wide) expired Jan 10, G2 (6.00, rolling
/// over at most 2.50 for a month) Jan 20, 3.00 of usage before Jan 10 and 3.00 between. Returns
/// the scenario (its grant G2) and G1.
async fn two_expiries_then_invoice(swept_first: bool) -> (Scenario, CreditGrantId) {
    use meteroid_store::domain::credit::GrantRollover;
    use meteroid_store::domain::price_components::CreditExpiry;
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let day = |d| NaiveDate::from_ymd_opt(2024, 1, d).unwrap();
    set_usage_until(&usage, day(10), 30);
    set_usage_until(&usage, day(20), 60);
    set_usage_until(&usage, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(), 60);
    let fund = |amount| {
        fund_grant(
            env.store(),
            TENANT_ID,
            CUST_UBER_ID,
            None,
            Grant {
                expires_at: Some(Utc::now() + Duration::hours(1)),
                ..Grant::invoice(amount)
            },
        )
    };
    let g1 = fund(300).await;
    let g2 = fund(600).await;
    let rollover = GrantRollover {
        max_minor: Some(250),
        expiry: CreditExpiry::Month(1),
    }
    .to_json()
    .expect("rollover json");
    let s = Scenario {
        env,
        usage,
        subscription_id,
        grant: g2,
    };
    s.execute(&format!(
        "SET session_replication_role = replica;
         UPDATE customer_credit_grant SET effective_at = '2024-01-01', expires_at = '2024-01-10'
             WHERE id = '{}';
         UPDATE customer_credit_grant SET effective_at = '2024-01-01', expires_at = '2024-01-20',
             rollover = '{rollover}'::jsonb WHERE id = '{}';
         SET session_replication_role = origin;",
        g1.as_uuid(),
        g2.as_uuid()
    ))
    .await;
    if swept_first {
        s.sweep().await;
    }
    (s, g1)
}

/// Whichever runs first, the invoice or the sweep, each expiry settles in its turn: G1 pays the
/// usage before Jan 10, G2 the usage after, so G2 rolls 2.50 over and 0.50 lapses. Settling G2
/// alone first would charge all 6.00 to it and roll nothing.
#[tokio::test]
async fn an_invoice_before_the_sweep_settles_earlier_expiries_before_rolling_over() {
    for swept_first in [true, false] {
        let (s, g1) = two_expiries_then_invoice(swept_first).await;
        let invoice = s.finalize().await;
        assert_eq!(
            (invoice.total, invoice.applied_credits, invoice.amount_due),
            (600, 600, 0),
            "swept first: {swept_first}"
        );
        for _ in 0..2 {
            assert_eq!(
                s.amounts_of(g1, LedgerEntryKind::Draw).await,
                vec![-300],
                "swept first: {swept_first}"
            );
            assert!(s.amounts_of(g1, LedgerEntryKind::Expiry).await.is_empty());
            assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-300]);
            assert_eq!(s.amounts(LedgerEntryKind::Rollover).await, vec![-250]);
            assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-50]);
            let rolled = diesel_models::customer_credit::CustomerCreditGrantRow::list_for_customer(
                &mut s.env.conn().await,
                TENANT_ID,
                CUST_UBER_ID,
            )
            .await
            .expect("grants")
            .into_iter()
            .filter(|grant| grant.source_kind == "ROLLOVER")
            .map(|grant| (grant.amount, grant.source_id))
            .collect::<Vec<_>>();
            assert_eq!(rolled, vec![(250, s.grant.as_uuid())], "rolled once");
            assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
            s.sweep().await;
        }
    }
}

#[tokio::test]
async fn a_remainder_under_the_cap_rolls_over_whole() {
    let s = rollover_scenario(100, Some(10_000)).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Rollover).await, vec![-4_000]);
    assert!(s.amounts(LedgerEntryKind::Expiry).await.is_empty());
    let rolled = s.rolled_grants().await;
    assert_eq!(rolled.len(), 1);
    assert_eq!(rolled[0].amount_minor, 4_000);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn nothing_rolls_over_when_the_usage_spent_the_whole_grant() {
    let s = rollover_scenario(500, None).await;
    assert_eq!(
        s.sweep().await,
        0,
        "a spent grant has nothing left to lapse"
    );
    assert_eq!(s.amounts(LedgerEntryKind::Draw).await, vec![-GRANT]);
    assert!(s.amounts(LedgerEntryKind::Rollover).await.is_empty());
    assert!(s.amounts(LedgerEntryKind::Expiry).await.is_empty());
    assert!(s.rolled_grants().await.is_empty());
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn a_repeated_sweep_rolls_over_exactly_once() {
    let s = rollover_scenario(100, Some(2_000)).await;
    assert_eq!(s.sweep().await, 1);
    let statement = s.statement().await;
    assert_eq!(s.sweep().await, 0);
    assert_eq!(
        s.statement().await.len(),
        statement.len(),
        "nothing recorded twice"
    );
    assert_eq!(s.rolled_grants().await.len(), 1);

    // Value a void gives back to the expired grant after it rolled lapses in the void itself: a
    // grant rolls once, and the sweep finds nothing left.
    let invoice = s.finalize().await;
    s.env
        .try_void_invoice(invoice.id)
        .await
        .expect("void the settled invoice");
    assert_eq!(s.amounts(LedgerEntryKind::DrawReversal).await, vec![1_000]);
    assert_eq!(s.sweep().await, 0);
    assert_eq!(s.amounts(LedgerEntryKind::Rollover).await, vec![-2_000]);
    assert_eq!(
        s.amounts(LedgerEntryKind::Expiry).await,
        vec![-2_000, -1_000]
    );
    assert_eq!(s.rolled_grants().await.len(), 1);
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

#[tokio::test]
async fn credit_lapsing_without_its_usage_settled_never_rolls_over() {
    let s = rollover_scenario(100, None).await;
    s.execute(
        "CREATE FUNCTION test_refuse_draw() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'draws are refused'; END $$;
         CREATE TRIGGER test_refuse_draw BEFORE INSERT ON customer_credit_entry
             FOR EACH ROW WHEN (NEW.kind = 'DRAW') EXECUTE FUNCTION test_refuse_draw();",
    )
    .await;
    assert_eq!(s.sweep().await, 0);
    expire_grant_hours_ago(s.env.store(), s.grant, 8 * 24).await;
    assert_eq!(s.sweep().await, 1);
    assert_eq!(s.amounts(LedgerEntryKind::Expiry).await, vec![-GRANT]);
    assert!(s.amounts(LedgerEntryKind::Rollover).await.is_empty());
    assert!(s.rolled_grants().await.is_empty());
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The wallet's balance and its statement keep what the sweep will roll over, as alerts, checks
/// and the live balance do (old code dropped all 5,000 until the sweep ran: wallet 0, the
/// synthesized expiry line -5,000).
#[tokio::test]
async fn the_wallet_and_its_statement_keep_the_credit_about_to_roll_over() {
    let s = rollover_scenario(100, Some(2_000)).await;
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        2_000
    );
    let statement = s.statement().await;
    let expiry = statement
        .iter()
        .find(|entry| entry.kind == LedgerEntryKind::Expiry && entry.grant_id == s.grant)
        .expect("the synthesized expiry");
    assert_eq!(
        expiry.amount_minor,
        -(GRANT - 2_000),
        "only what will not roll"
    );
    assert_eq!(
        statement.last().map(|entry| entry.balance_after_minor),
        Some(2_000)
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;

    assert_eq!(s.sweep().await, 1);
    assert_eq!(
        available(s.env.store(), TENANT_ID, CUST_UBER_ID, None).await,
        2_000
    );
    assert_eq!(
        s.statement()
            .await
            .last()
            .map(|entry| entry.balance_after_minor),
        Some(2_000),
        "the sweep moves the value, it does not change the balance"
    );
    assert_ledger_balances(s.env.store(), TENANT_ID, CUST_UBER_ID).await;
}

// ---- Promotional credit, taken off before tax ----

/// A closed January with `units` of usage read at draft time, and no credit yet.
async fn usage_period(units: i64) -> (TestEnv, MutableMockUsageClient, SubscriptionId) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    set_period_usage(&usage, units);
    env.process_cycle_transitions().await;
    (env, usage, subscription_id)
}

/// Promotional credit expiring mid-period settles the usage before its expiry ahead of paid
/// credit, and the invoice keeps that settlement as credit taken off before tax.
#[tokio::test]
async fn an_expiring_promotional_grant_settles_first_and_the_invoice_keeps_it_before_tax() {
    use crate::helpers::credit::fund_promotional_grant;
    use diesel_async::SimpleAsyncConnection;
    let (env, usage, subscription_id) = usage_period(150).await;
    let promo = fund_promotional_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(GRANT)
        },
    )
    .await;
    let paid = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(9_000),
    )
    .await;
    let period_start = start_date().and_time(NaiveTime::MIN);
    let expiry = NaiveDate::from_ymd_opt(2024, 1, 15)
        .unwrap()
        .and_time(NaiveTime::MIN);
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            expiry.and_utc().to_rfc3339(),
            promo.as_uuid()
        ))
        .await
        .expect("move the promotional expiry inside the period");
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start,
            period_end: expiry,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: period_start,
                end: expiry,
            },
        },
    );

    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        1
    );
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        period_start.date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(settlements.len(), 1);
    assert_eq!(
        settlements[0].amount, 1_000,
        "the promotional grant pays first"
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.promotional_credits, 1_000);
    assert_eq!(invoice.total, 500);
    assert_eq!(invoice.applied_credits, 500);
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(
        crate::helpers::credit::grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, paid).await,
        8_500
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

// ---- Credit restricted to products ----

/// Credit expiring mid-period settles only the usage of its products: the grant restricted to
/// another product lapses whole, the one restricted to the billed product pays its usage.
#[tokio::test]
async fn an_expiring_restricted_grant_settles_only_its_products() {
    use crate::helpers::credit::fund_product_grant;
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::domain::credit::GrantClassification;
    let (env, usage, subscription_id) = usage_period(150).await;
    let restricted = |amount: i64, product| {
        fund_product_grant(
            env.store(),
            TENANT_ID,
            CUST_UBER_ID,
            Grant {
                expires_at: Some(Utc::now() + Duration::hours(1)),
                ..Grant::invoice(amount)
            },
            GrantClassification::Paid,
            vec![product],
        )
    };
    let elsewhere = restricted(5_000, PRODUCT_DB_STORAGE_ID).await;
    let api_calls = restricted(300, PRODUCT_API_CALLS_ID).await;
    let period_start = start_date().and_time(NaiveTime::MIN);
    let expiry = NaiveDate::from_ymd_opt(2024, 1, 15)
        .unwrap()
        .and_time(NaiveTime::MIN);
    for grant in [elsewhere, api_calls] {
        env.conn()
            .await
            .batch_execute(&format!(
                "SET session_replication_role = replica;
                 UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                     expires_at = '{}' WHERE id = '{}';
                 SET session_replication_role = origin;",
                expiry.and_utc().to_rfc3339(),
                grant.as_uuid()
            ))
            .await
            .expect("move the expiry inside the period");
    }
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start,
            period_end: expiry,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: period_start,
                end: expiry,
            },
        },
    );

    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        1,
        "the API-calls credit was spent; only the other one has value to lapse"
    );
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        period_start.date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 300, "only the API-calls credit pays");
    let lapsed = |grant: CreditGrantId| {
        let env = &env;
        async move {
            let wallet = wallets(env.store(), TENANT_ID, CUST_UBER_ID)
                .await
                .remove(0);
            let (entries, _) = env
                .store()
                .customer_credit_ledger(TENANT_ID, wallet.id, 100, 0)
                .await
                .expect("statement");
            entries
                .iter()
                .filter(|e| e.grant_id == grant && e.kind == LedgerEntryKind::Expiry)
                .map(|e| e.amount_minor)
                .sum::<i64>()
        }
    };
    assert_eq!(lapsed(elsewhere).await, -5_000, "it pays for no usage");
    assert_eq!(lapsed(api_calls).await, 0);

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].total, 1_500);
    assert_eq!(
        invoices[0].applied_credits, 300,
        "the invoice keeps the settlement"
    );
    assert_eq!(invoices[0].amount_due, 1_200);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Paid credit expiring mid-period settles the usage before its expiry ahead of promotional
/// credit that does not expire: the customer's money pays before it lapses. The invoice keeps that
/// settlement and takes promotional credit only off what it did not pay.
#[tokio::test]
async fn expiring_paid_credit_settles_before_promotional_credit_and_never_lapses_paid_usage() {
    use crate::helpers::credit::{fund_promotional_grant, grant_remaining};
    use diesel_async::SimpleAsyncConnection;
    let (env, usage, subscription_id) = usage_period(1_500).await;
    let paid = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(12_000)
        },
    )
    .await;
    let promo = fund_promotional_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(20_000),
    )
    .await;
    let period_start = start_date().and_time(NaiveTime::MIN);
    let expiry = NaiveDate::from_ymd_opt(2024, 1, 15)
        .unwrap()
        .and_time(NaiveTime::MIN);
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z' WHERE id = '{}';
             SET session_replication_role = origin;",
            expiry.and_utc().to_rfc3339(),
            paid.as_uuid(),
            promo.as_uuid()
        ))
        .await
        .expect("move the paid expiry inside the period");
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start,
            period_end: expiry,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(1_000),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: period_start,
                end: expiry,
            },
        },
    );

    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        1
    );
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        period_start.date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(settlements.len(), 1);
    assert_eq!(
        settlements[0].amount, 10_000,
        "the expiring paid credit pays"
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, promo).await,
        20_000
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.subtotal, 15_000);
    assert_eq!(
        invoice.promotional_credits, 5_000,
        "only what the settlement left"
    );
    assert_eq!(invoice.total, 10_000);
    assert_eq!(invoice.applied_credits, 10_000, "the settlement, kept");
    assert_eq!(invoice.amount_due, 0);
    let wallet = wallets(env.store(), TENANT_ID, CUST_UBER_ID)
        .await
        .remove(0);
    let (entries, _) = env
        .store()
        .customer_credit_ledger(TENANT_ID, wallet.id, 100, 0)
        .await
        .expect("statement");
    let of_paid = |kind: LedgerEntryKind| {
        entries
            .iter()
            .filter(|e| e.grant_id == paid && e.kind == kind)
            .map(|e| e.amount_minor)
            .sum::<i64>()
    };
    assert_eq!(
        of_paid(LedgerEntryKind::Expiry),
        -2_000,
        "only the unused paid credit lapses"
    );
    assert_eq!(
        of_paid(LedgerEntryKind::DrawReversal),
        0,
        "no settled usage comes back to it"
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, promo).await,
        15_000
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

// ---- The live balance knows the tax ----

/// A French company under the EU VAT engine (20%).
async fn french_company(env: &TestEnv) -> common_domain::ids::CustomerId {
    let customer_id = common_domain::ids::CustomerId::from_proto(uuid::Uuid::new_v4().to_string())
        .expect("customer id");
    diesel_models::customers::CustomerRowNew {
        id: customer_id,
        name: "Client SAS".to_string(),
        created_at: Some(Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(
            serde_json::to_value(meteroid_store::domain::Address {
                line1: Some("1 Rue de la Paix".to_string()),
                line2: None,
                city: Some("Paris".to_string()),
                country: Some(
                    <common_domain::country::CountryCode as std::str::FromStr>::from_str("FR")
                        .expect("FR"),
                ),
                state: None,
                zip_code: Some("75002".to_string()),
            })
            .expect("address"),
        ),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some("FR12345678901".to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: true,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("customer");
    customer_id
}

/// A French customer's closed January under the EU VAT engine (20%), with `units` of usage read
/// at draft time.
async fn taxed_usage_period(
    units: i64,
) -> (TestEnv, SubscriptionId, common_domain::ids::CustomerId) {
    use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            meteroid_store::domain::InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("EU VAT");
    let customer_id = french_company(&env).await;
    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    set_period_usage(&usage, units);
    env.process_cycle_transitions().await;
    (env, subscription_id, customer_id)
}

/// Paid credit is a means of payment: expiring with the period, it settles the period's usage
/// with its tax, as the invoice bills it. 55.00 against 50.00 of usage at 20% is kept whole, and
/// only the 5.00 it cannot pay is left to cash.
#[tokio::test]
async fn expiring_paid_credit_settles_usage_with_its_tax() {
    use crate::helpers::credit::grant_remaining;
    use diesel_async::SimpleAsyncConnection;
    let (env, subscription_id, customer_id) = taxed_usage_period(500).await;
    let paid = fund_grant(
        env.store(),
        TENANT_ID,
        customer_id,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(5_500)
        },
    )
    .await;
    // Expiring with the period, so all of its usage was consumed before the expiry.
    let expiry = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            expiry.and_utc().to_rfc3339(),
            paid.as_uuid()
        ))
        .await
        .expect("move the expiry to the period end");

    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        0,
        "nothing is left to lapse"
    );
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        start_date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 5_500, "60.00 invoiced, 55.00 held");
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, customer_id, paid).await,
        0
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].total, 6_000);
    assert_eq!(
        invoices[0].applied_credits, 5_500,
        "the settlement, kept whole"
    );
    assert_eq!(invoices[0].amount_due, 500);
}

/// Expiring paid credit settles 30.03 of 50.00 of usage at 20% (60.00 invoiced): that pays 25.02
/// of the usage, the most whose invoiced figure (30.02) it covers. Promotional credit, live since
/// January (only credit effective before the period ended pays its usage), settles the other 24.98
/// before tax in the same settlement; the invoice keeps both, and the paid grant's last cent lapses.
#[tokio::test]
async fn a_partial_paid_settlement_leaves_promotional_credit_the_usage_it_did_not_pay() {
    use crate::helpers::credit::{fund_promotional_grant, grant_remaining};
    use diesel_async::SimpleAsyncConnection;
    let (env, subscription_id, customer_id) = taxed_usage_period(500).await;
    let paid = fund_grant(
        env.store(),
        TENANT_ID,
        customer_id,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(3_003)
        },
    )
    .await;
    let expiry = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            expiry.and_utc().to_rfc3339(),
            paid.as_uuid()
        ))
        .await
        .expect("move the expiry to the period end");
    let promo = fund_promotional_grant(
        env.store(),
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(10_000),
    )
    .await;

    env.store()
        .expire_customer_credit(200)
        .await
        .expect("expiry sweep");
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        start_date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(
        settlements.iter().map(|s| s.amount).collect::<Vec<_>>(),
        vec![3_003 + 2_498],
        "the paid credit's 30.03 and the promotional 24.98, one EUR settlement"
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.promotional_credits, 2_498);
    assert_eq!(invoice.tax_amount, 500, "20% of 25.02, rounded");
    assert_eq!(invoice.total, 3_002);
    assert_eq!(invoice.applied_credits, 3_002, "kept from the settlement");
    assert_eq!(invoice.amount_due, 0);
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, customer_id, promo).await,
        10_000 - 2_498
    );
    let paid = wallets(env.store(), TENANT_ID, customer_id)
        .await
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .find(|grant| grant.id == paid)
        .expect("the paid grant");
    assert_eq!(
        (paid.remaining_minor, paid.expired_minor),
        (0, Some(1)),
        "the cent no usage figure matches is returned and lapses with its grant"
    );
    assert_ledger_balances(env.store(), TENANT_ID, customer_id).await;
}

/// API calls and storage used 0.07 and 0.06 in January at 20%, with paid credit expiring at the
/// period end. Settled product by product, the tax rounded down twice (0.15 settled); the invoice
/// rounds 20% once (0.026 → 0.03) and asks 0.16, the last cent left to cash. The settlement now
/// owes what its invoice asks, which takes it over to the cent (OQ14).
#[tokio::test]
async fn a_settlement_rounds_tax_per_rate_like_the_invoice_taking_it_over() {
    use crate::data::plans::{PlanSeed, SeedComp};
    use crate::helpers::credit::grant_remaining;
    use common_domain::ids::{PlanVersionId, PriceComponentId, PriceId};
    use diesel_async::SimpleAsyncConnection;
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    use meteroid_store::domain::UsagePricingModel;
    use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let version = PlanVersionId::new();
    let cent = UsagePricingModel::PerUnit {
        rate: Decimal::new(1, 2),
    };
    PlanSeed::seed_version(
        &mut env.conn().await,
        PLAN_USAGE_ALPHA_ID,
        version,
        91,
        "EUR",
        &[
            SeedComp::usage(
                PriceComponentId::new(),
                "API Calls",
                PRODUCT_API_CALLS_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                cent.clone(),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "DB Storage",
                PRODUCT_DB_STORAGE_ID,
                METRIC_DATABASE_SIZE,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                cent,
            ),
        ],
    )
    .await
    .expect("two-product usage plan");
    env.set_consolidate_and_grace(false, 720).await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            meteroid_store::domain::InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("EU VAT");
    let customer_id = french_company(&env).await;
    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(version)
        .start_date(start_date())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let (start, end) = (
        start_date().and_time(NaiveTime::MIN),
        NaiveDate::from_ymd_opt(2024, 2, 1)
            .unwrap()
            .and_time(NaiveTime::MIN),
    );
    for (metric, units) in [(METRIC_BANDWIDTH, 7), (METRIC_DATABASE_SIZE, 6)] {
        usage.set_usage(
            MockUsageDataParams {
                metric_id: metric,
                period_start: start,
                period_end: end,
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: std::collections::HashMap::new(),
                }],
                period: UsagePeriod { start, end },
            },
        );
    }
    env.process_cycle_transitions().await;
    let paid = fund_grant(
        env.store(),
        TENANT_ID,
        customer_id,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(1_000)
        },
    )
    .await;
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant SET effective_at = '2024-01-01T00:00:00Z',
                 expires_at = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            end.and_utc().to_rfc3339(),
            paid.as_uuid()
        ))
        .await
        .expect("move the expiry to the period end");

    env.store()
        .expire_customer_credit(200)
        .await
        .expect("expiry sweep");
    let settlements = CreditUsageSettlementRow::list_for_period(
        &mut env.conn().await,
        TENANT_ID,
        subscription_id,
        start_date(),
        "EUR",
    )
    .await
    .expect("settlements");
    assert_eq!(
        settlements
            .iter()
            .map(|settlement| (settlement.amount, settlement.tax_minor))
            .collect::<Vec<_>>(),
        vec![(16, 3)]
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(
        (
            invoice.subtotal,
            invoice.tax_amount,
            invoice.total,
            invoice.applied_credits,
            invoice.amount_due
        ),
        (13, 3, 16, 16, 0)
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, customer_id, paid).await,
        984,
        "what the usage left lapsed"
    );
    assert_ledger_balances(env.store(), TENANT_ID, customer_id).await;
}

/// The rates of the usage windows are asked of the tax engine only to settle credit that lapsed:
/// an engine that cannot answer fails the finalization that settles a lapse, and is never asked
/// otherwise. The draft is edited by hand, so its own lines are not taxed again.
#[tokio::test]
async fn the_window_taxes_are_resolved_only_when_lapsed_credit_settles_them() {
    for lapsed in [true, false] {
        let usage = MutableMockUsageClient::new();
        let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
        seed_usage_only_plan(&env).await;
        env.set_consolidate_and_grace(false, 0).await;
        let subscription_id = subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
            .start_date(start_date())
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
        set_period_usage(&usage, 100);
        env.process_cycle_transitions().await;
        let grant = fund_grant(
            env.store(),
            TENANT_ID,
            CUST_UBER_ID,
            None,
            Grant {
                expires_at: Some(Utc::now() + Duration::hours(1)),
                ..Grant::invoice(GRANT)
            },
        )
        .await;
        if lapsed {
            expire_grant(env.store(), grant).await;
        }
        let draft = env
            .get_invoices(subscription_id)
            .await
            .into_iter()
            .find(|invoice| invoice.status == InvoiceStatusEnum::Draft)
            .expect("the period's draft");
        {
            use diesel_async::SimpleAsyncConnection;
            env.conn()
                .await
                .batch_execute(&format!(
                    "UPDATE invoice SET manually_edited_at = now() WHERE id = '{}';
                     UPDATE customer SET billing_address =
                         '{{\"line1\":\"1 Rue de Rivoli\",\"city\":\"Paris\",\"country\":\"FR\",\"zip_code\":\"75001\"}}'::jsonb
                     WHERE id = '{}';
                     UPDATE invoicing_entity SET tax_resolver = 'EXTERNAL' WHERE tenant_id = '{}';",
                    draft.id.as_uuid(),
                    CUST_UBER_ID.as_uuid(),
                    TENANT_ID.as_uuid()
                ))
                .await
                .expect("an engine that cannot answer");
        }
        let finalized = env
            .services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await;
        if lapsed {
            assert!(finalized.is_err(), "the lapse needs the window rates");
        } else {
            let invoice = finalized.expect("no lapse, no rate asked").invoice;
            assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
            assert_eq!(invoice.applied_credits, 1_000, "100 units at 0.10 EUR");
        }
    }
}

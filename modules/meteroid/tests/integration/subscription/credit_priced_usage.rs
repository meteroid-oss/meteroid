//! Usage priced in a credit unit (Orb's pricing units): the unit's wallet pays first, before tax,
//! and only what it does not cover is billed in money at the price's rate, then taxed.
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{
    AddOnId, BaseId, CreditGrantId, CreditUnitId, CreditUnitPriceId, CustomerId, InvoiceId,
    LedgerEntryId, PriceComponentId, PriceId, SubscriptionId, WalletId,
};
use diesel_models::credit_units::{CreditUnitPriceRowNew, CreditUnitRowNew};
use diesel_models::customer_credit::{
    CreditUsageSettlementRow, CustomerCreditAccountRow, CustomerCreditAccountRowNew,
    CustomerCreditEntryRow, CustomerCreditEntryRowNew, CustomerCreditGrantRow,
    CustomerCreditGrantRowNew,
};
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid::services::credit_note_rendering::{
    CreditNoteGenerateResult, CreditNotePdfRenderingService,
};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::credit_units::PriceCreditUnit;
use meteroid_store::domain::einvoicing::EInvoicingStatus;
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum, InvoiceType, TaxResolverEnum};
use meteroid_store::domain::invoice_lines::LineCreditUnit;
use meteroid_store::domain::price_components::{
    PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{
    Address, BillingPeriodEnum, EInvoicingFormat, Invoice, InvoicingEntityPatch, LineItem,
    UsagePeriod,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use std::str::FromStr;

use super::usage::{
    PLAN_VERSION_USAGE_ONLY_ID, PRICE_USAGE_ONLY_API_CALLS_ID, seed_usage_only_plan,
};
use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::helpers::credit::{Grant, available, expire_grant, fixture_start, fund, fund_grant};
use crate::meteroid_it::container::SeedLevel;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;

const UNIT_CODE: &str = "CREDITS";

fn start_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

/// 3 credits per unit at precision 2: `units` units consume `units * 300` credit minor units.
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
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

struct UnitScenario {
    env: TestEnv,
    usage: MutableMockUsageClient,
    customer_id: CustomerId,
    unit_id: CreditUnitId,
    subscriptions: Vec<SubscriptionId>,
}

struct Options {
    customer: Option<CustomerId>,
    subscriptions: usize,
    consolidate: bool,
    /// Adds a money-priced usage component: DB storage at EUR 0.10 per unit.
    money_component: bool,
    /// The price's own rate per credit; zero leaves it empty.
    rate: Decimal,
    /// The unit's list price in EUR.
    list_price: Option<Decimal>,
    /// Prices the usage in credits through a monthly add-on; the plan's own price stays in money.
    add_on: bool,
    end_date: Option<NaiveDate>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            customer: None,
            subscriptions: 1,
            consolidate: false,
            money_component: false,
            rate: dec!(0.01),
            list_price: None,
            add_on: false,
            end_date: None,
        }
    }
}

/// The usage-only EUR plan, repriced at 3 CREDITS per unit with 1 credit = EUR 0.01, and
/// subscriptions in their first period: `close_period` creates the period's draft.
async fn unit_scenario(options: Options) -> UnitScenario {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(options.consolidate, 0).await;
    let unit_id = CreditUnitId::new();
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::price::dsl as p;
        let mut conn = env.conn().await;
        CreditUnitRowNew {
            id: unit_id,
            tenant_id: TENANT_ID,
            code: UNIT_CODE.to_owned(),
            name: "Credit".to_owned(),
            plural_name: "Credits".to_owned(),
            symbol: None,
            precision: 2,
        }
        .insert(&mut conn)
        .await
        .expect("credit unit");
        let pricing =
            serde_json::to_value(Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(3) }))
                .unwrap();
        if !options.add_on {
            diesel::update(p::price.filter(p::id.eq(PRICE_USAGE_ONLY_API_CALLS_ID)))
                .set((
                    p::pricing.eq(pricing),
                    p::credit_unit_id.eq(Some(unit_id)),
                    p::credit_unit_rate.eq(Some(options.rate)),
                ))
                .execute(&mut conn)
                .await
                .expect("price the usage in credits");
        }
        if let Some(unit_price) = options.list_price {
            CreditUnitPriceRowNew {
                id: CreditUnitPriceId::new(),
                tenant_id: TENANT_ID,
                credit_unit_id: unit_id,
                currency: "EUR".to_owned(),
                unit_price,
            }
            .insert(&mut conn)
            .await
            .expect("the unit's list price");
        }
        if options.money_component {
            PlanSeed::seed_components(
                &mut conn,
                PLAN_VERSION_USAGE_ONLY_ID,
                &[SeedComp::usage(
                    PriceComponentId::new(),
                    "DB Storage",
                    PRODUCT_DB_STORAGE_ID,
                    METRIC_DATABASE_SIZE,
                    PriceId::new(),
                    DieselBillingPeriodEnum::Monthly,
                    UsagePricingModel::PerUnit { rate: dec!(0.10) },
                )],
                "EUR",
            )
            .await
            .expect("money-priced usage component");
        }
    }
    let customer_id = options.customer.unwrap_or(CUST_UBER_ID);
    if options.customer.is_some() {
        create_french_b2b_customer(&env, customer_id).await;
        env.store()
            .patch_invoicing_entity(
                Actor::System,
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("EU VAT on the invoicing entity");
    }
    let add_on_id = if options.add_on {
        Some(credit_priced_add_on(&env, unit_id, options.rate).await)
    } else {
        None
    };
    let mut subscriptions = Vec::new();
    for _ in 0..options.subscriptions {
        let builder = subscription()
            .customer(customer_id)
            .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
            .start_date(start_date())
            .on_start()
            .no_trial();
        let builder = match options.end_date {
            Some(end_date) => builder.end_date(end_date),
            None => builder,
        };
        let subscription_id = builder.create(env.services()).await;
        if let Some(add_on_id) = add_on_id {
            add_to_subscription(&env, subscription_id, add_on_id).await;
        }
        subscriptions.push(subscription_id);
    }
    UnitScenario {
        env,
        usage,
        customer_id,
        unit_id,
        subscriptions,
    }
}

/// A monthly add-on billing bandwidth at 3 credits per unit, each worth `rate`.
async fn credit_priced_add_on(env: &TestEnv, unit_id: CreditUnitId, rate: Decimal) -> AddOnId {
    env.store()
        .create_add_on_from_ref(
            "Bandwidth in credits".to_owned(),
            ProductRef::New {
                name: "Bandwidth in credits".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_BANDWIDTH,
                    model: UsageModel::PerUnit,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(3) }),
                credit_unit: Some(PriceCreditUnit { unit_id, rate }),
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("credit-priced add-on")
        .id
}

async fn add_to_subscription(env: &TestEnv, subscription_id: SubscriptionId, add_on_id: AddOnId) {
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            subscription_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id,
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            start_date(),
        )
        .await
        .expect("add the credit-priced add-on");
}

async fn create_french_b2b_customer(env: &TestEnv, customer_id: CustomerId) {
    use diesel_models::customers::CustomerRowNew;
    let address = Address {
        line1: Some("123 Avenue des Champs-Élysées".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str("FR").expect("country code")),
        state: None,
        zip_code: Some("75008".to_string()),
    };
    let mut conn = env.conn().await;
    CustomerRowNew {
        id: customer_id,
        name: "Credit Units SAS".to_string(),
        created_at: Some(Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).unwrap()),
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
    .insert(&mut conn)
    .await
    .expect("French customer");
}

impl UnitScenario {
    async fn close_period(&self) {
        self.env.process_cycle_transitions().await;
    }

    /// A promotional grant of `amount` credit minor units in the unit's wallet.
    async fn fund_units(&self, amount: i64, expires_at: Option<DateTime<Utc>>) -> CreditGrantId {
        let mut conn = self.env.conn().await;
        let account = CustomerCreditAccountRow::find_or_create(
            &mut conn,
            &CustomerCreditAccountRowNew {
                id: WalletId::new(),
                tenant_id: TENANT_ID,
                customer_id: self.customer_id,
                currency: UNIT_CODE.to_owned(),
                credit_unit_id: Some(self.unit_id),
            },
        )
        .await
        .expect("unit wallet");
        let grant = CustomerCreditGrantRow::insert_new(
            &mut conn,
            &CustomerCreditGrantRowNew {
                id: CreditGrantId::new(),
                tenant_id: TENANT_ID,
                customer_id: self.customer_id,
                account_id: account.id,
                amount,
                remaining: amount,
                priority: 100,
                classification: "PROMOTIONAL".to_owned(),
                effective_at: fixture_start(),
                expires_at,
                source_kind: "MANUAL".to_owned(),
                source_id: uuid::Uuid::now_v7(),
                invoice_id: None,
                line_local_id: None,
                credit_note_id: None,
                reason: "Integration-test credits".to_owned(),
                reference: None,
                created_by: None,
                cost_minor: None,
                cost_currency: None,
                rollover: None,
                subscription_id: None,
                application: "INVOICE".to_owned(),
                ends_at: None,
                end_reason: None,
                pays_usage_from: None,
            },
        )
        .await
        .expect("unit grant")
        .expect("a new grant");
        CustomerCreditEntryRow::insert_new(
            &mut conn,
            &CustomerCreditEntryRowNew {
                id: LedgerEntryId::new(),
                tenant_id: TENANT_ID,
                customer_id: self.customer_id,
                grant_id: grant.id,
                kind: "FUND".to_owned(),
                amount,
                source_id: grant.source_id,
                source_kind: None,
                invoice_id: None,
                credit_note_id: None,
                occurred_at: grant.effective_at,
                reason: None,
                created_by: None,
                cost_minor: None,
                payment_movement_id: None,
            },
        )
        .await
        .expect("fund entry");
        grant.id
    }

    /// Moves a unit grant's expiry `hours` into the past (grant terms are otherwise immutable).
    async fn expire(&self, grant: CreditGrantId, hours: i64) {
        use diesel_async::SimpleAsyncConnection;
        self.env
            .conn()
            .await
            .batch_execute(&format!(
                "SET session_replication_role = replica;
                 UPDATE customer_credit_grant SET expires_at = now() - interval '{hours} hours'
                 WHERE id = '{}';
                 SET session_replication_role = origin;",
                grant.as_uuid()
            ))
            .await
            .expect("expire the grant");
    }

    async fn remaining(&self, grant: CreditGrantId) -> i64 {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_credit_grant::dsl as g;
        let mut conn = self.env.conn().await;
        g::customer_credit_grant
            .filter(g::id.eq(grant))
            .select(g::remaining)
            .first(&mut conn)
            .await
            .expect("grant remaining")
    }

    /// The movements of one grant, oldest first, as (kind, amount, invoice).
    async fn movements(&self, grant: CreditGrantId) -> Vec<(String, i64, Option<InvoiceId>)> {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer_credit_entry::dsl as e;
        let mut conn = self.env.conn().await;
        e::customer_credit_entry
            .filter(e::grant_id.eq(grant))
            .order((e::created_at.asc(), e::id.asc()))
            .select((e::kind, e::amount, e::invoice_id))
            .load::<(String, i64, Option<InvoiceId>)>(&mut conn)
            .await
            .expect("grant movements")
    }

    async fn settlements(&self) -> Vec<CreditUsageSettlementRow> {
        self.settlements_in(UNIT_CODE).await
    }

    async fn settlements_in(&self, currency: &str) -> Vec<CreditUsageSettlementRow> {
        let mut conn = self.env.conn().await;
        let mut all = Vec::new();
        for subscription_id in &self.subscriptions {
            all.extend(
                CreditUsageSettlementRow::list_for_period(
                    &mut conn,
                    TENANT_ID,
                    *subscription_id,
                    start_date(),
                    currency,
                )
                .await
                .expect("settlements"),
            );
        }
        all
    }

    async fn draft(&self) -> Invoice {
        let invoices = self.env.get_invoices(self.subscriptions[0]).await;
        assert_eq!(invoices.len(), 1);
        invoices.into_iter().next().unwrap()
    }

    /// Every due event processed, the subscription's invoice holding a credit-priced line.
    async fn credit_invoice(&self) -> Invoice {
        self.env.process_due_events().await;
        self.env
            .get_invoices(self.subscriptions[0])
            .await
            .into_iter()
            .find(|invoice| invoice.line_items.iter().any(|l| l.credit_unit.is_some()))
            .expect("an invoice with a credit-priced line")
    }

    async fn finalize(&self) -> Invoice {
        self.env.process_due_events().await;
        let invoices = self.env.get_invoices(self.subscriptions[0]).await;
        assert_eq!(invoices.len(), 1);
        invoices.into_iter().next().unwrap()
    }
}

fn credit_line(invoice: &Invoice) -> (&LineItem, &LineCreditUnit) {
    let line = invoice
        .line_items
        .iter()
        .find(|line| line.credit_unit.is_some())
        .expect("a credit-priced line");
    (line, line.credit_unit.as_ref().unwrap())
}

/// The draft estimates what the wallet would cover; finalization draws exactly that under the
/// unit's grant locks, and nothing is billed in money.
#[tokio::test]
async fn usage_fully_covered_by_the_unit_wallet_bills_no_money() {
    let s = unit_scenario(Options::default()).await;
    let grant = s.fund_units(100_000, None).await;
    set_period_usage(&s.usage, 300);
    s.close_period().await;

    let draft = s.draft().await;
    let (line, credit) = credit_line(&draft);
    assert_eq!((credit.consumed, credit.covered), (90_000, 90_000));
    assert_eq!(credit.unit_price, Some(dec!(3)));
    assert_eq!(credit.rate, dec!(0.01));
    assert_eq!(line.amount_subtotal, 0);
    assert_eq!(line.unit_price, None);
    assert_eq!(line.quantity, Some(dec!(300)));
    assert_eq!(s.remaining(grant).await, 100_000, "a draft only estimates");

    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 90_000));
    assert_eq!(line.amount_subtotal, 0);
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (0, 0, 0)
    );
    assert_eq!(s.remaining(grant).await, 10_000);
    assert_eq!(
        s.movements(grant).await[1..].to_vec(),
        vec![("DRAW".to_owned(), -90_000, Some(invoice.id))]
    );
}

/// Credits the wallet does not cover are billed in money at the rate, and only that money is
/// taxed: the covered credits bear no tax. The money wallet still pays the money owed: the
/// invoice's unit draw does not read as a money draw already made.
#[tokio::test]
async fn partially_covered_usage_bills_and_taxes_only_the_overage() {
    let s = unit_scenario(Options {
        customer: Some(CustomerId::new()),
        ..Default::default()
    })
    .await;
    s.close_period().await;
    let grant = s.fund_units(60_000, None).await;
    let money = fund(s.env.store(), TENANT_ID, s.customer_id, None, 100).await;
    set_period_usage(&s.usage, 300);

    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 60_000));
    assert_eq!(
        line.amount_subtotal, 300,
        "300 uncovered credits at EUR 0.01"
    );
    assert_eq!(line.taxable_amount, 300);
    assert_eq!(line.tax_rate, dec!(0.2));
    assert_eq!(line.tax_amount, 60, "20% of the overage only");
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (300, 60, 360)
    );
    assert_eq!((invoice.applied_credits, invoice.amount_due), (100, 260));
    assert_eq!(s.remaining(grant).await, 0);
    assert_eq!(
        s.movements(money).await[1..].to_vec(),
        vec![("DRAW".to_owned(), -100, Some(invoice.id))]
    );
}

/// Without a unit wallet every credit is billed in money; the customer's money wallet then pays
/// that money like any usage, and nothing else.
#[tokio::test]
async fn without_a_unit_wallet_the_money_wallet_pays_only_the_money_overage() {
    let s = unit_scenario(Options::default()).await;
    s.close_period().await;
    let money = fund(s.env.store(), TENANT_ID, s.customer_id, None, 500).await;
    set_period_usage(&s.usage, 300);

    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 0));
    assert_eq!(line.amount_subtotal, 900);
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (900, 500, 400)
    );
    assert_eq!(
        available(s.env.store(), TENANT_ID, s.customer_id, None).await,
        0
    );
    assert_eq!(
        s.movements(money).await[1..].to_vec(),
        vec![("DRAW".to_owned(), -500, Some(invoice.id))]
    );
}

/// A unit grant that expired mid-period paid the usage before its expiry in credits; the invoice
/// takes that settlement over in the unit and draws nothing again.
#[tokio::test]
async fn an_expired_unit_grant_settles_its_usage_in_credits() {
    let s = unit_scenario(Options::default()).await;
    s.close_period().await;
    let grant = s
        .fund_units(500_000, Some(Utc::now() + Duration::hours(1)))
        .await;
    s.expire(grant, 2).await;
    set_period_usage(&s.usage, 100);

    assert_eq!(
        s.env
            .store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        1
    );
    let settlements = s.settlements().await;
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].amount, 30_000, "100 units at 3 credits");
    assert_eq!(settlements[0].invoice_id, None);

    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!(line.amount_subtotal, 0);
    assert_eq!(s.settlements().await[0].invoice_id, Some(invoice.id));
    let draws = s
        .movements(grant)
        .await
        .into_iter()
        .filter(|(kind, _, _)| kind == "DRAW")
        .map(|(_, amount, _)| amount)
        .collect::<Vec<_>>();
    assert_eq!(
        draws,
        vec![-30_000],
        "the settlement is taken over, not drawn again"
    );
}

/// Two subscriptions consolidated into one invoice draw the unit once, against the consolidated
/// invoice; each subscription's lines show what the draw paid of its own usage.
#[tokio::test]
async fn a_consolidated_invoice_draws_the_unit_once_over_every_merged_period() {
    let s = unit_scenario(Options {
        subscriptions: 2,
        consolidate: true,
        ..Default::default()
    })
    .await;
    s.close_period().await;
    let grant = s.fund_units(45_000, None).await;
    set_period_usage(&s.usage, 100);

    s.env.process_due_events().await;
    let parents = s
        .env
        .get_customer_invoices(s.customer_id)
        .await
        .into_iter()
        .filter(|invoice| {
            invoice.subscription_id.is_none() && invoice.invoice_type == InvoiceType::Recurring
        })
        .collect::<Vec<_>>();
    assert_eq!(parents.len(), 1, "both drafts are consolidated");
    let parent = &parents[0];
    assert_eq!(parent.status, InvoiceStatusEnum::Finalized);
    let blocks = parent
        .line_items
        .iter()
        .filter_map(|line| line.credit_unit.as_ref().map(|credit| (line, credit)))
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 2);
    // The shared credit pays each subscription's own lines in turn: it covers one fully, and the
    // other's remainder is billed as 150 uncovered credits at EUR 0.01.
    let mut covered = blocks
        .iter()
        .map(|(line, credit)| (credit.consumed, credit.covered, line.amount_subtotal))
        .collect::<Vec<_>>();
    covered.sort();
    assert_eq!(covered, vec![(30_000, 15_000, 150), (30_000, 30_000, 0)]);
    assert_eq!(parent.total, 150);
    assert_eq!(
        s.movements(grant).await[1..].to_vec(),
        vec![("DRAW".to_owned(), -45_000, Some(parent.id))]
    );
    let mut kept = Vec::new();
    for child in s.env.get_consolidated_children(parent.id).await {
        let (_, credit) = credit_line(&child);
        kept.push(credit.covered);
    }
    kept.sort();
    assert_eq!(
        kept,
        vec![15_000, 30_000],
        "the drafts keep what the parent drew"
    );
}

/// A credit note on a credit-priced line credits its money overage; the credits it drew stay
/// drawn (they come back by voiding the invoice).
#[tokio::test]
async fn a_credit_note_credits_only_the_money_overage() {
    let s = unit_scenario(Options::default()).await;
    s.close_period().await;
    let grant = s.fund_units(60_000, None).await;
    set_period_usage(&s.usage, 300);
    let invoice = s.finalize().await;
    let (line, _) = credit_line(&invoice);
    assert_eq!(line.amount_subtotal, 300);
    // A credit to the balance needs a paid invoice.
    s.env
        .services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::from(invoice.amount_due) / Decimal::from(100),
            invoice.created_at.and_utc().fixed_offset(),
            None,
        )
        .await
        .expect("mark the invoice paid");
    run_once_invoice_orchestration(
        Arc::new(s.env.store().clone()),
        Arc::new(s.env.services().clone()),
    )
    .await;

    let note = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: line.local_id.clone(),
                    amount_cents: 300,
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

    assert!(
        note.line_items
            .iter()
            .all(|line| line.credit_unit.is_none()),
        "a credit note credits money"
    );
    assert_eq!(s.remaining(grant).await, 0, "the drawn credits stay drawn");
    assert_eq!(
        available(s.env.store(), TENANT_ID, s.customer_id, None).await,
        300,
        "the money overage is credited to the balance"
    );
}

/// Voiding the invoice gives the drawn credits back to the unit's grant.
#[tokio::test]
async fn voiding_the_invoice_gives_the_credits_back() {
    let s = unit_scenario(Options::default()).await;
    s.close_period().await;
    let grant = s.fund_units(60_000, None).await;
    set_period_usage(&s.usage, 300);
    let invoice = s.finalize().await;
    assert_eq!(s.remaining(grant).await, 0);

    s.env
        .try_void_invoice(invoice.id)
        .await
        .expect("void the invoice");
    assert_eq!(s.remaining(grant).await, 60_000);
    assert_eq!(
        s.movements(grant).await[1..]
            .iter()
            .map(|(kind, amount, _)| (kind.as_str(), *amount))
            .collect::<Vec<_>>(),
        vec![("DRAW", -60_000), ("DRAW_REVERSAL", 60_000)]
    );
}

/// 1,000 DB storage units at EUR 0.10: EUR 100.
fn set_period_storage(usage: &MutableMockUsageClient, units: i64) {
    let start = start_date().and_time(NaiveTime::MIN);
    let end = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_DATABASE_SIZE,
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

/// A period priced in CREDITS and in EUR, with a grant of each denomination expiring together.
async fn two_denominations_expiring() -> (UnitScenario, CreditGrantId, CreditGrantId) {
    let s = unit_scenario(Options {
        money_component: true,
        ..Default::default()
    })
    .await;
    s.close_period().await;
    let units = s
        .fund_units(50_000, Some(Utc::now() + Duration::hours(1)))
        .await;
    s.expire(units, 2).await;
    let money = fund_grant(
        s.env.store(),
        TENANT_ID,
        s.customer_id,
        None,
        Grant {
            expires_at: Some(Utc::now() + Duration::hours(1)),
            ..Grant::invoice(20_000)
        },
    )
    .await;
    expire_grant(s.env.store(), money).await;
    set_period_usage(&s.usage, 100);
    set_period_storage(&s.usage, 1_000);
    (s, units, money)
}

/// Each denomination pays its own usage: what the CREDITS wallet settled does not count as paid
/// toward the EUR usage (old code subtracted the 30,000 credits from the EUR 100 and skipped it).
#[tokio::test]
async fn an_expiry_settles_each_denomination_against_its_own_settlements() {
    let (s, units, money) = two_denominations_expiring().await;

    assert_eq!(
        s.env
            .store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep"),
        2
    );
    let credits = s.settlements_in(UNIT_CODE).await;
    assert_eq!(
        credits.iter().map(|row| row.amount).collect::<Vec<_>>(),
        vec![30_000],
        "100 units at 3 credits"
    );
    let euros = s.settlements_in("EUR").await;
    assert_eq!(
        euros.iter().map(|row| row.amount).collect::<Vec<_>>(),
        vec![10_000],
        "1,000 units at EUR 0.10"
    );
    assert_eq!((s.remaining(units).await, s.remaining(money).await), (0, 0));

    let invoice = s.finalize().await;
    let (_, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (10_000, 10_000, 0)
    );
}

/// The same holds when the invoice finalizes before the sweep and settles the expiries itself.
#[tokio::test]
async fn an_invoice_finalizing_before_the_sweep_settles_each_denomination() {
    let (s, _, _) = two_denominations_expiring().await;

    let invoice = s.finalize().await;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    let (_, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (10_000, 10_000, 0),
        "the expired EUR credit pays its usage, not the customer"
    );
    let settled = |rows: Vec<CreditUsageSettlementRow>| {
        rows.into_iter()
            .map(|row| (row.amount, row.invoice_id))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        settled(s.settlements_in(UNIT_CODE).await),
        vec![(30_000, Some(invoice.id))]
    );
    assert_eq!(
        settled(s.settlements_in("EUR").await),
        vec![(10_000, Some(invoice.id))]
    );
}

/// A credit-priced line has no money unit price: crediting a quantity of it is refused with the
/// way out, and crediting an amount of its overage works.
#[tokio::test]
async fn a_credit_priced_line_is_credited_by_amount_not_by_quantity() {
    let s = unit_scenario(Options::default()).await;
    s.close_period().await;
    s.fund_units(60_000, None).await;
    set_period_usage(&s.usage, 300);
    let invoice = s.finalize().await;
    let (line, _) = credit_line(&invoice);
    assert_eq!(line.amount_subtotal, 300);
    let note = |item: CreditLineItem| CreateCreditNoteParams {
        invoice_id: invoice.id,
        line_items: vec![item],
        reason: None,
        memo: None,
        credit_type: CreditType::DebtCancellation,
        custom_properties: None,
    };

    let refused = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            note(CreditLineItem::Line {
                local_id: line.local_id.clone(),
                quantity: dec!(300),
            }),
        )
        .await
        .expect_err("a quantity of credit-priced usage has no money price");
    assert!(
        format!("{refused:?}").contains("is priced in CREDITS: credit its money overage by amount"),
        "{refused:?}"
    );

    let credited = s
        .env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            note(CreditLineItem::Amount {
                local_id: line.local_id.clone(),
                amount_cents: 100,
            }),
        )
        .await
        .expect("an amount of the overage is credited");
    assert_eq!(credited.line_items[0].amount_subtotal, -100);
    assert_eq!(credited.line_items[0].credit_unit, None);
}

/// A full credit note on a credit-priced line states its money overage at the effective rate, as a
/// partial one does: quantity × price is the line amount (PEPPOL-EN16931-R120), and the note
/// renders as a PDF with its e-invoice.
#[tokio::test]
async fn a_full_credit_note_states_credit_priced_usage_at_its_effective_rate() {
    let s = unit_scenario(Options {
        customer: Some(CustomerId::new()),
        ..Default::default()
    })
    .await;
    s.env
        .store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                einvoicing_format: Some(Some(EInvoicingFormat::FacturX)),
                vat_number: Some("FR12889123456".to_owned()),
                legal_number: Some(Some("88912345600015".to_owned())),
                address_line1: Some("12 Rue de Rivoli".to_owned()),
                zip_code: Some("75004".to_owned()),
                city: Some("Paris".to_owned()),
                country: Some(CountryCode::from_str("FR").expect("country code")),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("Factur-X on the invoicing entity");
    s.close_period().await;
    s.fund_units(50_000, None).await;
    set_period_usage(&s.usage, 300);
    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 50_000));
    assert_eq!(
        (line.amount_subtotal, line.quantity, line.unit_price),
        (400, Some(dec!(300)), None)
    );

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
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("full credit note");
    let credited = note
        .line_items
        .iter()
        .find(|credited| credited.local_id == line.local_id)
        .expect("the credit-priced line is credited");
    assert_eq!(credited.amount_subtotal, -400);
    assert_eq!(credited.quantity, Some(dec!(300)));
    let unit_price = credited.unit_price.expect("an effective rate");
    assert_eq!((dec!(300) * unit_price).round_dp(2), dec!(4.00));

    s.env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("finalize the credit note");
    let rendering = CreditNotePdfRenderingService::try_new(
        s.env.setup.object_store.clone(),
        Arc::new(s.env.store().clone()),
    )
    .expect("credit note rendering");
    let rendered = rendering
        .generate_pdfs(vec![note.id])
        .await
        .expect("render the credit note");
    assert!(
        matches!(rendered[..], [CreditNoteGenerateResult::Success { .. }]),
        "the PDF renders"
    );
    let note = s
        .env
        .store()
        .get_credit_note_by_id(TENANT_ID, note.id)
        .await
        .expect("credit note");
    assert_eq!(
        note.einvoicing_status,
        Some(EInvoicingStatus::Generated),
        "{:?}",
        note.einvoicing_report
    );
    assert!(note.xml_document_id.is_some());
}

/// The sweep lapsed the expired unit grant after settling its usage; voiding the invoice that took
/// that settlement over gives the 30,000 credits back to the grant, and the void lapses them at
/// once, as it does money. The old code lapsed only the invoice currency, leaving them spendable.
#[tokio::test]
async fn voiding_an_invoice_lapses_what_it_gives_back_to_expired_unit_credit_at_once() {
    // A money line keeps the invoice voidable: with nothing due it would be paid at finalization.
    let s = unit_scenario(Options {
        money_component: true,
        ..Default::default()
    })
    .await;
    s.close_period().await;
    let grant = s
        .fund_units(100_000, Some(Utc::now() + Duration::hours(1)))
        .await;
    s.expire(grant, 2).await;
    set_period_usage(&s.usage, 100);
    set_period_storage(&s.usage, 100);
    let sweep = || async {
        s.env
            .store()
            .expire_customer_credit(200)
            .await
            .expect("expiry sweep")
    };
    assert_eq!(sweep().await, 1);
    let invoice = s.finalize().await;
    let (_, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!(
        invoice.amount_due, 1_000,
        "100 units of storage at EUR 0.10"
    );

    s.env
        .try_void_invoice(invoice.id)
        .await
        .expect("void the invoice");
    let movements = s.movements(grant).await;
    let of = |kind: &str| {
        let mut amounts = movements
            .iter()
            .filter(|(k, _, _)| k == kind)
            .map(|(_, amount, _)| *amount)
            .collect::<Vec<_>>();
        amounts.sort_unstable();
        amounts
    };
    assert_eq!(of("DRAW"), vec![-30_000]);
    assert_eq!(of("DRAW_REVERSAL"), vec![30_000]);
    assert_eq!(
        of("EXPIRY"),
        vec![-70_000, -30_000],
        "the credits given back to the expired grant lapse with the void"
    );
    assert_eq!(s.remaining(grant).await, 0);
    assert_eq!(sweep().await, 0, "nothing is left for the sweep to lapse");
}

/// A price that leaves its rate empty bills the credits no wallet covers at the unit's list
/// price, where the plan invoices usage past zero.
#[tokio::test]
async fn an_empty_rate_bills_the_overage_at_the_unit_list_price() {
    let s = unit_scenario(Options {
        rate: Decimal::ZERO,
        list_price: Some(dec!(0.02)),
        ..Default::default()
    })
    .await;
    s.close_period().await;
    set_period_usage(&s.usage, 300);

    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 0));
    assert_eq!(credit.rate, dec!(0.02));
    assert_eq!(line.amount_subtotal, 1_800, "900 credits at EUR 0.02");
}

/// The customer's agreed price per credit comes before the list price, and before a rate the
/// price sets itself (below).
#[tokio::test]
async fn an_empty_rate_prefers_the_customer_agreed_price() {
    let s = unit_scenario(Options {
        rate: Decimal::ZERO,
        list_price: Some(dec!(0.02)),
        ..Default::default()
    })
    .await;
    s.fund_units(30_000, None).await;
    agree_unit_price(&s, "0.015").await;
    s.close_period().await;
    set_period_usage(&s.usage, 300);

    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 30_000));
    assert_eq!(credit.rate, dec!(0.015));
    assert_eq!(
        line.amount_subtotal, 900,
        "600 uncovered credits at EUR 0.015"
    );
}

#[tokio::test]
async fn the_agreed_price_comes_before_a_rate_the_price_sets() {
    let set = unit_scenario(Options {
        list_price: Some(dec!(0.02)),
        ..Default::default()
    })
    .await;
    set.fund_units(30_000, None).await;
    agree_unit_price(&set, "0.015").await;
    set.close_period().await;
    set_period_usage(&set.usage, 300);
    let invoice = set.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!(credit.rate, dec!(0.015));
    assert_eq!(
        line.amount_subtotal, 900,
        "600 uncovered credits at the agreed EUR 0.015"
    );
}

/// Where the plan denies access at zero, an empty rate stays empty: what still arrives past zero
/// is recorded at no charge, as before.
#[tokio::test]
async fn an_empty_rate_bills_nothing_where_access_is_denied_at_zero() {
    let s = unit_scenario(Options {
        rate: Decimal::ZERO,
        list_price: Some(dec!(0.02)),
        ..Default::default()
    })
    .await;
    {
        use diesel_async::SimpleAsyncConnection;
        s.env
            .conn()
            .await
            .batch_execute(&format!(
                "UPDATE plan_version SET credit_exhausted_rule = 'PAUSE' WHERE id = '{}'",
                PLAN_VERSION_USAGE_ONLY_ID.as_uuid()
            ))
            .await
            .expect("deny access at zero");
    }
    s.close_period().await;
    set_period_usage(&s.usage, 300);

    let invoice = s.finalize().await;
    let (line, credit) = credit_line(&invoice);
    assert_eq!(credit.rate, Decimal::ZERO);
    assert_eq!(line.amount_subtotal, 0);
}

async fn agree_unit_price(s: &UnitScenario, unit_price: &str) {
    use diesel_async::SimpleAsyncConnection;
    s.env
        .conn()
        .await
        .batch_execute(&format!(
            "UPDATE customer_credit_account SET unit_price = {unit_price}, \
             unit_price_currency = 'EUR' WHERE customer_id = '{}' AND credit_unit_id = '{}'",
            s.customer_id.as_uuid(),
            s.unit_id.as_uuid()
        ))
        .await
        .expect("the customer's agreed price");
}

/// A subscription that completed on its end date is still billed past zero under its plan's
/// rule: its final invoice bills the overage at the customer's agreed price, below the price's.
#[tokio::test]
async fn a_completed_subscription_bills_its_overage_at_the_agreed_price() {
    let s = unit_scenario(Options {
        end_date: Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
        ..Default::default()
    })
    .await;
    s.fund_units(30_000, None).await;
    agree_unit_price(&s, "0.005").await;
    set_period_usage(&s.usage, 300);
    s.close_period().await;
    s.close_period().await;

    let invoice = s.credit_invoice().await;
    assert_eq!(
        s.env.get_subscription(s.subscriptions[0]).await.status,
        SubscriptionStatusEnum::Completed
    );
    let (line, credit) = credit_line(&invoice);
    assert_eq!((credit.consumed, credit.covered), (90_000, 30_000));
    assert_eq!(credit.rate, dec!(0.005));
    assert_eq!(credit.price_rate, Some(dec!(0.01)));
    assert_eq!(
        line.amount_subtotal, 300,
        "600 uncovered credits at the agreed EUR 0.005"
    );
}

/// Usage an add-on prices in credits follows the plan's rule like the plan's own prices.
#[tokio::test]
async fn a_credit_priced_add_on_bills_its_overage_at_the_agreed_price() {
    let s = unit_scenario(Options {
        add_on: true,
        ..Default::default()
    })
    .await;
    s.fund_units(30_000, None).await;
    agree_unit_price(&s, "0.005").await;
    s.close_period().await;
    set_period_usage(&s.usage, 300);

    let invoice = s.credit_invoice().await;
    let (line, credit) = credit_line(&invoice);
    assert!(line.sub_add_on_id.is_some());
    assert_eq!((credit.consumed, credit.covered), (90_000, 30_000));
    assert_eq!(credit.rate, dec!(0.005));
    assert_eq!(line.amount_subtotal, 300);
}

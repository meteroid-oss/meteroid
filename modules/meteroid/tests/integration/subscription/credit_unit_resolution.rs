//! A fee priced in a credit unit keeps its unit wherever it is resolved: a component or add-on
//! closed mid-period still bills its last usage in credits, and a preview prices a unit pack as
//! the subscription bills it.
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::actor::Actor;
use common_domain::ids::{BillableMetricId, CreditUnitId, SubscriptionId};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::credit_units::{CreditUnitNew, PriceCreditUnit};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum, InvoiceStatusEnum};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{
    Invoice, LineItem, SubscriptionFee, UpdateInvoiceParams, UsagePeriod, UsagePricingModel,
};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;

use super::credit_unit_wallets::pack_scenario;
use super::usage::{
    PLAN_VERSION_USAGE_ONLY_ID, PRICE_USAGE_ONLY_API_CALLS_ID, seed_usage_only_plan,
};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::helpers::credit::{available, fund};
use crate::meteroid_it::container::SeedLevel;

const UNIT_CODE: &str = "CREDITS";

fn jan(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
}

fn set_usage(
    usage: &MutableMockUsageClient,
    metric_id: BillableMetricId,
    from: NaiveDate,
    to: NaiveDate,
    units: i64,
) {
    let (start, end) = (from.and_time(NaiveTime::MIN), to.and_time(NaiveTime::MIN));
    usage.set_usage(
        MockUsageDataParams {
            metric_id,
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

struct Scenario {
    env: TestEnv,
    usage: MutableMockUsageClient,
    unit_id: CreditUnitId,
    subscription_id: SubscriptionId,
}

/// The usage-only EUR plan priced at 3 CREDITS (precision 2) per API call, 1 credit = EUR 0.01,
/// subscribed on Jan 1 with 1,000.00 credits in the customer's CREDITS wallet. Drafts wait out a
/// 30-day grace so they can be read before they finalize.
async fn scenario() -> Scenario {
    scenario_with_credits(Some(100_000)).await
}

/// `scenario`, funding `credits` (minor units) in the CREDITS wallet when given.
async fn scenario_with_credits(credits: Option<i64>) -> Scenario {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 720).await;
    let unit_id = env
        .store()
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, UNIT_CODE, "Credit", "Credits", None, 2).unwrap(),
        )
        .await
        .expect("credit unit")
        .id;
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::price::dsl as p;
        let pricing =
            serde_json::to_value(Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(3) }))
                .unwrap();
        diesel::update(p::price.filter(p::id.eq(PRICE_USAGE_ONLY_API_CALLS_ID)))
            .set((
                p::pricing.eq(pricing),
                p::credit_unit_id.eq(Some(unit_id)),
                p::credit_unit_rate.eq(Some(dec!(0.01))),
            ))
            .execute(&mut env.conn().await)
            .await
            .expect("price the usage in credits");
    }
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(jan(1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    if let Some(credits) = credits {
        fund(
            env.store(),
            TENANT_ID,
            CUST_UBER_ID,
            Some(UNIT_CODE),
            credits,
        )
        .await;
    }
    Scenario {
        env,
        usage,
        unit_id,
        subscription_id,
    }
}

impl Scenario {
    async fn amend(&self, amendment: SubscriptionAmendment, on: NaiveDate) {
        self.env
            .services()
            .apply_amendment_immediate_at(
                Actor::System,
                self.subscription_id,
                TENANT_ID,
                amendment,
                on,
            )
            .await
            .expect("amendment");
    }

    /// The renewal draft of Feb 1, which bills January's usage.
    async fn renewal_draft(&self) -> Invoice {
        self.env.process_cycle_transitions().await;
        let invoices = self.env.get_invoices(self.subscription_id).await;
        assert_eq!(invoices.len(), 1);
        let draft = invoices.into_iter().next().unwrap();
        assert_eq!(draft.status, InvoiceStatusEnum::Draft);
        draft
    }

    async fn finalized(&self) -> Invoice {
        self.env.process_due_events().await;
        let invoices = self.env.get_invoices(self.subscription_id).await;
        assert_eq!(invoices.len(), 1);
        let invoice = invoices.into_iter().next().unwrap();
        assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
        invoice
    }

    async fn credits_left(&self) -> i64 {
        available(self.env.store(), TENANT_ID, CUST_UBER_ID, Some(UNIT_CODE)).await
    }
}

fn metric_line(invoice: &Invoice, metric_id: BillableMetricId) -> &LineItem {
    invoice
        .line_items
        .iter()
        .find(|line| line.metric_id == Some(metric_id))
        .expect("a line for the metric")
}

/// 100 calls used before the component is removed on Jan 15 are 300.00 credits: the renewal
/// covers them from the wallet and bills no money, like an active component would.
#[tokio::test]
async fn a_component_removed_mid_period_still_bills_its_usage_in_credits() {
    let s = scenario().await;
    set_usage(&s.usage, METRIC_BANDWIDTH, jan(1), jan(15), 100);
    let api_calls = s
        .env
        .get_subscription_components(s.subscription_id)
        .await
        .into_iter()
        .next()
        .expect("the API Calls component");
    s.amend(
        SubscriptionAmendment {
            apply_mode: PlanChangeMode::Immediate,
            component_changes: ComponentChanges {
                edited: vec![],
                added: vec![],
                removed: vec![api_calls.id],
            },
            add_on_changes: AddOnChanges::default(),
        },
        jan(15),
    )
    .await;

    let draft = s.renewal_draft().await;
    let line = metric_line(&draft, METRIC_BANDWIDTH);
    assert_eq!((line.start_date, line.end_date), (jan(1), jan(15)));
    let credit = line
        .credit_unit
        .as_ref()
        .expect("the closed segment is billed in credits");
    assert_eq!(
        (credit.unit_id, credit.rate, credit.unit_price),
        (s.unit_id, dec!(0.01), Some(dec!(3)))
    );
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!(line.amount_subtotal, 0, "nothing is billed in money");
    assert_eq!(s.credits_left().await, 100_000, "a draft only estimates");

    let invoice = s.finalized().await;
    let credit = metric_line(&invoice, METRIC_BANDWIDTH)
        .credit_unit
        .as_ref()
        .expect("still in credits");
    assert_eq!((credit.consumed, credit.covered), (30_000, 30_000));
    assert_eq!((invoice.total, invoice.amount_due), (0, 0));
    assert_eq!(s.credits_left().await, 70_000);
}

/// A storage add-on priced at 2 CREDITS per GB, attached on Jan 1 and removed on Jan 15 after
/// 50 GB: its last segment is 100.00 credits, covered by the wallet.
#[tokio::test]
async fn an_add_on_removed_mid_period_still_bills_its_usage_in_credits() {
    let s = scenario().await;
    set_usage(&s.usage, METRIC_DATABASE_SIZE, jan(1), jan(15), 50);
    let add_on_id = s
        .env
        .store()
        .create_add_on_from_ref(
            "Storage".to_owned(),
            ProductRef::New {
                name: "Storage".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_DATABASE_SIZE,
                    model: UsageModel::PerUnit,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(2) }),
                credit_unit: Some(PriceCreditUnit {
                    unit_id: s.unit_id,
                    rate: dec!(0.01),
                }),
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a usage add-on priced in credits")
        .id;
    s.amend(
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
        jan(1),
    )
    .await;
    let attached = SubscriptionAddOnRow::list_by_subscription_id_active(
        &mut s.env.conn().await,
        &TENANT_ID,
        &s.subscription_id,
    )
    .await
    .expect("active add-ons");
    assert_eq!(attached.len(), 1);
    s.amend(
        SubscriptionAmendment {
            apply_mode: PlanChangeMode::Immediate,
            component_changes: ComponentChanges::default(),
            add_on_changes: AddOnChanges {
                added: vec![],
                edited: vec![],
                removed: vec![attached[0].id],
            },
        },
        jan(15),
    )
    .await;

    let draft = s.renewal_draft().await;
    let line = metric_line(&draft, METRIC_DATABASE_SIZE);
    let credit = line
        .credit_unit
        .as_ref()
        .expect("the closed add-on segment is billed in credits");
    assert_eq!(credit.unit_id, s.unit_id);
    assert_eq!((credit.consumed, credit.covered), (10_000, 10_000));
    assert_eq!(line.amount_subtotal, 0);

    s.finalized().await;
    assert_eq!(s.credits_left().await, 90_000);
}

/// The plan's calls (100, 300.00 credits over the month) and a storage add-on removed on Jan 15
/// (50 GB, 100.00 credits) both start Jan 1. A 350.00-credit grant that expired settled
/// 350.00 of their usage at its sweep, in one settlement of the period: the draft shows it placed
/// on both lines as finalizing places it (100.00 on the add-on, 250.00 on the calls, 50.00 billed
/// at EUR 0.01), never all of it on the first line starting Jan 1.
#[tokio::test]
async fn a_settlement_of_two_lines_starting_together_is_previewed_as_finalized() {
    use crate::helpers::credit::{Grant, expire_grant, fund_grant};
    use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
    let s = scenario_with_credits(None).await;
    set_usage(&s.usage, METRIC_BANDWIDTH, jan(1), feb(1), 100);
    set_usage(&s.usage, METRIC_DATABASE_SIZE, jan(1), jan(15), 50);
    let add_on_id = storage_add_on(&s).await;
    s.amend(
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
        jan(1),
    )
    .await;
    let attached = SubscriptionAddOnRow::list_by_subscription_id_active(
        &mut s.env.conn().await,
        &TENANT_ID,
        &s.subscription_id,
    )
    .await
    .expect("active add-ons");
    s.amend(
        SubscriptionAmendment {
            apply_mode: PlanChangeMode::Immediate,
            component_changes: ComponentChanges::default(),
            add_on_changes: AddOnChanges {
                added: vec![],
                edited: vec![],
                removed: vec![attached[0].id],
            },
        },
        jan(15),
    )
    .await;
    let grant = fund_grant(
        s.env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some(UNIT_CODE),
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..Grant::invoice(35_000)
        },
    )
    .await;
    expire_grant(s.env.store(), grant).await;
    s.env
        .store()
        .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, Utc::now())
        .await
        .expect("the sweep settles the period");

    let covered = |invoice: &Invoice| {
        let of = |metric_id| {
            let line = metric_line(invoice, metric_id);
            let credit = line.credit_unit.as_ref().expect("priced in credits");
            (credit.consumed, credit.covered, line.amount_subtotal)
        };
        (of(METRIC_DATABASE_SIZE), of(METRIC_BANDWIDTH))
    };
    let draft = s.renewal_draft().await;
    assert_eq!(
        covered(&draft),
        ((10_000, 10_000, 0), (30_000, 25_000, 50)),
        "the preview places the settlement as finalizing will"
    );
    let invoice = s.finalized().await;
    assert_eq!(covered(&invoice), covered(&draft));
    assert_eq!(invoice.subtotal, 50);
    assert_eq!(s.credits_left().await, 0);
}

/// A storage add-on priced at 2 CREDITS per GB.
async fn storage_add_on(s: &Scenario) -> common_domain::ids::AddOnId {
    s.env
        .store()
        .create_add_on_from_ref(
            "Storage".to_owned(),
            ProductRef::New {
                name: "Storage".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_DATABASE_SIZE,
                    model: UsageModel::PerUnit,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(2) }),
                credit_unit: Some(PriceCreditUnit {
                    unit_id: s.unit_id,
                    rate: dec!(0.01),
                }),
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a usage add-on priced in credits")
        .id
}

fn feb(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 2, day).unwrap()
}

/// The create-subscription summary prices a unit pack as creation bills it: taxed when sold and
/// granting the unit, not an untaxed money prepayment.
#[tokio::test]
async fn a_subscription_preview_bills_a_unit_pack_as_creation_does() {
    let s = pack_scenario().await;
    let request = subscription()
        .customer(s.customer_id)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .request();
    let (content, details) = s
        .env
        .services()
        .preview_create_subscription(&request, TENANT_ID)
        .await
        .expect("preview");

    let pack = content
        .invoice_lines
        .iter()
        .find(|line| line.name.starts_with("Credit pack"))
        .expect("the pack line");
    assert_eq!(
        pack.credit_grant_unit.as_ref().map(|unit| unit.unit_id),
        Some(s.unit_id)
    );
    assert_eq!(pack.credit_grant_amount, Some(500_000));
    assert!(pack.taxed_at_purchase);
    assert_eq!((pack.amount_subtotal, pack.tax_amount), (4_000, 800));

    let previewed = details
        .price_components
        .iter()
        .find(|component| component.name == "Credit pack")
        .expect("the pack component");
    assert!(matches!(
        &previewed.fee,
        SubscriptionFee::Credit { credit_unit: Some(unit), .. } if unit.unit_id == s.unit_id
    ));
}

fn memo_only(memo: &str) -> UpdateInvoiceParams {
    UpdateInvoiceParams {
        memo: Some(Some(memo.to_owned())),
        reference: None,
        purchase_order: None,
        due_date: None,
        line_items: None,
        discount: None,
        customer_details: None,
        invoicing_entity_id: None,
        annexes: None,
        custom_properties: None,
    }
}

/// Kept refusal (passes on the old code too): what a credit-priced draft's wallet covers is only
/// settled at finalization, and an edited draft finalizes from its stored lines, so its lines and
/// discount stay out of hand edits. Its metadata stays editable and does not freeze the draft.
#[tokio::test]
async fn a_credit_priced_draft_takes_metadata_edits_but_not_billing_edits() {
    let s = scenario().await;
    set_usage(
        &s.usage,
        METRIC_BANDWIDTH,
        jan(1),
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        100,
    );
    let draft = s.renewal_draft().await;
    assert!(metric_line(&draft, METRIC_BANDWIDTH).credit_unit.is_some());

    let discounted = UpdateInvoiceParams {
        discount: Some("1".to_owned()),
        ..memo_only("discounted")
    };
    assert!(
        s.env
            .services()
            .update_draft_invoice(draft.id, TENANT_ID, discounted)
            .await
            .is_err()
    );

    let edited = s
        .env
        .services()
        .update_draft_invoice(draft.id, TENANT_ID, memo_only("PO 42"))
        .await
        .expect("a metadata edit");
    assert_eq!(edited.invoice.memo.as_deref(), Some("PO 42"));
    assert_eq!(
        edited.invoice.manually_edited_at, None,
        "the draft keeps being recomputed"
    );
}

//! **Halyard AI**, a fictional LLM API priced per million tokens and billed from credit. Five
//! plans (Free, Pay as you go, Scale, Studio, Enterprise) and eight accounts, each with months of
//! history kept as data: every balance follows from the invoices, payments and grants it records,
//! and the books it must show at the seed's `as_of` close each history.

use super::common::{customer, metric, org, rate};
use super::domain::{
    AutoTopUpAmount, AutoTopUpSpec, BlockSpec, BudgetSpec, CardBehavior, CardSpec, Collection,
    CreditProgram, CreditUnitSpec, Customer, EntitlementOverrideSpec, FeatureKind, FeatureSpec,
    FeeType, Plan, PriceComponent, PromoCodeSpec, Scenario, UnitLabel, UnitPricing,
};
use super::timeline::{
    Action, ChargeOutcome, DeclineReason, Event, Expected, ExpectedGrant, ExpectedWallet, Expiry,
    GrantRef, History, InvoiceExpectation, SalesTax, Spike, UsageAmount, UsagePlan, UsageTarget,
    When, ago, cycle, cycle_at,
};
use crate::presets::credit_traffic::{
    API_KEY_PROPERTY, MODEL_PROPERTY, MODELS, TOKENS_METRIC, TOKENS_PER_PRICE_UNIT,
    TOKENS_PROPERTY, traffic_for,
};
use chrono::Duration;
use meteroid_store::domain::credit_exhausted::{CreditExhaustedRule, UsageAtZero};
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::price_components::{
    CreditExpiry, CreditRollover, MatrixDimension, MatrixRow, UsagePricingModel,
};
use meteroid_store::domain::{
    BillingMetricAggregateEnum, BillingPeriodEnum, CustomerTaxRate, Dimension, PlanTypeEnum,
    SegmentationMatrix, UnitConversionRoundingEnum,
};
use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;

const USD: &str = "USD";
pub const FREE: &str = "Free";
pub const PAY_AS_YOU_GO: &str = "Pay as you go";
pub const SCALE: &str = "Scale";
pub const STUDIO: &str = "Studio";
pub const ENTERPRISE: &str = "Enterprise";

const TOKENS_FEATURE: &str = "model_tokens";
const MAX_MODEL_FEATURE: &str = "halyard_max";
pub const CREDITS: &str = "HALYARD_CREDITS";
const LAUNCH_CODE: &str = "LAUNCH100";

const MONTHLY_CREDIT_PACK: &str = "Monthly credit pack";
const WELCOME_CREDIT: &str = "Welcome credit";
const INCLUDED_CREDITS: &str = "Included Halyard credits";
const STUDIO_FEE: &str = "Studio plan";

const NEW_YORK_TAX: SalesTax = SalesTax {
    name: "New York sales tax",
    rate: dec!(0.08875),
};
const OHIO_TAX: SalesTax = SalesTax {
    name: "Ohio sales tax",
    rate: dec!(0.08),
};

const LIST_PRICES: [Decimal; 3] = [dec!(2), dec!(12), dec!(60)];
/// Enterprise usage at 20% off list.
const ENTERPRISE_PRICES: [Decimal; 3] = [dec!(1.6), dec!(9.6), dec!(48)];
/// Studio usage in Halyard credits, worth $0.01 each: list price in cents.
const CREDIT_PRICES: [Decimal; 3] = [dec!(200), dec!(1200), dec!(6000)];
const CREDIT_VALUE: Decimal = dec!(0.01);

const MILLION: u64 = 1_000_000;
/// Purchased credit stays spendable this long after payment.
const PURCHASE_TERM_MONTHS: u32 = 12;

fn model_usage(prices: [Decimal; 3], credit_unit: Option<UnitPricing>) -> PriceComponent {
    let rates = MODELS
        .iter()
        .zip(prices)
        .map(|(model, price)| MatrixRow {
            dimension1: MatrixDimension {
                key: MODEL_PROPERTY.to_string(),
                value: (*model).to_string(),
            },
            dimension2: None,
            per_unit_price: price,
        })
        .collect();
    PriceComponent {
        name: "Model usage".to_string(),
        fee: FeeType::Usage {
            metric_code: TOKENS_METRIC.to_string(),
            pricing: UsagePricingModel::Matrix { rates },
            cadence: BillingPeriodEnum::Monthly,
            credit_unit,
        },
    }
}

fn credit(
    name: &str,
    unit_price: Decimal,
    credit_amount: Decimal,
    expiry: CreditExpiry,
    credit_unit: Option<&str>,
    rollover: Option<CreditRollover>,
    once: bool,
) -> PriceComponent {
    PriceComponent {
        name: name.to_string(),
        fee: FeeType::Credit {
            unit_price,
            quantity: 1,
            credit_amount,
            expiry: Some(expiry),
            credit_unit: credit_unit.map(str::to_string),
            rollover,
            once,
        },
    }
}

fn plans() -> Vec<Plan> {
    let card_on_receipt = Collection::ChargeAutomatically;
    vec![
        Plan::new(FREE, USD, PlanTypeEnum::Free, vec![])
            .with_hard_cycle_limit(TOKENS_FEATURE, dec!(5000000), 80, None)
            .with_feature(MAX_MODEL_FEATURE, false)
            .with_self_service_rank(1),
        Plan::new(
            PAY_AS_YOU_GO,
            USD,
            PlanTypeEnum::Standard,
            vec![model_usage(LIST_PRICES, None)],
        )
        .with_payment(0, card_on_receipt)
        .with_self_service_rank(2)
        .with_credit_exhausted_rule(CreditExhaustedRule::Invoice),
        Plan::new(
            SCALE,
            USD,
            PlanTypeEnum::Standard,
            vec![
                credit(
                    MONTHLY_CREDIT_PACK,
                    dec!(500),
                    dec!(550),
                    CreditExpiry::BillingPeriod,
                    None,
                    None,
                    false,
                ),
                credit(
                    WELCOME_CREDIT,
                    Decimal::ZERO,
                    dec!(50),
                    CreditExpiry::Day(90),
                    None,
                    None,
                    true,
                ),
                model_usage(LIST_PRICES, None),
            ],
        )
        .with_payment(0, card_on_receipt)
        .with_self_service_rank(3)
        .with_credit_exhausted_rule(CreditExhaustedRule::Pause),
        Plan::new(
            STUDIO,
            USD,
            PlanTypeEnum::Standard,
            vec![
                rate(STUDIO_FEE, dec!(49)),
                credit(
                    INCLUDED_CREDITS,
                    Decimal::ZERO,
                    dec!(5000),
                    CreditExpiry::BillingPeriod,
                    Some(CREDITS),
                    Some(CreditRollover {
                        max_amount: Some(dec!(1000)),
                        expiry: CreditExpiry::Month(1),
                    }),
                    false,
                ),
                model_usage(
                    CREDIT_PRICES,
                    Some(UnitPricing {
                        credit_unit: CREDITS.to_string(),
                        rate: CREDIT_VALUE,
                    }),
                ),
            ],
        )
        .with_payment(0, card_on_receipt)
        .with_self_service_rank(4)
        .with_credit_exhausted_rule(CreditExhaustedRule::Pause),
        Plan::new(
            ENTERPRISE,
            USD,
            PlanTypeEnum::Standard,
            vec![model_usage(ENTERPRISE_PRICES, None)],
        )
        .with_minimum(dec!(5000))
        .with_credit_exhausted_rule(CreditExhaustedRule::Invoice)
        .with_payment(
            30,
            Collection::SendInvoice {
                bank_transfer: true,
            },
        ),
    ]
}

pub fn ai_credits_scenario() -> Scenario {
    let mut tokens = metric(
        TOKENS_METRIC,
        "Model tokens",
        BillingMetricAggregateEnum::Sum,
        TOKENS_PROPERTY,
    );
    tokens.unit_label = Some(UnitLabel {
        singular: "token",
        plural: "tokens",
    });
    tokens.unit_conversion_factor = Some(TOKENS_PER_PRICE_UNIT);
    tokens.unit_conversion_rounding = Some(UnitConversionRoundingEnum::None);
    tokens.segmentation_matrix = Some(SegmentationMatrix::Single(Dimension {
        key: MODEL_PROPERTY.to_string(),
        values: MODELS.iter().map(|m| (*m).to_string()).collect(),
    }));

    let mut halyard = org("US-HALYARD-AI", "San Francisco", "94107");
    halyard.invoice_footer_info = Some("Questions about this invoice: billing@halyard.test".into());
    halyard.country = Some("US");
    halyard.state = Some("CA");
    halyard.tax_resolver = Some(TaxResolverEnum::Manual);

    Scenario {
        name: "Halyard AI".to_string(),
        metrics: vec![tokens],
        features: vec![
            FeatureSpec {
                code: TOKENS_FEATURE.to_string(),
                name: "Model tokens".to_string(),
                kind: FeatureKind::Metered {
                    metric_code: TOKENS_METRIC.to_string(),
                    unlimited_by_default: true,
                },
            },
            FeatureSpec {
                code: MAX_MODEL_FEATURE.to_string(),
                name: "halyard-max".to_string(),
                kind: FeatureKind::Boolean {
                    default_enabled: true,
                },
            },
        ],
        plans: plans(),
        customers: customers(),
        add_ons: vec![],
        quotes: vec![],
        organization: Some(halyard),
        credits: Some(CreditProgram {
            usage_metric_code: TOKENS_METRIC.to_string(),
        }),
        credit_units: vec![CreditUnitSpec {
            code: CREDITS,
            name: "Halyard credit",
            plural_name: "Halyard credits",
            symbol: None,
            precision: 0,
            selling_prices: vec![(USD, CREDIT_VALUE)],
        }],
        promo_codes: vec![PromoCodeSpec {
            code: LAUNCH_CODE,
            name: "Launch credit",
            amount: dec!(100),
            currency: USD,
            expires_after_days: 25,
        }],
        reporting_currency: Some(USD.to_string()),
    }
}

fn card(brand: &'static str, last4: &'static str, behavior: CardBehavior) -> CardSpec {
    CardSpec {
        brand,
        last4,
        behavior,
        expires_in_months: 34,
    }
}

fn account(name: &str, city: &str, state: &str, history: History) -> Customer {
    let mut account = customer(name, USD, "US", city, vec![]);
    if let Some(address) = account.billing_address.as_mut() {
        address.state = Some(state.to_string());
    }
    account.tax_rates = history
        .sales_tax
        .iter()
        .map(|tax| CustomerTaxRate {
            tax_code: "SALES_TAX".to_string(),
            name: tax.name.to_string(),
            rate: tax.rate,
        })
        .collect();
    account.history = Some(history);
    account
}

/// Usage generation for `alias`: its traffic profile's mix and keys, ramping by `growth`.
fn usage_plan(alias: &str, growth: f64, targets: Vec<UsageTarget>) -> UsagePlan {
    let traffic = traffic_for(alias).expect("a traffic profile for every Halyard account");
    UsagePlan {
        mix: traffic.model_mix,
        api_keys: traffic.api_keys,
        monthly_growth: growth,
        runs_per_hour: traffic.runs_per_hour,
        targets,
    }
}

fn m(millions: f64) -> u64 {
    (millions * MILLION as f64).round() as u64
}

fn ev(at: When, action: Action) -> Event {
    Event::new(at, action)
}

fn add_card(card_handle: &'static str, spec: CardSpec) -> Action {
    Action::AddCard {
        card: card_handle,
        spec,
        make_default: true,
    }
}

fn subscribe(plan: &'static str) -> Action {
    Action::Subscribe {
        plan,
        ends: None,
        billing_day_anchor: None,
    }
}

fn bill(invoice: &'static str, expected: InvoiceExpectation) -> Action {
    Action::BillCycle { invoice, expected }
}

/// A usage invoice paid from credit, with nothing left to collect.
fn usage_bill(invoice: &'static str, usage: Decimal, applied: Vec<(GrantRef, Decimal)>) -> Action {
    bill(
        invoice,
        InvoiceExpectation {
            usage,
            applied,
            ..InvoiceExpectation::empty()
        },
    )
}

fn charge(payable: &'static str) -> Action {
    Action::Charge {
        payable,
        outcome: ChargeOutcome::Paid,
    }
}

fn purchase(purchase: &'static str, amount: Decimal) -> Action {
    Action::Purchase {
        purchase,
        denomination: USD,
        amount,
        expires_in_months: Some(PURCHASE_TERM_MONTHS),
    }
}

/// An automatic top-up adds credit that never expires.
fn top_up(purchase: &'static str, amount: Decimal, outcome: ChargeOutcome) -> Action {
    Action::AutoTopUp {
        purchase,
        denomination: USD,
        amount,
        expires_in_months: None,
        outcome,
    }
}

fn wallet_rules(
    denomination: &'static str,
    low_balance: &[Decimal],
    usage_at_zero: UsageAtZero,
    auto_top_up: Option<AutoTopUpSpec>,
) -> Action {
    Action::ConfigureWallet {
        denomination,
        low_balance_thresholds: low_balance.to_vec(),
        usage_at_zero,
        auto_top_up,
    }
}

fn pack(invoice: &'static str, component: &'static str) -> GrantRef {
    GrantRef::PlanCredit { invoice, component }
}

/// The bonus credit a pack sold below its face granted beside its paid credit.
fn bonus(invoice: &'static str, component: &'static str) -> GrantRef {
    GrantRef::PlanBonus { invoice, component }
}

/// Expiry of credit paid at `paid`, `months` later.
fn months_after(paid: When, months: u32) -> Option<Expiry> {
    Some(Expiry::After {
        from: paid,
        expiry: CreditExpiry::Month(months),
    })
}

fn grant(source: GrantRef, remaining: Decimal, expires: Option<Expiry>) -> ExpectedGrant {
    ExpectedGrant {
        source,
        remaining,
        expires,
    }
}

fn usd_wallet(
    on_account: Decimal,
    unbilled: Decimal,
    grants: Vec<ExpectedGrant>,
) -> ExpectedWallet {
    ExpectedWallet {
        denomination: USD,
        on_account,
        unbilled,
        balance: on_account - unbilled,
        grants,
    }
}

fn now() -> When {
    ago(Duration::zero())
}

fn customers() -> Vec<Customer> {
    vec![
        tessera_health(),
        brightloom_labs(),
        quillbase(),
        nightjar_robotics(),
        ferrous_logistics(),
        lumen_notes(),
        wren_labs(),
        meridian_freight(),
    ]
}

/// Scale since three cycles ago, charged by Amex as each invoice is issued. Each $500 pack adds
/// $500 of paid credit and $50 of bonus credit that lapse with the period, after paying its usage;
/// the bonus pays first. The $50 welcome credit pays the busy second period's usage over $550.
fn tessera_health() -> Customer {
    let scale_bill = |invoice, usage, applied, amount_due| {
        bill(
            invoice,
            InvoiceExpectation {
                advance: dec!(500),
                usage,
                applied,
                amount_due,
                ..InvoiceExpectation::empty()
            },
        )
    };
    let history = History {
        cycle_age_days: 9,
        sales_tax: None,
        events: vec![
            ev(
                cycle(3).plus_minutes(-10),
                add_card("amex", card("amex", "0005", CardBehavior::Approved)),
            ),
            ev(cycle(3), subscribe(SCALE)),
            ev(
                cycle(3),
                scale_bill("inv-1", Decimal::ZERO, vec![], dec!(500)),
            ),
            ev(cycle(3), charge("inv-1")),
            ev(
                cycle(3).plus_minutes(1),
                wallet_rules(USD, &[dec!(100)], UsageAtZero::FollowPlan, None),
            ),
            ev(
                cycle(2),
                scale_bill(
                    "inv-2",
                    dec!(380),
                    vec![
                        (bonus("inv-1", MONTHLY_CREDIT_PACK), dec!(50)),
                        (pack("inv-1", MONTHLY_CREDIT_PACK), dec!(330)),
                    ],
                    dec!(500),
                ),
            ),
            ev(cycle(2), charge("inv-2")),
            ev(
                cycle(1),
                scale_bill(
                    "inv-3",
                    dec!(610),
                    vec![
                        (bonus("inv-2", MONTHLY_CREDIT_PACK), dec!(50)),
                        (pack("inv-2", MONTHLY_CREDIT_PACK), dec!(500)),
                        (pack("inv-1", WELCOME_CREDIT), dec!(50)),
                    ],
                    dec!(510),
                ),
            ),
            ev(cycle(1), charge("inv-3")),
            ev(
                cycle(0),
                scale_bill(
                    "inv-4",
                    dec!(465),
                    vec![
                        (bonus("inv-3", MONTHLY_CREDIT_PACK), dec!(50)),
                        (pack("inv-3", MONTHLY_CREDIT_PACK), dec!(415)),
                    ],
                    dec!(500),
                ),
            ),
            ev(cycle(0), charge("inv-4")),
        ],
        usage: usage_plan(
            "tessera-health",
            1.03,
            vec![
                UsageTarget::new(
                    cycle(3),
                    cycle(2),
                    UsageAmount::Tokens([m(4.0), m(16.0), m(3.0)]),
                ),
                UsageTarget::new(
                    cycle(2),
                    cycle(1),
                    UsageAmount::Tokens([m(3.5), m(19.0), m(6.25)]),
                ),
                UsageTarget::new(
                    cycle(1),
                    cycle(0),
                    UsageAmount::Tokens([m(4.5), m(20.5), m(3.5)]),
                ),
                UsageTarget::new(
                    cycle(0),
                    now(),
                    UsageAmount::Tokens([m(1.5), m(6.0), m(1.25)]),
                ),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(550),
                dec!(150),
                vec![
                    grant(
                        pack("inv-4", MONTHLY_CREDIT_PACK),
                        dec!(500),
                        Some(Expiry::At(cycle(-1))),
                    ),
                    grant(
                        bonus("inv-4", MONTHLY_CREDIT_PACK),
                        dec!(50),
                        Some(Expiry::At(cycle(-1))),
                    ),
                ],
            )],
            ..Expected::default()
        },
    };
    account("Tessera Health", "Boston", "MA", history)
}

/// Pay as you go and growing: two $1,000 purchases, then an automatic top-up back to $1,500
/// whenever the balance falls to $400, each of about $1,100.
fn brightloom_labs() -> Customer {
    let bought = |at: When| at.plus_minutes(1);
    let p1 = cycle_at(4, 0, 10, 0);
    let p2 = cycle_at(3, -3, 10, 0);
    let tu1 = cycle_at(2, 6, 10, 0);
    let tu2 = cycle_at(1, 9, 10, 0);
    let tu3 = cycle_at(0, 7, 10, 0);
    let paid = ChargeOutcome::Paid;
    let history = History {
        cycle_age_days: 27,
        sales_tax: None,
        events: vec![
            ev(
                cycle(4).plus_minutes(-10),
                add_card("visa", card("visa", "4242", CardBehavior::Approved)),
            ),
            ev(cycle(4), subscribe(PAY_AS_YOU_GO)),
            ev(cycle(4), bill("inv-1", InvoiceExpectation::empty())),
            ev(p1, purchase("P1", dec!(1000))),
            ev(bought(p1), charge("P1")),
            ev(p2, purchase("P2", dec!(1000))),
            ev(bought(p2), charge("P2")),
            ev(
                bought(p2).plus_minutes(1),
                wallet_rules(
                    USD,
                    &[],
                    UsageAtZero::FollowPlan,
                    Some(AutoTopUpSpec {
                        threshold: dec!(400),
                        amount: AutoTopUpAmount::ToTarget(dec!(1500)),
                        daily_cap: None,
                        monthly_cap: Some(dec!(6000)),
                    }),
                ),
            ),
            ev(
                cycle(3),
                usage_bill(
                    "inv-2",
                    dec!(620),
                    vec![(GrantRef::Purchase("P1"), dec!(620))],
                ),
            ),
            ev(
                cycle(2),
                usage_bill(
                    "inv-3",
                    dec!(780),
                    vec![
                        (GrantRef::Purchase("P1"), dec!(380)),
                        (GrantRef::Purchase("P2"), dec!(400)),
                    ],
                ),
            ),
            ev(tu1, top_up("TU1", dec!(1100), paid)),
            ev(
                cycle(1),
                usage_bill(
                    "inv-4",
                    dec!(940),
                    vec![
                        (GrantRef::Purchase("P2"), dec!(600)),
                        (GrantRef::Purchase("TU1"), dec!(340)),
                    ],
                ),
            ),
            ev(tu2, top_up("TU2", dec!(1100), paid)),
            ev(
                cycle(0),
                usage_bill(
                    "inv-5",
                    dec!(1150),
                    vec![
                        (GrantRef::Purchase("TU1"), dec!(760)),
                        (GrantRef::Purchase("TU2"), dec!(390)),
                    ],
                ),
            ),
            ev(tu3, top_up("TU3", dec!(1100), paid)),
        ],
        usage: usage_plan(
            "brightloom-labs",
            1.2,
            vec![
                UsageTarget::new(
                    bought(p1).plus_minutes(4),
                    cycle(3),
                    UsageAmount::Priced(dec!(620)),
                )
                .with_checkpoint(p2, dec!(560)),
                UsageTarget::new(cycle(3), cycle(2), UsageAmount::Priced(dec!(780))),
                UsageTarget::new(cycle(2), cycle(1), UsageAmount::Priced(dec!(940)))
                    .with_checkpoint(tu1, dec!(200)),
                UsageTarget::new(cycle(1), cycle(0), UsageAmount::Priced(dec!(1150)))
                    .with_checkpoint(tu2, dec!(360)),
                UsageTarget::new(cycle(0), now(), UsageAmount::Priced(dec!(1188)))
                    .with_checkpoint(tu3, dec!(310)),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(1810),
                dec!(1188),
                vec![
                    grant(GrantRef::Purchase("TU2"), dec!(710), None),
                    grant(GrantRef::Purchase("TU3"), dec!(1100), None),
                ],
            )],
            ..Expected::default()
        },
    };
    account("Brightloom Labs", "Austin", "TX", history)
}

/// A $500 purchase declined on an expired card, paid the next day once the card was updated; a
/// runaway batch job, a launch promotion code, a per-key token limit and a soft budget.
fn quillbase() -> Customer {
    let p1 = cycle_at(2, 0, 10, 0);
    let retried = cycle_at(2, 1, 9, 0);
    let redeemed = cycle_at(0, 0, 10, 0);
    let expired_card = CardSpec {
        expires_in_months: -3,
        ..card("mastercard", "4444", CardBehavior::Expired)
    };
    let history = History {
        cycle_age_days: 5,
        sales_tax: None,
        events: vec![
            ev(
                cycle(2).plus_minutes(-10),
                add_card("mastercard-old", expired_card),
            ),
            ev(cycle(2), subscribe(PAY_AS_YOU_GO)),
            ev(cycle(2), bill("inv-1", InvoiceExpectation::empty())),
            ev(p1, purchase("P1", dec!(500))),
            ev(
                p1.plus_minutes(1),
                Action::Charge {
                    payable: "P1",
                    outcome: ChargeOutcome::Declined(DeclineReason::CardExpired),
                },
            ),
            ev(
                cycle_at(2, 0, 18, 30),
                add_card(
                    "mastercard",
                    card("mastercard", "4444", CardBehavior::Approved),
                ),
            ),
            ev(
                cycle_at(2, 0, 18, 31),
                Action::RemoveCard {
                    card: "mastercard-old",
                },
            ),
            ev(retried, charge("P1")),
            ev(
                retried.plus_minutes(1),
                wallet_rules(USD, &[dec!(75)], UsageAtZero::FollowPlan, None),
            ),
            ev(
                retried.plus_minutes(2),
                Action::SetEntitlementOverride(EntitlementOverrideSpec {
                    feature_code: TOKENS_FEATURE.to_string(),
                    limit: dec!(10000000),
                    group_by: Some(API_KEY_PROPERTY.to_string()),
                    block: Some(BlockSpec {
                        grace_pct: Some(10),
                    }),
                    thresholds: vec![80, 100],
                }),
            ),
            ev(
                retried.plus_minutes(3),
                Action::SetBudget(BudgetSpec {
                    amount: dec!(300),
                    hard: false,
                }),
            ),
            ev(
                cycle(1),
                usage_bill(
                    "inv-2",
                    dec!(165),
                    vec![(GrantRef::Purchase("P1"), dec!(165))],
                ),
            ),
            ev(
                cycle(0),
                usage_bill(
                    "inv-3",
                    dec!(240),
                    vec![(GrantRef::Purchase("P1"), dec!(240))],
                ),
            ),
            ev(redeemed, Action::RedeemPromoCode { code: LAUNCH_CODE }),
        ],
        usage: usage_plan(
            "quillbase",
            1.05,
            vec![
                UsageTarget::new(
                    retried.plus_minutes(5),
                    cycle(1),
                    UsageAmount::Priced(dec!(165)),
                )
                .with_spike(Spike {
                    from: cycle_at(2, 21, 0, 0),
                    to: cycle_at(2, 24, 0, 0),
                    amount: dec!(90),
                    api_key: "prod-batch",
                }),
                UsageTarget::new(cycle(1), cycle(0), UsageAmount::Priced(dec!(240))),
                UsageTarget::new(cycle(0), now(), UsageAmount::Priced(dec!(40))),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(195),
                dec!(40),
                vec![
                    grant(
                        GrantRef::PromoCode(LAUNCH_CODE),
                        dec!(100),
                        Some(Expiry::After {
                            from: redeemed,
                            expiry: CreditExpiry::Day(25),
                        }),
                    ),
                    grant(
                        GrantRef::Purchase("P1"),
                        dec!(95),
                        months_after(retried, PURCHASE_TERM_MONTHS),
                    ),
                ],
            )],
            ..Expected::default()
        },
    };
    account("Quillbase", "Brooklyn", "NY", history)
}

/// Blocked once credit runs out: a $250 purchase, now spent down to $0 with usage paused.
fn nightjar_robotics() -> Customer {
    let p1 = cycle_at(1, 0, 10, 0);
    let paused = ago(Duration::hours(5));
    let low = ago(Duration::days(3));
    let history = History {
        cycle_age_days: 21,
        sales_tax: None,
        events: vec![
            ev(
                cycle(1).plus_minutes(-10),
                add_card("visa", card("visa", "1881", CardBehavior::Approved)),
            ),
            ev(cycle(1), subscribe(PAY_AS_YOU_GO)),
            ev(cycle(1), bill("inv-1", InvoiceExpectation::empty())),
            ev(p1, purchase("P1", dec!(250))),
            ev(p1.plus_minutes(1), charge("P1")),
            ev(
                p1.plus_minutes(2),
                wallet_rules(USD, &[dec!(25)], UsageAtZero::AlwaysPause, None),
            ),
            ev(
                cycle(0),
                usage_bill(
                    "inv-2",
                    dec!(124),
                    vec![(GrantRef::Purchase("P1"), dec!(124))],
                ),
            ),
        ],
        usage: usage_plan(
            "nightjar-robotics",
            1.0,
            vec![
                UsageTarget::new(p1.plus_minutes(5), cycle(0), UsageAmount::Priced(dec!(124))),
                UsageTarget::new(cycle(0), paused, UsageAmount::Priced(dec!(126)))
                    .with_checkpoint(low, dec!(101)),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(126),
                dec!(126),
                vec![grant(
                    GrantRef::Purchase("P1"),
                    dec!(126),
                    months_after(p1.plus_minutes(1), PURCHASE_TERM_MONTHS),
                )],
            )],
            usage_paused_at: Some(paused),
            low_balance_at: Some(low),
            ..Expected::default()
        },
    };
    account("Nightjar Robotics", "Pittsburgh", "PA", history)
}

/// A fixed $250 top-up below $100, armed on a card later swapped for one that is hard declined:
/// the first top-up fails and the rule is suspended.
fn ferrous_logistics() -> Customer {
    let p1 = cycle_at(2, 0, 10, 0);
    let low = cycle_at(0, 2, 10, 0);
    let history = History {
        cycle_age_days: 24,
        sales_tax: None,
        events: vec![
            ev(
                cycle(2).plus_minutes(-10),
                add_card("visa-4242", card("visa", "4242", CardBehavior::Approved)),
            ),
            ev(cycle(2), subscribe(PAY_AS_YOU_GO)),
            ev(cycle(2), bill("inv-1", InvoiceExpectation::empty())),
            ev(p1, purchase("P1", dec!(250))),
            ev(p1.plus_minutes(1), charge("P1")),
            ev(
                p1.plus_minutes(2),
                wallet_rules(
                    USD,
                    &[dec!(100)],
                    UsageAtZero::FollowPlan,
                    Some(AutoTopUpSpec {
                        threshold: dec!(100),
                        amount: AutoTopUpAmount::Fixed(dec!(250)),
                        daily_cap: None,
                        monthly_cap: Some(dec!(1000)),
                    }),
                ),
            ),
            ev(
                cycle(1),
                usage_bill(
                    "inv-2",
                    dec!(64),
                    vec![(GrantRef::Purchase("P1"), dec!(64))],
                ),
            ),
            ev(
                cycle_at(1, 17, 11, 0),
                add_card(
                    "visa-0002",
                    card("visa", "0002", CardBehavior::HardDeclined),
                ),
            ),
            ev(
                cycle(0),
                usage_bill(
                    "inv-3",
                    dec!(78),
                    vec![(GrantRef::Purchase("P1"), dec!(78))],
                ),
            ),
            ev(
                low,
                top_up(
                    "TU1",
                    dec!(250),
                    ChargeOutcome::Declined(DeclineReason::HardDecline),
                ),
            ),
        ],
        usage: usage_plan(
            "ferrous-logistics",
            1.0,
            vec![
                UsageTarget::new(p1.plus_minutes(5), cycle(1), UsageAmount::Priced(dec!(64))),
                UsageTarget::new(cycle(1), cycle(0), UsageAmount::Priced(dec!(78))),
                UsageTarget::new(cycle(0), now(), UsageAmount::Priced(dec!(72)))
                    .with_checkpoint(low, dec!(8)),
            ],
        ),
        expected: Expected {
            wallets: vec![usd_wallet(
                dec!(108),
                dec!(72),
                vec![grant(
                    GrantRef::Purchase("P1"),
                    dec!(108),
                    months_after(p1.plus_minutes(1), PURCHASE_TERM_MONTHS),
                )],
            )],
            auto_top_up_suspended: true,
            low_balance_at: Some(low),
            ..Expected::default()
        },
    };
    account("Ferrous Logistics", "Chicago", "IL", history)
}

/// Studio: a $49 fee includes 5,000 Halyard credits each cycle, expiring at its end, up to 1,000
/// rolling over for one more month; usage past them is $0.01 a credit. Taxed in New York, 2,000
/// more credits bought this cycle, plus a $25 service credit.
fn lumen_notes() -> Customer {
    let paid = |back: i32| cycle(back).plus_minutes(2);
    let credits = |invoice| pack(invoice, INCLUDED_CREDITS);
    // The $49 fee and any overage are taxed; included credits are not.
    let studio_bill = |invoice, usage, applied, overage: Decimal, tax: Decimal| {
        bill(
            invoice,
            InvoiceExpectation {
                advance: dec!(49),
                usage,
                overage,
                applied,
                amount_due: dec!(49) + overage,
                tax,
                ..InvoiceExpectation::empty()
            },
        )
    };
    let service_credit_expiry = cycle_at(-4, -15, 0, 0);
    let bought = cycle_at(0, 3, 14, 0);
    let history = History {
        cycle_age_days: 23,
        sales_tax: Some(NEW_YORK_TAX),
        events: vec![
            ev(
                cycle(2).plus_minutes(-10),
                add_card("visa", card("visa", "4242", CardBehavior::Approved)),
            ),
            ev(cycle(2), subscribe(STUDIO)),
            ev(
                cycle(2),
                studio_bill("inv-1", Decimal::ZERO, vec![], Decimal::ZERO, dec!(4.35)),
            ),
            ev(paid(2), charge("inv-1")),
            ev(
                paid(2).plus_minutes(1),
                Action::SetEntitlementOverride(EntitlementOverrideSpec {
                    feature_code: TOKENS_FEATURE.to_string(),
                    limit: dec!(10000000),
                    group_by: Some(API_KEY_PROPERTY.to_string()),
                    block: Some(BlockSpec { grace_pct: None }),
                    thresholds: vec![80, 100],
                }),
            ),
            ev(
                cycle(1),
                studio_bill(
                    "inv-2",
                    dec!(5480),
                    vec![(credits("inv-1"), dec!(5000))],
                    dec!(4.80),
                    dec!(4.77),
                ),
            ),
            ev(paid(1), charge("inv-2")),
            ev(
                cycle(0),
                studio_bill(
                    "inv-3",
                    dec!(4380),
                    vec![(credits("inv-2"), dec!(4380))],
                    Decimal::ZERO,
                    dec!(4.35),
                ),
            ),
            ev(paid(0), charge("inv-3")),
            ev(
                bought,
                Action::Purchase {
                    purchase: "P1",
                    denomination: CREDITS,
                    amount: dec!(2000),
                    expires_in_months: Some(PURCHASE_TERM_MONTHS),
                },
            ),
            ev(bought.plus_minutes(1), charge("P1")),
            ev(
                cycle_at(0, 7, 11, 0),
                Action::AddCredit {
                    denomination: USD,
                    amount: dec!(25),
                    reason: "Service credit",
                    promotional: true,
                    expires: Some(service_credit_expiry),
                },
            ),
        ],
        usage: usage_plan(
            "lumen-notes",
            1.0,
            vec![
                UsageTarget::new(
                    paid(2).plus_minutes(5),
                    cycle(1),
                    UsageAmount::Priced(dec!(5480)),
                ),
                UsageTarget::new(cycle(1), cycle(0), UsageAmount::Priced(dec!(4380))),
                UsageTarget::new(
                    cycle(0),
                    now(),
                    UsageAmount::TokensByKey(vec![
                        ("lumen-prod", [m(2.4), m(1.7), m(0.1)]),
                        ("lumen-staging", [m(0.6), m(0.3), m(0.05)]),
                    ]),
                ),
            ],
        ),
        expected: Expected {
            wallets: vec![
                ExpectedWallet {
                    denomination: CREDITS,
                    on_account: dec!(7620),
                    unbilled: dec!(3900),
                    balance: dec!(3720),
                    grants: vec![
                        grant(
                            GrantRef::Purchase("P1"),
                            dec!(2000),
                            months_after(bought.plus_minutes(1), PURCHASE_TERM_MONTHS),
                        ),
                        grant(
                            GrantRef::Rollover {
                                invoice: "inv-2",
                                component: INCLUDED_CREDITS,
                            },
                            dec!(620),
                            Some(Expiry::After {
                                from: cycle(0),
                                expiry: CreditExpiry::Month(1),
                            }),
                        ),
                        grant(credits("inv-3"), dec!(5000), Some(Expiry::At(cycle(-1)))),
                    ],
                },
                usd_wallet(
                    dec!(25),
                    Decimal::ZERO,
                    vec![grant(
                        GrantRef::Added("Service credit"),
                        dec!(25),
                        Some(Expiry::At(service_credit_expiry)),
                    )],
                ),
            ],
            ..Expected::default()
        },
    };
    account("Lumen Notes", "New York", "NY", history)
}

/// Free: 5M tokens a cycle on halyard-lite and halyard-standard, reached two days ago.
fn wren_labs() -> Customer {
    let reached = cycle_at(0, 5, 16, 40);
    let history = History {
        cycle_age_days: 7,
        sales_tax: None,
        events: vec![ev(cycle(2), subscribe(FREE))],
        usage: usage_plan(
            "wren-labs",
            1.1,
            vec![
                UsageTarget::new(cycle(2), cycle(1), UsageAmount::Tokens([m(1.6), m(0.5), 0])),
                UsageTarget::new(
                    cycle(1),
                    cycle(0),
                    UsageAmount::Tokens([m(3.35), m(1.05), 0]),
                ),
                UsageTarget::new(cycle(0), reached, UsageAmount::Tokens([m(3.8), m(1.2), 0])),
            ],
        ),
        expected: Expected {
            limit_reached_at: Some(reached),
            ..Expected::default()
        },
    };
    account("Wren Labs", "Oakland", "CA", history)
}

/// Enterprise: a $5,000 monthly minimum on a 12-month contract, invoiced on the billing day,
/// net 30, paid by bank transfer; the latest invoice is still open.
fn meridian_freight() -> Customer {
    let months: [(i32, Decimal, Decimal, (i64, i64)); 6] = [
        (5, dec!(4210), dec!(790), (24, 15)),
        (4, dec!(5380), Decimal::ZERO, (21, 11)),
        (3, dec!(5910), Decimal::ZERO, (26, 10)),
        (2, dec!(6240), Decimal::ZERO, (19, 14)),
        (1, dec!(5120), Decimal::ZERO, (27, 9)),
        (0, dec!(4640), dec!(360), (0, 0)),
    ];
    const INVOICES: [&str; 6] = ["inv-2", "inv-3", "inv-4", "inv-5", "inv-6", "inv-7"];
    let mut events = vec![
        ev(
            cycle(6),
            Action::Subscribe {
                plan: ENTERPRISE,
                ends: Some(cycle_at(-6, -1, 0, 0)),
                billing_day_anchor: None,
            },
        ),
        ev(cycle(6), bill("inv-1", InvoiceExpectation::empty())),
    ];
    let mut targets = Vec::new();
    for ((back, usage, shortfall, (day, hour)), invoice) in months.into_iter().zip(INVOICES) {
        targets.push(UsageTarget::new(
            cycle(back + 1),
            cycle(back),
            UsageAmount::Priced(usage),
        ));
        events.push(ev(
            cycle(back),
            bill(
                invoice,
                InvoiceExpectation {
                    usage,
                    minimum_shortfall: shortfall,
                    amount_due: usage + shortfall,
                    tax: ((usage + shortfall) * OHIO_TAX.rate)
                        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
                    ..InvoiceExpectation::empty()
                },
            ),
        ));
        if back > 0 {
            events.push(ev(
                cycle_at(back, day, hour, 0),
                Action::BankTransfer { payable: invoice },
            ));
        }
    }
    targets.push(UsageTarget::new(
        cycle(0),
        now(),
        UsageAmount::Priced(dec!(1240)),
    ));
    events.sort_by_key(|e| match e.at {
        When::Cycle { back, offset } => (-back, offset),
        _ => (i32::MAX, Duration::zero()),
    });
    let history = History {
        cycle_age_days: 7,
        sales_tax: Some(OHIO_TAX),
        events,
        usage: usage_plan("meridian-freight", 1.0, targets),
        expected: Expected {
            open_invoices: vec![("inv-7", cycle_at(0, 30, 0, 0))],
            ..Expected::default()
        },
    };
    account("Meridian Freight", "Columbus", "OH", history)
}

#[cfg(test)]
mod tests;

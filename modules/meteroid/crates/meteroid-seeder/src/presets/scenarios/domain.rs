use meteroid_store::domain::credit_exhausted::CreditExhaustedRule;
use std::collections::HashMap;

use chrono::NaiveDate;
use common_domain::ids::{CreditUnitId, RateCardId};
use educe::Educe;
use meteroid_store::StoreResult;
use meteroid_store::domain::credit_units::PriceCreditUnit;
use meteroid_store::domain::eligibility::Eligibility;
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::price_components::{CreditExpiry, CreditRollover};
use meteroid_store::domain::{
    Address, BillingMetricAggregateEnum, BillingPeriodEnum, BillingType, CapacityThreshold,
    CustomerTaxRate, DowngradePolicy, PlanTypeEnum, Pricing, RatingModel, SegmentationMatrix,
    ShippingAddress, TermRate, TierRow, UnitConversionRoundingEnum, UpgradePolicy,
    UsagePricingModel,
};
use meteroid_store::errors::StoreError;
use rust_decimal::Decimal;

use super::timeline::{Event, History};

#[derive(Clone)]
pub struct Scenario {
    pub name: String, // will be the tenant name
    /// What the modules add to the seed, told through `AnalyticsHook::sandbox_seeded`.
    pub template: meteroid_store::hooks::SandboxTemplate,
    pub metrics: Vec<BillableMetric>,
    /// Rate cards on the metrics, which plans price usage by, referenced by code.
    pub rate_cards: Vec<RateCardSpec>,
    /// Metered features plans reference by code in their entitlements.
    pub features: Vec<FeatureSpec>,
    pub plans: Vec<Plan>,
    pub customers: Vec<Customer>,
    pub organization: Option<OrganizationDetails>,
    /// Catalog of add-ons subscriptions may attach at creation, referenced by name.
    pub add_ons: Vec<AddOn>,
    /// Quotes to create after subscriptions, referencing customers/plans by name.
    pub quotes: Vec<QuoteSpec>,
    /// Prepaid-credit setup: each customer's history is replayed and live usage keeps flowing.
    pub credits: Option<CreditProgram>,
    /// Denominations usage can be priced in and plans can grant, referenced by code.
    pub credit_units: Vec<CreditUnitSpec>,
    /// Promotion codes customers redeem for promotional credit, referenced by code.
    pub promo_codes: Vec<PromoCodeSpec>,
    /// Discounts on plans, applied to the subscriptions that name them by code.
    pub coupons: Vec<CouponSpec>,
    /// The customer custom properties customers hold, defined before them.
    pub customer_properties: Vec<CustomerPropertySpec>,
    /// Currency the tenant reports in; `None` keeps the tenant's default.
    pub reporting_currency: Option<String>,
}

/// A plan's prices in another currency than its own, each component matched to the plan's by
/// name: the cells of that currency.
#[derive(Clone)]
pub struct CurrencyPrices {
    pub currency: &'static str,
    pub components: Vec<PriceComponent>,
}

/// A plan's copy on a pricing page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PricingPageCopy {
    pub tagline: &'static str,
    pub popular: bool,
    pub highlights: Vec<HighlightSpec>,
    pub cta_label: Option<&'static str>,
    /// Sold by contacting sales at `contact_url`: on the page, never self-serve.
    pub sales_led: bool,
    pub hide_price: bool,
    pub contact_url: Option<&'static str>,
}

/// A line of a plan's card: text, or a feature by code, shown with its value on the plan.
#[derive(Clone, Debug, PartialEq)]
pub enum HighlightSpec {
    Text(&'static str),
    Feature(&'static str),
}

/// A money rate card on one of the scenario's metrics, each key's rates in force from
/// `effective_from`.
#[derive(Clone, Debug, PartialEq)]
pub struct RateCardSpec {
    pub code: &'static str,
    pub name: &'static str,
    pub metric_code: &'static str,
    pub dimensions: Vec<&'static str>,
    /// Rates are per this many units of the metric.
    pub unit_size: Decimal,
    pub effective_from: NaiveDate,
    pub rates: Vec<RateSpec>,
}

/// A key of a rate card, a pattern per dimension (`*`: any value), and its rate by currency.
#[derive(Clone, Debug, PartialEq)]
pub struct RateSpec {
    pub key: Vec<&'static str>,
    pub label: Option<&'static str>,
    pub rates: Vec<(&'static str, Decimal)>,
}

/// A credit-based scenario: one metered metric whose usage draws the customers' wallets down.
#[derive(Clone, Debug, PartialEq)]
pub struct CreditProgram {
    pub usage_metric_code: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreditUnitSpec {
    pub code: &'static str,
    pub name: &'static str,
    pub plural_name: &'static str,
    pub symbol: Option<&'static str>,
    pub precision: u8,
    /// Price of one credit by currency, for selling the unit; none leaves it unsold.
    pub selling_prices: Vec<(&'static str, Decimal)>,
}

/// A code worth `amount` of promotional credit, expiring `expires_after_days` after redemption.
#[derive(Clone, Debug, PartialEq)]
pub struct PromoCodeSpec {
    pub code: &'static str,
    pub name: &'static str,
    pub amount: Decimal,
    pub currency: &'static str,
    pub expires_after_days: u32,
}

/// A customer custom property: one of `options`, or free text when there are none.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomerPropertySpec {
    pub key: &'static str,
    pub name: &'static str,
    pub options: Vec<&'static str>,
}

/// A percentage off the plans named, redeemed with `code` by who `eligibility` lets in.
#[derive(Clone, Debug, PartialEq)]
pub struct CouponSpec {
    pub code: &'static str,
    pub name: &'static str,
    pub percent_off: Decimal,
    /// Plan names; empty: every plan.
    pub plans: Vec<&'static str>,
    /// Months it discounts each subscription from redemption; `None`: forever.
    pub months: Option<u32>,
    pub eligibility: Option<Eligibility>,
    pub valid_from: Option<NaiveDate>,
    pub expires_on: Option<NaiveDate>,
    pub redemption_limit: Option<u32>,
}

/// How a saved card behaves on the mock payment gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardBehavior {
    Approved,
    /// A soft decline: retrying later may succeed.
    Declined,
    /// The card is past its expiry date: every charge is declined as expired.
    Expired,
    /// A hard decline (lost card): an automatic top-up on it is suspended at once.
    HardDeclined,
}

impl CardBehavior {
    /// The scenario code the mock gateway reads from the payment method id.
    pub fn gateway_code(self) -> &'static str {
        match self {
            CardBehavior::Approved => "ok",
            CardBehavior::Declined => "decline",
            CardBehavior::Expired => "expired",
            CardBehavior::HardDeclined => "hard",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardSpec {
    pub brand: &'static str,
    pub last4: &'static str,
    pub behavior: CardBehavior,
    /// Card expiry, in months from the seed's `as_of` month (negative for an expired card).
    pub expires_in_months: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AutoTopUpAmount {
    /// Adds the same amount every time.
    Fixed(Decimal),
    /// Brings the balance back up to this.
    ToTarget(Decimal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoTopUpSpec {
    /// Tops up once the balance falls to this or below.
    pub threshold: Decimal,
    pub amount: AutoTopUpAmount,
    pub daily_cap: Option<Decimal>,
    pub monthly_cap: Option<Decimal>,
}

/// A per-billing-cycle limit set for one customer: an override of a metered feature's
/// entitlement. `block` rejects usage past it instead of letting it run.
#[derive(Clone, Debug, PartialEq)]
pub struct EntitlementOverrideSpec {
    pub feature_code: String,
    pub limit: Decimal,
    pub block: Option<BlockSpec>,
    pub thresholds: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BudgetSpec {
    pub amount: Decimal,
    pub hard: bool,
}

/// A quote for a customer on a plan. When `convert` is set the quote is published,
/// accepted and converted into a subscription; otherwise it is left open (Pending).
#[derive(Clone)]
pub struct QuoteSpec {
    pub customer: String,
    pub plan_name: String,
    pub convert: bool,
}

#[derive(Clone)]
pub struct Plan {
    pub name: String,
    pub currency: String,
    pub plan_type: PlanTypeEnum,
    pub components: Vec<PriceComponent>,
    pub entitlements: Vec<PlanEntitlementSpec>,
    /// A newer version, published once the subscriptions exist: they all stay on the first one.
    pub revision: Option<Vec<PriceComponent>>,
    pub payment: PlanPayment,
    /// Period minimum commitment over every component, in the plan currency.
    pub minimum: Option<Decimal>,
    /// Offered for self-service plan changes in the portal, in this order.
    pub self_service_rank: Option<i32>,
    /// What usage does once the credit runs out; unset takes the default of the components.
    pub credit_exhausted_rule: Option<CreditExhaustedRule>,
    /// Prices in other currencies; the components' own are in the plan's currency.
    pub currency_prices: Vec<CurrencyPrices>,
    pub pricing_page: Option<PricingPageCopy>,
    /// One option per cadence, named after it ("Annual", alias `annual`): a charge priced on
    /// several bills on the option's. Empty: the one default option.
    pub cadence_options: Vec<BillingPeriodEnum>,
    /// Who may get it; `None`: anyone.
    pub eligibility: Option<Eligibility>,
}

/// How a plan's invoices are collected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanPayment {
    /// Days from issue to due date; 0 is due on receipt.
    pub net_terms: u32,
    pub collection: Collection,
}

impl Default for PlanPayment {
    fn default() -> Self {
        PlanPayment {
            net_terms: 30,
            collection: Collection::SendInvoice {
                bank_transfer: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    /// The customer's default card is charged when the invoice is issued.
    ChargeAutomatically,
    /// The invoice is sent and paid by the customer, by bank transfer when set.
    SendInvoice { bank_transfer: bool },
}

impl Plan {
    pub fn new(
        name: &str,
        currency: &str,
        plan_type: PlanTypeEnum,
        components: Vec<PriceComponent>,
    ) -> Self {
        Plan {
            name: name.to_string(),
            currency: currency.to_string(),
            plan_type,
            components,
            entitlements: vec![],
            revision: None,
            payment: PlanPayment::default(),
            minimum: None,
            self_service_rank: None,
            credit_exhausted_rule: None,
            currency_prices: vec![],
            pricing_page: None,
            cadence_options: vec![],
            eligibility: None,
        }
    }

    pub fn with_currency_prices(
        mut self,
        currency: &'static str,
        components: Vec<PriceComponent>,
    ) -> Self {
        self.currency_prices.push(CurrencyPrices {
            currency,
            components,
        });
        self
    }

    pub fn with_pricing_page(
        mut self,
        tagline: &'static str,
        popular: bool,
        highlights: Vec<HighlightSpec>,
    ) -> Self {
        self.pricing_page = Some(PricingPageCopy {
            tagline,
            popular,
            highlights,
            ..Default::default()
        });
        self
    }

    /// Sold by contacting sales at `contact_url`, its button reading `cta`.
    pub fn sold_by_sales(
        mut self,
        cta: &'static str,
        contact_url: &'static str,
        hide_price: bool,
    ) -> Self {
        let page = self.pricing_page.get_or_insert_with(Default::default);
        page.cta_label = Some(cta);
        page.sales_led = true;
        page.hide_price = hide_price;
        page.contact_url = Some(contact_url);
        self
    }

    pub fn with_eligibility(mut self, eligibility: Eligibility) -> Self {
        self.eligibility = Some(eligibility);
        self
    }

    /// Each charge rated by a scenario rate card, priced by the card created for its code.
    pub fn resolve_rate_cards(&mut self, ids: &HashMap<String, RateCardId>) -> StoreResult<()> {
        let components = self
            .components
            .iter_mut()
            .chain(
                self.currency_prices
                    .iter_mut()
                    .flat_map(|prices| prices.components.iter_mut()),
            )
            .chain(self.revision.iter_mut().flatten());
        for component in components {
            let FeeType::RateCard {
                metric_code,
                rate_card,
                multiplier,
                cadence,
            } = &component.fee
            else {
                continue;
            };
            let rate_card_id = ids.get(rate_card).copied().ok_or_else(|| {
                StoreError::ValueNotFound(format!("Rate card was not found {rate_card}"))
            })?;
            let priced = FeeType::Usage {
                metric_code: metric_code.clone(),
                pricing: UsagePricingModel::RateCard {
                    rate_card_id,
                    multiplier: *multiplier,
                },
                cadence: *cadence,
                credit_unit: None,
            };
            component.fee = priced;
        }
        Ok(())
    }

    pub fn with_credit_exhausted_rule(mut self, rule: CreditExhaustedRule) -> Self {
        self.credit_exhausted_rule = Some(rule);
        self
    }

    pub fn with_revision(mut self, components: Vec<PriceComponent>) -> Self {
        self.revision = Some(components);
        self
    }

    /// A per-billing-cycle limit that lets usage run past it.
    pub fn with_cycle_limit(
        mut self,
        feature_code: &str,
        limit: Decimal,
        warning_pct: u32,
    ) -> Self {
        self.entitlements.push(PlanEntitlementSpec::Metered {
            feature_code: feature_code.to_string(),
            limit,
            warning_pct,
            block: None,
        });
        self
    }

    /// A per-billing-cycle limit that rejects usage once reached, past `grace_pct` when set.
    pub fn with_hard_cycle_limit(
        mut self,
        feature_code: &str,
        limit: Decimal,
        warning_pct: u32,
        grace_pct: Option<u32>,
    ) -> Self {
        self.entitlements.push(PlanEntitlementSpec::Metered {
            feature_code: feature_code.to_string(),
            limit,
            warning_pct,
            block: Some(BlockSpec { grace_pct }),
        });
        self
    }

    pub fn with_feature(mut self, feature_code: &str, enabled: bool) -> Self {
        self.entitlements.push(PlanEntitlementSpec::Boolean {
            feature_code: feature_code.to_string(),
            enabled,
        });
        self
    }

    pub fn with_payment(mut self, net_terms: u32, collection: Collection) -> Self {
        self.payment = PlanPayment {
            net_terms,
            collection,
        };
        self
    }

    pub fn with_minimum(mut self, amount: Decimal) -> Self {
        self.minimum = Some(amount);
        self
    }

    pub fn with_cadence_options(mut self, cadences: &[BillingPeriodEnum]) -> Self {
        self.cadence_options = cadences.to_vec();
        self
    }

    pub fn with_self_service_rank(mut self, rank: i32) -> Self {
        self.self_service_rank = Some(rank);
        self
    }

    pub fn entitlement(&self, feature_code: &str) -> Option<&PlanEntitlementSpec> {
        self.entitlements
            .iter()
            .find(|e| e.feature_code() == feature_code)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureSpec {
    pub code: String,
    pub name: String,
    pub kind: FeatureKind,
    /// The product it belongs to, by name: only plans selling it get it. `None`: every plan.
    pub product: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureKind {
    /// Usage of one of the scenario's metrics; `unlimited_by_default` grants it without limit to
    /// every plan that does not set it.
    Metered {
        metric_code: String,
        unlimited_by_default: bool,
    },
    /// An on/off capability; `default_enabled` holds for every plan that does not set it.
    Boolean { default_enabled: bool },
    /// A number every plan in its scope gets, such as the seats included.
    Number { default: Decimal },
}

impl FeatureSpec {
    pub fn metric_code(&self) -> Option<&str> {
        match &self.kind {
            FeatureKind::Metered { metric_code, .. } => Some(metric_code),
            FeatureKind::Boolean { .. } | FeatureKind::Number { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanEntitlementSpec {
    /// A per-billing-cycle limit; `block` rejects usage past it instead of letting it run.
    Metered {
        feature_code: String,
        limit: Decimal,
        warning_pct: u32,
        block: Option<BlockSpec>,
    },
    Boolean {
        feature_code: String,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockSpec {
    pub grace_pct: Option<u32>,
}

impl PlanEntitlementSpec {
    pub fn feature_code(&self) -> &str {
        match self {
            PlanEntitlementSpec::Metered { feature_code, .. }
            | PlanEntitlementSpec::Boolean { feature_code, .. } => feature_code,
        }
    }

    pub fn limit(&self) -> Option<Decimal> {
        match self {
            PlanEntitlementSpec::Metered { limit, .. } => Some(*limit),
            PlanEntitlementSpec::Boolean { .. } => None,
        }
    }
}

#[derive(Clone, Educe)]
#[educe(Default)]
pub struct BillableMetric {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    #[educe(Default = BillingMetricAggregateEnum::Sum)]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<SegmentationMatrix>,
    pub usage_group_key: Option<String>,
    /// How one unit and many units of the metric read, e.g. "token" / "tokens".
    pub unit_label: Option<UnitLabel>,
    /// What the sandbox's generated events carry; `None`: the default sample.
    pub usage: Option<UsageSample>,
}

/// Generated events valued in whole units within `[min, max]`, each carrying every property.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageSample {
    pub min: u64,
    pub max: u64,
    pub properties: Vec<EventProperty>,
}

/// An event property and its values, each drawn as often as its weight says.
#[derive(Clone, Debug, PartialEq)]
pub struct EventProperty {
    pub key: &'static str,
    pub values: Vec<(&'static str, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLabel {
    pub singular: &'static str,
    pub plural: &'static str,
}

#[derive(Clone)]
pub struct Subscription {
    pub plan_name: String,
    pub start_date: NaiveDate,
    /// Cancellation effective date: a churn when past, a scheduled cancellation when future.
    pub end_date: Option<NaiveDate>,
    /// Trial length in days; the subscription starts in trial before billing.
    pub trial_days: Option<u32>,
    /// Initial slot/seat count for the plan's slot component, when it has one.
    pub seats: Option<u32>,
    /// Add-ons attached at creation, referencing the scenario's add-on catalog by name.
    pub add_ons: Vec<AddOnRef>,
    pub minimum: Option<MinimumSpec>,
    /// Prices replacing the plan's for this subscription, matched to its components by name.
    pub overrides: Vec<PriceComponent>,
    /// The alias of the plan option it buys; none: the plan's default. It bills in its
    /// customer's currency.
    pub option: Option<&'static str>,
    /// Codes of the scenario coupons applied to it.
    pub coupons: Vec<&'static str>,
}

/// A subscription-level minimum commitment, scoped to one product when named.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimumSpec {
    pub product_name: Option<String>,
    pub amount: Decimal,
}

impl Subscription {
    /// An open-ended subscription on `plan` starting at `start`.
    pub fn active(plan_name: impl Into<String>, start_date: NaiveDate) -> Self {
        Subscription {
            plan_name: plan_name.into(),
            start_date,
            end_date: None,
            trial_days: None,
            seats: None,
            add_ons: vec![],
            minimum: None,
            overrides: vec![],
            option: None,
            coupons: vec![],
        }
    }

    pub fn on_option(mut self, alias: &'static str) -> Self {
        self.option = Some(alias);
        self
    }

    pub fn with_coupon(mut self, code: &'static str) -> Self {
        self.coupons.push(code);
        self
    }

    /// A subscription that churned on `end_date`.
    pub fn churned(
        plan_name: impl Into<String>,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Self {
        Subscription {
            end_date: Some(end_date),
            ..Subscription::active(plan_name, start_date)
        }
    }

    /// A running subscription with a cancellation scheduled for `ends_on`.
    pub fn ending(plan_name: impl Into<String>, start_date: NaiveDate, ends_on: NaiveDate) -> Self {
        Subscription::churned(plan_name, start_date, ends_on)
    }

    pub fn with_minimum(mut self, product_name: Option<&str>, amount: Decimal) -> Self {
        self.minimum = Some(MinimumSpec {
            product_name: product_name.map(str::to_string),
            amount,
        });
        self
    }

    pub fn with_trial(mut self, days: u32) -> Self {
        self.trial_days = Some(days);
        self
    }

    pub fn with_seats(mut self, seats: u32) -> Self {
        self.seats = Some(seats);
        self
    }

    pub fn with_override(mut self, component: PriceComponent) -> Self {
        self.overrides.push(component);
        self
    }

    pub fn with_add_on(mut self, name: impl Into<String>, quantity: u32) -> Self {
        self.add_ons.push(AddOnRef {
            name: name.into(),
            quantity,
        });
        self
    }
}

/// A reference from a subscription to a scenario add-on, with a quantity.
#[derive(Clone)]
pub struct AddOnRef {
    pub name: String,
    pub quantity: u32,
}

/// A catalog add-on offered in a scenario: a flat recurring fee attachable to
/// subscriptions. Kept intentionally simple (name + monthly price).
#[derive(Clone)]
pub struct AddOn {
    pub name: String,
    pub price: Decimal,
    pub cadence: BillingPeriodEnum,
}

#[derive(Clone)]
pub struct Customer {
    pub name: String,
    pub email: String,
    pub currency: String,
    pub alias: Option<String>,
    pub phone: Option<String>,
    pub vat_number: Option<String>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_emails: Vec<String>,
    /// One or more subscriptions over the customer's lifetime. A churn followed by
    /// a new subscription on a different plan models an upgrade/downgrade.
    pub subscriptions: Vec<Subscription>,
    pub usage: UsageProfile,
    /// Replayed history of a credit customer; its usage is generated from the history's targets.
    pub history: Option<History>,
    /// Credit of a customer without a history, set up before its subscriptions are billed: a
    /// grant pays the invoices dated after it.
    pub credit: Vec<Event>,
    /// Sales tax charged on the customer's invoices, through the Manual tax resolver.
    pub tax_rates: Vec<CustomerTaxRate>,
    /// Custom properties as the customer holds them, keyed by property; null: none.
    pub custom_properties: serde_json::Value,
}

/// How a customer's synthetic usage is generated. Every profile other than `Generated`
/// stages a detector outcome for the customer's current billing cycle.
#[derive(Clone, Debug, PartialEq)]
pub enum UsageProfile {
    Generated,
    /// Pays but sends no usage events (shelfware / dormant whale).
    None,
    /// The current cycle carries exactly `pct_of_limit` % of the plan limit on `metric_code`.
    CycleUsage {
        metric_code: String,
        pct_of_limit: u32,
    },
    /// Keeps `keep_ratio` of the generated events inside the current cycle.
    Collapse {
        keep_ratio: f64,
    },
}

impl Customer {
    pub fn without_usage(mut self) -> Self {
        self.usage = UsageProfile::None;
        self
    }

    pub fn with_cycle_usage(mut self, metric_code: &str, pct_of_limit: u32) -> Self {
        self.usage = UsageProfile::CycleUsage {
            metric_code: metric_code.to_string(),
            pct_of_limit,
        };
        self
    }

    pub fn with_usage_collapse(mut self, keep_ratio: f64) -> Self {
        self.usage = UsageProfile::Collapse { keep_ratio };
        self
    }

    pub fn with_credit(mut self, events: Vec<Event>) -> Self {
        self.credit = events;
        self
    }

    pub fn has_staged_usage(&self) -> bool {
        self.usage != UsageProfile::Generated
    }

    /// The subscription running on `today`, the latest-started one when several overlap.
    pub fn current_subscription(&self, today: NaiveDate) -> Option<&Subscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.start_date <= today && s.end_date.is_none_or(|end| end > today))
            .max_by_key(|s| s.start_date)
    }
}

#[derive(Clone)]
pub struct OrganizationDetails {
    pub vat_number: Option<String>,
    pub address_line1: Option<String>,
    pub city: Option<String>,
    pub zip_code: Option<String>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    /// Pinned so the invoice language doesn't follow the seeding org's country (the pool org).
    pub invoicing_language: Option<String>,
    /// ISO country and region the entity is established in, which tax follows.
    pub country: Option<&'static str>,
    pub state: Option<&'static str>,
    pub tax_resolver: Option<TaxResolverEnum>,
}

/// The default preset doesn't exercise every fee primitive; the DSL keeps the
/// full set so derived variations can use them.
#[derive(Clone)]
#[allow(dead_code)]
pub enum FeeType {
    Rate {
        rates: Vec<TermRate>,
    },
    Slot {
        rates: Vec<TermRate>,
        slot_unit_name: String,
        upgrade_policy: UpgradePolicy,
        downgrade_policy: DowngradePolicy,
        minimum_count: Option<u32>,
        quota: Option<u32>,
        /// Seats free of charge before each one is billed at its rate.
        included: Option<u32>,
    },
    Capacity {
        metric_code: String,
        thresholds: Vec<CapacityThreshold>,
        cadence: BillingPeriodEnum,
    },
    Usage {
        metric_code: String,
        /// Amounts are in `credit_unit` when set, otherwise in the plan currency.
        pricing: UsagePricingModel,
        cadence: BillingPeriodEnum,
        credit_unit: Option<UnitPricing>,
    },
    /// Usage rated by the scenario rate card of code `rate_card`, each rate scaled by
    /// `multiplier`; the runner prices it by the card once created.
    RateCard {
        metric_code: String,
        rate_card: String,
        multiplier: Option<Decimal>,
        cadence: BillingPeriodEnum,
    },
    ExtraRecurring {
        unit_price: Decimal,
        quantity: u32,
        billing_type: BillingType,
        cadence: BillingPeriodEnum,
    },
    OneTime {
        unit_price: Decimal,
        quantity: u32,
    },
    /// A credit pack billed in advance each period; the credit is granted once it is paid.
    Credit {
        unit_price: Decimal,
        quantity: u32,
        credit_amount: Decimal,
        expiry: Option<CreditExpiry>,
        /// Code of the scenario credit unit `credit_amount` is granted in; the plan currency
        /// when `None`.
        credit_unit: Option<String>,
        rollover: Option<CreditRollover>,
        /// Granted with the first billing period only.
        once: bool,
    },
}

/// The first `included` units free, each one past them at `rate`.
pub fn included_then(included: u32, rate: Decimal) -> RatingModel {
    RatingModel::Tiered {
        tiers: vec![
            TierRow {
                first_unit: 0,
                rate: Decimal::ZERO,
                flat_fee: None,
                flat_cap: None,
            },
            TierRow {
                first_unit: u64::from(included),
                rate,
                flat_fee: None,
                flat_cap: None,
            },
        ],
    }
}

/// Usage priced in a credit unit, each credit worth `rate` of the plan currency as overage.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitPricing {
    pub credit_unit: String,
    pub rate: Decimal,
}

impl FeeType {
    pub fn metric_code(&self) -> Option<String> {
        match self {
            FeeType::Capacity { metric_code, .. } => Some(metric_code.clone()),
            FeeType::Usage { metric_code, .. } | FeeType::RateCard { metric_code, .. } => {
                Some(metric_code.clone())
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct PriceComponent {
    pub name: String,
    pub fee: FeeType,
}

impl PriceComponent {
    pub fn to_domain(
        &self,
        metrics: &[meteroid_store::domain::BillableMetric],
        currency: &str,
        credit_units: &HashMap<String, CreditUnitId>,
    ) -> StoreResult<meteroid_store::domain::PriceComponentNewInternal> {
        let unit_id = |code: &str| {
            credit_units.get(code).copied().ok_or_else(|| {
                StoreError::ValueNotFound(format!("Credit unit was not found {code}"))
            })
        };
        let maybe_metric_code = self.fee.metric_code();

        let maybe_metric = maybe_metric_code
            .and_then(|metric_code| metrics.iter().find(|metric| metric.code == metric_code));

        let domain_fee: meteroid_store::domain::FeeType = match &self.fee {
            FeeType::Rate { rates } => meteroid_store::domain::FeeType::Rate {
                rates: rates.clone(),
            },
            FeeType::Slot {
                rates,
                slot_unit_name,
                upgrade_policy,
                downgrade_policy,
                minimum_count,
                quota,
                ..
            } => meteroid_store::domain::FeeType::Slot {
                rates: rates.clone(),
                slot_unit_name: slot_unit_name.clone(),
                upgrade_policy: upgrade_policy.clone(),
                downgrade_policy: downgrade_policy.clone(),
                minimum_count: *minimum_count,
                quota: *quota,
            },
            FeeType::Capacity {
                metric_code,
                thresholds,
                cadence,
            } => {
                let metric = maybe_metric.ok_or(StoreError::ValueNotFound(format!(
                    "Metric was not found {metric_code}"
                )))?;
                meteroid_store::domain::FeeType::Capacity {
                    metric_id: metric.id,
                    thresholds: thresholds.clone(),
                    cadence: *cadence,
                }
            }
            FeeType::Usage {
                metric_code,
                pricing,
                cadence,
                ..
            } => {
                let metric = maybe_metric.ok_or(StoreError::ValueNotFound(format!(
                    "Metric was not found {metric_code}"
                )))?;
                meteroid_store::domain::FeeType::Usage {
                    metric_id: metric.id,
                    pricing: pricing.clone(),
                    cadence: *cadence,
                }
            }
            FeeType::RateCard { rate_card, .. } => {
                return Err(StoreError::ValueNotFound(format!(
                    "Rate card {rate_card} is not created yet"
                ))
                .into());
            }
            FeeType::ExtraRecurring {
                unit_price,
                quantity,
                billing_type,
                cadence,
            } => meteroid_store::domain::FeeType::ExtraRecurring {
                unit_price: *unit_price,
                quantity: *quantity,
                billing_type: billing_type.clone(),
                cadence: *cadence,
            },
            FeeType::OneTime {
                unit_price,
                quantity,
            } => meteroid_store::domain::FeeType::OneTime {
                unit_price: *unit_price,
                quantity: *quantity,
            },
            FeeType::Credit {
                unit_price,
                quantity,
                credit_amount,
                expiry,
                once,
                ..
            } => meteroid_store::domain::FeeType::Credit {
                unit_price: *unit_price,
                quantity: *quantity,
                credit_amount: *credit_amount,
                expiry: *expiry,
                once: *once,
            },
        };

        use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
        use meteroid_store::domain::prices::{extract_fee_structure, extract_pricing};

        let (fee_type_enum, fee_structure) = extract_fee_structure(&domain_fee);
        let mut pricing_list = extract_pricing(&domain_fee);

        // The legacy fee carries neither the granted unit nor the rollover: set them on the price.
        if let FeeType::Credit {
            credit_unit,
            rollover,
            ..
        } = &self.fee
        {
            let granted_in = credit_unit.as_deref().map(unit_id).transpose()?;
            for (_, pricing) in &mut pricing_list {
                if let Pricing::Credit {
                    credit_unit_id,
                    rollover: price_rollover,
                    ..
                } = pricing
                {
                    *credit_unit_id = granted_in;
                    *price_rollover = *rollover;
                }
            }
        }
        // The legacy fee prices every seat: the free ones are a first tier at zero.
        if let FeeType::Slot {
            included: Some(included),
            ..
        } = &self.fee
        {
            for (_, pricing) in &mut pricing_list {
                if let Pricing::Slot { model, .. } = pricing
                    && let Some(rate) = model.per_unit_rate()
                {
                    *model = included_then(*included, rate);
                }
            }
        }
        let priced_in = match &self.fee {
            FeeType::Usage {
                credit_unit: Some(unit),
                ..
            } => Some(PriceCreditUnit {
                unit_id: unit_id(&unit.credit_unit)?,
                rate: unit.rate,
            }),
            _ => None,
        };

        let prices = pricing_list
            .into_iter()
            .map(|(cadence, pricing)| {
                PriceEntry::New(PriceInput {
                    cadence,
                    currency: currency.to_string(),
                    pricing,
                    credit_unit: priced_in.clone(),
                })
            })
            .collect();

        // The charge owns its structure: it stays when the product is reused by name.
        Ok(meteroid_store::domain::PriceComponentNewInternal {
            name: self.name.clone(),
            product_ref: ProductRef::New {
                name: self.name.clone(),
                fee_type: fee_type_enum,
                fee_structure: fee_structure.clone(),
            },
            fee_structure: Some(fee_structure),
            prices,
        })
    }
}

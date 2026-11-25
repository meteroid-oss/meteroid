use billing_core::pricing;
use rust_decimal::Decimal;

use crate::domain::credit_units::FeeCreditUnit;
use crate::domain::enums::UnitConversionRoundingEnum;
use crate::domain::invoice_lines::{UsageBreakdown, UsageBreakdownItem};
use crate::domain::{BillableMetric, LineItem, UsagePricingModel};

use super::fees::pricing_error;
use crate::StoreResult;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use common_domain::ids::{BillableMetricId, SubscriptionAddOnId, SubscriptionPriceComponentId};
use common_utils::decimals::{MAX_QUANTITY_SCALE, round_quantity};
use error_stack::ResultExt;
use std::collections::{BTreeMap, HashMap};

/// Key to match existing line items during invoice refresh
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingLineKey {
    pub metric_id: BillableMetricId,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub group_by_dimensions: Option<HashMap<String, String>>,
}

impl std::hash::Hash for ExistingLineKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.metric_id.hash(state);
        self.sub_component_id.hash(state);
        self.sub_add_on_id.hash(state);
        // For HashMap, we need to hash in a deterministic order
        if let Some(ref dims) = self.group_by_dimensions {
            let mut sorted_dims: Vec<_> = dims.iter().collect();
            sorted_dims.sort_by_key(|(k, _)| *k);
            for (k, v) in sorted_dims {
                k.hash(state);
                v.hash(state);
            }
        }
    }
}

impl ExistingLineKey {
    pub fn from_line_item(line: &LineItem) -> Option<Self> {
        Some(Self {
            metric_id: line.metric_id?,
            sub_component_id: line.sub_component_id,
            sub_add_on_id: line.sub_add_on_id,
            group_by_dimensions: line.group_by_dimensions.clone(),
        })
    }
}

/// The unit price an operator pinned on a draft line, in the denomination the fee now prices in:
/// a pin never crosses from money to credits or between units.
pub(crate) fn pinned_unit_price(
    line: &LineItem,
    credit_unit: Option<&FeeCreditUnit>,
) -> Option<Decimal> {
    match (&line.credit_unit, credit_unit) {
        (None, None) => line.unit_price,
        (Some(block), Some(unit)) if block.unit_id == unit.unit_id => block.unit_price,
        _ => None,
    }
}

/// Whether a line bills metered usage of a subscription component or add-on.
pub(crate) fn is_usage_based_line(line: &LineItem) -> bool {
    line.metric_id.is_some() && (line.sub_component_id.is_some() || line.sub_add_on_id.is_some())
}

/// The unit prices an operator pinned on a draft's usage lines, by the line they sit on.
#[derive(Debug, Clone, Default)]
pub struct PinnedPrices(HashMap<ExistingLineKey, Decimal>);

impl PinnedPrices {
    /// `tax_inclusive` drafts store net lines, priced in the tax-inclusive figures they were quoted in.
    /// A line priced in a credit unit pins its price in credits, which bear no tax.
    pub fn add_lines(&mut self, lines: &[LineItem], tax_inclusive: bool) {
        for line in lines.iter().filter(|line| is_usage_based_line(line)) {
            if let Some(credit) = &line.credit_unit {
                if let (Some(key), Some(price)) =
                    (ExistingLineKey::from_line_item(line), credit.unit_price)
                {
                    self.0.insert(key, price);
                }
                continue;
            }
            let line = if tax_inclusive {
                line.clone().into_tax_inclusive_quote()
            } else {
                line.clone()
            };
            if let (Some(key), Some(price)) =
                (ExistingLineKey::from_line_item(&line), line.unit_price)
            {
                self.0.insert(key, price);
            }
        }
    }

    /// Adds `other`'s prices, which win where both pin a line.
    pub(crate) fn extend(&mut self, other: &PinnedPrices) {
        self.0
            .extend(other.0.iter().map(|(key, price)| (key.clone(), *price)));
    }

    pub(crate) fn unit_price(
        &self,
        metric_id: BillableMetricId,
        sub_component_id: Option<SubscriptionPriceComponentId>,
        sub_add_on_id: Option<SubscriptionAddOnId>,
        dimensions: &HashMap<String, String>,
    ) -> Option<Decimal> {
        self.0
            .get(&ExistingLineKey {
                metric_id,
                sub_component_id,
                sub_add_on_id,
                group_by_dimensions: Some(dimensions.clone()),
            })
            .copied()
    }
}

/// Values of a breakdown key an invoice line lists one by one; the rest are summed as others.
const MAX_BREAKDOWN_ITEMS: usize = 100;

/// The usage of `usage`, read grouped by `breakdown_key` too, that the one line of group
/// `dimensions` priced: a line rated by dimension weighs each item by the rates its keys
/// resolve to, any other line by its quantity; none when no usage of its group was read.
pub(crate) fn line_breakdown(
    dimensions: Option<&HashMap<String, String>>,
    metric: &BillableMetric,
    breakdown_key: &str,
    rated: Option<&pricing::CardRates>,
    usage: &[GroupedUsageData],
) -> Option<UsageBreakdown> {
    // A line rated by dimension stands for the usage of all values of the dimensions it keys.
    let keyed: &[String] = rated.map_or(&[], |card| card.dimensions.as_slice());
    let line_dimensions = sorted_dimensions(
        dimensions
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let mut totals: HashMap<Option<&str>, (Decimal, Decimal)> = HashMap::new();
    for row in usage {
        let quantity = unrounded_units(metric.unit_conversion_factor, row.value);
        if quantity <= Decimal::ZERO {
            continue;
        }
        let weight = match rated {
            Some(card) => match rated_weight(card, &row.dimensions, quantity) {
                Some(weight) => weight,
                None => continue,
            },
            None => quantity,
        };
        let row_dimensions = sorted_dimensions(
            row.dimensions
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .filter(|(key, _)| *key != breakdown_key && !keyed.iter().any(|k| k == *key)),
        );
        if row_dimensions != line_dimensions {
            continue;
        }
        // A blank value counts as no value, so both land in the same item.
        let value = row
            .dimensions
            .get(breakdown_key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty());
        let total = totals.entry(value).or_default();
        total.0 += quantity;
        total.1 += weight;
    }
    if totals.is_empty() {
        return None;
    }
    let mut items = totals
        .into_iter()
        .map(|(value, (quantity, weight))| UsageBreakdownItem {
            value: value.map(str::to_owned),
            quantity,
            weight,
            others: false,
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| b.quantity.cmp(&a.quantity))
            .then_with(|| a.value.cmp(&b.value))
    });
    if items.len() > MAX_BREAKDOWN_ITEMS {
        let rest = items.split_off(MAX_BREAKDOWN_ITEMS - 1);
        items.push(UsageBreakdownItem {
            value: None,
            quantity: rest.iter().map(|item| item.quantity).sum(),
            weight: rest.iter().map(|item| item.weight).sum(),
            others: true,
        });
    }
    for item in &mut items {
        item.quantity = round_quantity(item.quantity, MAX_QUANTITY_SCALE);
    }
    Some(UsageBreakdown {
        key: breakdown_key.to_owned(),
        items,
    })
}

fn sorted_dimensions<'a>(
    dimensions: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str)> {
    let mut dimensions = dimensions.collect::<Vec<_>>();
    dimensions.sort_unstable();
    dimensions
}

/// A metric's unit conversion, applied to each group's period total after aggregation and before
/// pricing. Every path that prices usage (invoices, previews, the live credit projection, usage
/// costs) goes through it so all of them round alike.
pub(crate) fn convert_usage_units(metric: &BillableMetric, usage: UsageData) -> UsageData {
    UsageData {
        period: usage.period,
        data: usage
            .data
            .into_iter()
            .map(|usage| GroupedUsageData {
                value: converted_units(
                    metric.unit_conversion_factor,
                    metric.unit_conversion_rounding.as_ref(),
                    usage.value,
                ),
                dimensions: usage.dimensions,
            })
            .collect(),
    }
}

pub(crate) fn converted_units(
    factor: Option<i32>,
    rounding: Option<&UnitConversionRoundingEnum>,
    units: Decimal,
) -> Decimal {
    pricing::converted_units(factor, rounding.map(Into::into), units)
}

pub(crate) fn unrounded_units(factor: Option<i32>, units: Decimal) -> Decimal {
    pricing::unrounded_units(factor, units)
}

/// Whether the metric's conversion rounds, which makes its price a step function of the usage.
pub(crate) fn rounds_converted_units(metric: &BillableMetric) -> bool {
    metric
        .unit_conversion_factor
        .is_some_and(|factor| factor != 0)
        && !matches!(
            metric.unit_conversion_rounding,
            None | Some(UnitConversionRoundingEnum::None)
        )
}

/// What `quantity` units of usage carrying `dimensions` cost at the rate its key resolves to on
/// `card.on`, unrounded; none when no key rates it.
pub(crate) fn rated_weight(
    card: &pricing::CardRates,
    dimensions: &HashMap<String, String>,
    quantity: Decimal,
) -> Option<Decimal> {
    let dimensions: BTreeMap<String, String> = dimensions
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let rate =
        billing_core::rate_card::resolve(&card.dimensions, &card.entries, &dimensions, card.on)?;
    (quantity * rate * card.multiplier).checked_div(card.unit_size)
}

/// The lines a usage fee bills for the usage of one window; shared by the invoice and the live
/// credit projection so both price identically. `unit_price_override` is the per-group unit price
/// an operator pinned on a draft line.
pub(crate) fn usage_lines(
    model: &UsagePricingModel,
    usage: &[GroupedUsageData],
    precision: u8,
    unit_price_override: impl Fn(&HashMap<String, String>) -> Option<Decimal>,
) -> StoreResult<Vec<pricing::PricedLine>> {
    rated_usage_lines(&model.into(), usage, precision, unit_price_override)
}

/// [`usage_lines`] for a pricing model already resolved, e.g. a rate card's entries.
pub(crate) fn rated_usage_lines(
    model: &pricing::UsageModel,
    usage: &[GroupedUsageData],
    precision: u8,
    unit_price_override: impl Fn(&HashMap<String, String>) -> Option<Decimal>,
) -> StoreResult<Vec<pricing::PricedLine>> {
    pricing::usage_lines(model, usage, precision, unit_price_override)
        .map_err(pricing_error)
        .attach("Failed to convert price_total to subunit")
}

/// The overage lines of a capacity fee for the usage of one window.
pub(crate) fn capacity_overage_lines(
    included: Decimal,
    overage_rate: &Decimal,
    usage: &[GroupedUsageData],
    precision: u8,
) -> StoreResult<Vec<pricing::PricedLine>> {
    pricing::capacity_overage_lines(included, overage_rate, usage, precision)
        .map_err(pricing_error)
        .attach("Capacity overage exceeds the supported range")
}

/// Credit a pack sells, in whole minor units of the invoice currency. A fractional minor unit is
/// refused rather than rounded: the grant must be exactly what was sold.
pub(crate) fn pack_credit_minor(
    credit_amount: Decimal,
    units: Decimal,
    precision: u8,
) -> StoreResult<i64> {
    pricing::pack_credit_minor(credit_amount, units, precision).map_err(pricing_error)
}

/// The rollover an included pack line freezes: the cap is per pack, like the credit it grants.
pub(crate) fn pack_rollover(
    rollover: &crate::domain::price_components::CreditRollover,
    units: Decimal,
    precision: u8,
) -> StoreResult<crate::domain::credit::GrantRollover> {
    Ok(crate::domain::credit::GrantRollover {
        max_minor: rollover
            .max_amount
            .map(|max| pack_credit_minor(max, units, precision))
            .transpose()?,
        expiry: rollover.expiry,
    })
}

#[cfg(test)]
mod pack_credit_tests {
    use super::{pack_credit_minor, pack_rollover};
    use crate::domain::price_components::{CreditExpiry, CreditRollover};
    use rust_decimal_macros::dec;

    #[test]
    fn an_included_pack_freezes_its_rollover_cap_per_pack_in_minor_units() {
        let rollover = CreditRollover {
            max_amount: Some(dec!(200)),
            expiry: CreditExpiry::Month(1),
        };
        let frozen = pack_rollover(&rollover, dec!(3), 2).unwrap();
        assert_eq!(frozen.max_minor, Some(60_000));
        assert_eq!(frozen.expiry, CreditExpiry::Month(1));
        let uncapped = CreditRollover {
            max_amount: None,
            ..rollover
        };
        assert_eq!(
            pack_rollover(&uncapped, dec!(3), 2).unwrap().max_minor,
            None
        );
    }

    #[test]
    fn a_fractional_grant_is_refused() {
        assert!(pack_credit_minor(dec!(0.005), dec!(1), 2).is_err());
    }
}

#[cfg(test)]
mod credit_unit_pin_tests {
    use super::*;
    use crate::domain::LineItemKind;
    use crate::domain::invoice_lines::LineCreditUnit;
    use chrono::NaiveDate;
    use common_domain::ids::{BaseId, CreditUnitId};
    use rust_decimal_macros::dec;

    fn usage_line(
        metric_id: BillableMetricId,
        sub_component_id: SubscriptionPriceComponentId,
        unit_price: Option<Decimal>,
        credit_unit: Option<LineCreditUnit>,
    ) -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        LineItem {
            local_id: "l".to_owned(),
            name: "API calls".to_owned(),
            charge_type: None,
            amount_subtotal: 0,
            tax_rate: dec!(0.2),
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: Vec::new(),
            quantity: Some(dec!(10)),
            unit_price,
            start_date: day,
            end_date: day,
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: Some(sub_component_id),
            sub_add_on_id: None,
            product_id: None,
            metric_id: Some(metric_id),
            description: None,
            group_by_dimensions: Some(HashMap::new()),
            kind: LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
            usage_breakdown: None,
        }
    }

    fn block(unit_id: CreditUnitId) -> LineCreditUnit {
        LineCreditUnit {
            unit_id,
            code: "CREDITS".to_owned(),
            precision: 2,
            rate: dec!(0.01),
            price_rate: None,
            unit_price: Some(dec!(3)),
            sub_lines: Vec::new(),
            consumed: 3_000,
            covered: 1_000,
        }
    }

    fn fee_unit(unit_id: CreditUnitId) -> FeeCreditUnit {
        FeeCreditUnit {
            unit_id,
            code: "CREDITS".to_owned(),
            precision: 2,
            rate: dec!(0.01),
        }
    }

    #[test]
    fn a_credit_priced_line_pins_its_price_in_credits_untaxed() {
        let (metric, component, unit) = (
            BillableMetricId::new(),
            SubscriptionPriceComponentId::new(),
            CreditUnitId::new(),
        );
        let line = usage_line(metric, component, None, Some(block(unit)));
        let mut pinned = PinnedPrices::default();
        pinned.add_lines(std::slice::from_ref(&line), true);
        assert_eq!(
            pinned.unit_price(metric, Some(component), None, &HashMap::new()),
            Some(dec!(3)),
            "the price in credits, never grossed up by the line's tax rate"
        );
    }

    #[test]
    fn a_pin_never_crosses_from_money_to_credits_or_between_units() {
        let (metric, component, unit) = (
            BillableMetricId::new(),
            SubscriptionPriceComponentId::new(),
            CreditUnitId::new(),
        );
        let money = usage_line(metric, component, Some(dec!(0.5)), None);
        let credits = usage_line(metric, component, None, Some(block(unit)));
        assert_eq!(pinned_unit_price(&money, None), Some(dec!(0.5)));
        assert_eq!(pinned_unit_price(&money, Some(&fee_unit(unit))), None);
        assert_eq!(pinned_unit_price(&credits, None), None);
        assert_eq!(
            pinned_unit_price(&credits, Some(&fee_unit(unit))),
            Some(dec!(3))
        );
        assert_eq!(
            pinned_unit_price(&credits, Some(&fee_unit(CreditUnitId::new()))),
            None
        );
    }
}

#[cfg(test)]
mod unit_conversion_tests {
    use super::*;
    use crate::domain::enums::BillingMetricAggregateEnum;
    use crate::domain::{Period, UsagePeriod};
    use crate::services::credit::billing_windows::{PlannedWindow, price_window_groups};
    use crate::services::invoice_lines::invoice_lines::MeteredFee;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use common_domain::ids::{BaseId, ProductFamilyId, SubscriptionId, TenantId};
    use rust_decimal_macros::dec;

    /// A usage line as the breakdown tests read it.
    struct InvoiceLineInner {
        total: i64,
        group_by_dimensions: Option<HashMap<String, String>>,
        usage_breakdown: Option<UsageBreakdown>,
    }

    fn usage_lines(
        model: &UsagePricingModel,
        _metric: BillableMetricId,
        usage: &[GroupedUsageData],
        _period: &Period,
        precision: u8,
        pin: impl Fn(&HashMap<String, String>) -> Option<Decimal>,
    ) -> StoreResult<Vec<InvoiceLineInner>> {
        let lines = super::usage_lines(model, usage, precision, pin)?;
        Ok(lines
            .into_iter()
            .map(|line| InvoiceLineInner {
                total: line.total,
                group_by_dimensions: line.dimensions,
                usage_breakdown: None,
            })
            .collect())
    }

    /// Attaches to each line the usage of `usage`, read grouped by `breakdown_key` too, that the line
    /// priced. A line rated by dimension weighs each item by its rates; any other by its quantity.
    fn breakdown_lines(
        lines: &mut [InvoiceLineInner],
        metric: &BillableMetric,
        breakdown_key: &str,
        rated: Option<&pricing::CardRates>,
        usage: &[GroupedUsageData],
    ) {
        for line in lines.iter_mut() {
            if let Some(breakdown) = line_breakdown(
                line.group_by_dimensions.as_ref(),
                metric,
                breakdown_key,
                rated,
                usage,
            ) {
                line.usage_breakdown = Some(breakdown);
            }
        }
    }

    fn tokens(rounding: UnitConversionRoundingEnum) -> BillableMetric {
        BillableMetric {
            id: BillableMetricId::new(),
            name: "Tokens".to_owned(),
            description: None,
            code: "tokens".to_owned(),
            catalog_code: "tokens".to_owned(),
            aggregation_type: BillingMetricAggregateEnum::Sum,
            aggregation_key: Some("tokens".to_owned()),
            unit_conversion_factor: Some(1_000_000),
            unit_conversion_rounding: Some(rounding),
            unit_singular: Some("token".to_owned()),
            unit_plural: Some("tokens".to_owned()),
            segmentation_matrix: None,
            filters: None,
            usage_group_key: None,
            usage_breakdown_key: None,
            created_at: NaiveDateTime::default(),
            updated_at: None,
            archived_at: None,
            tenant_id: TenantId::new(),
            product_family_id: ProductFamilyId::new(),
            product_id: None,
            usage_scope: Default::default(),
            rated_dimensions: Vec::new(),
        }
    }

    #[test]
    fn invoice_and_live_projection_price_rounded_usage_alike() {
        let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let period = Period { start, end };
        let model = UsagePricingModel::PerUnit { rate: dec!(12) };
        let raw = || {
            vec![
                GroupedUsageData {
                    value: dec!(9_004_601),
                    dimensions: HashMap::from([("model".to_owned(), "standard".to_owned())]),
                },
                GroupedUsageData {
                    value: dec!(250_000),
                    dimensions: HashMap::from([("model".to_owned(), "lite".to_owned())]),
                },
            ]
        };

        for (rounding, expected_minor) in [
            (UnitConversionRoundingEnum::Up, Some(13_200)),
            (UnitConversionRoundingEnum::Down, Some(10_800)),
            (UnitConversionRoundingEnum::Nearest, Some(10_800)),
            (UnitConversionRoundingEnum::None, None),
        ] {
            let metric = tokens(rounding);
            let invoiced = convert_usage_units(
                &metric,
                UsageData {
                    data: raw(),
                    period: UsagePeriod {
                        start: start.and_time(NaiveTime::MIN),
                        end: end.and_time(NaiveTime::MIN),
                    },
                },
            );
            let invoice_minor: i64 =
                usage_lines(&model, metric.id, &invoiced.data, &period, 2, |_| None)
                    .unwrap()
                    .iter()
                    .map(|line| line.total)
                    .sum();

            let window = PlannedWindow {
                subscription_id: SubscriptionId::new(),
                metric: metric.clone(),
                fee: MeteredFee::Usage(model.clone()),
                sub_component_id: None,
                sub_add_on_id: None,
                product_id: None,
                tax_inclusive: false,
                taxes: Vec::new(),
                period: period.clone(),
                start: start.and_time(NaiveTime::MIN),
                end: end.and_time(NaiveTime::MIN),
                currency: "USD".to_owned(),
                precision: 2,
                overage: None,
                card: None,
            };
            let projected_minor = price_window_groups(
                &window,
                end.and_time(NaiveTime::MIN),
                raw(),
                &PinnedPrices::default(),
            )
            .unwrap();

            if let Some(expected) = expected_minor {
                assert_eq!(invoice_minor, expected);
            }
            assert_eq!(projected_minor, invoice_minor);
        }
    }

    fn breakdown_metric() -> BillableMetric {
        BillableMetric {
            unit_conversion_factor: None,
            unit_conversion_rounding: None,
            usage_breakdown_key: Some("project".to_owned()),
            ..tokens(UnitConversionRoundingEnum::None)
        }
    }

    fn row(value: Decimal, dimensions: &[(&str, &str)]) -> GroupedUsageData {
        GroupedUsageData {
            value,
            dimensions: dimensions
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn june() -> Period {
        Period {
            start: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        }
    }

    fn items(line: &InvoiceLineInner) -> Vec<(Option<&str>, Decimal, i64)> {
        let breakdown = line.usage_breakdown.as_ref().expect("a breakdown line");
        breakdown
            .items
            .iter()
            .zip(breakdown.allocate(line.total))
            .map(|(item, amount)| (item.value.as_deref(), item.quantity, amount))
            .collect()
    }

    #[test]
    fn a_breakdown_shares_the_package_price_of_the_whole_usage() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::Package {
            block_size: 100,
            rate: dec!(10),
        };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[row(dec!(150), &[])],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(120), &[("project", "alpha")]),
                row(dec!(30), &[("project", "beta")]),
            ],
        );

        // Billed apart, 120 and 30 would take three packages; together they take two.
        assert_eq!(lines[0].total, 2_000);
        assert_eq!(
            items(&lines[0]),
            vec![
                (Some("alpha"), dec!(120), 1_600),
                (Some("beta"), dec!(30), 400)
            ]
        );
    }

    #[test]
    fn a_blank_value_counts_as_no_value() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(&model, metric.id, &[row(dec!(10), &[])], &june(), 2, |_| {
            None
        })
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(6), &[("project", "alpha")]),
                row(dec!(2), &[]),
                row(dec!(1), &[("project", "")]),
                row(dec!(1), &[("project", "  ")]),
            ],
        );

        assert_eq!(
            items(&lines[0]),
            vec![(Some("alpha"), dec!(6), 600), (None, dec!(4), 400)]
        );
    }

    #[test]
    fn a_breakdown_breaks_down_each_group_line_on_its_own() {
        let metric = BillableMetric {
            usage_group_key: Some("region".to_owned()),
            ..breakdown_metric()
        };
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[
                row(dec!(10), &[("region", "eu")]),
                row(dec!(5), &[("region", "us")]),
            ],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(7), &[("region", "eu"), ("project", "alpha")]),
                row(dec!(3), &[("region", "eu")]),
                row(dec!(5), &[("region", "us"), ("project", "alpha")]),
            ],
        );

        let line = |region: &str| {
            lines
                .iter()
                .find(|line| line.group_by_dimensions.as_ref().unwrap()["region"] == region)
                .unwrap()
        };
        assert_eq!(
            items(line("eu")),
            vec![(Some("alpha"), dec!(7), 700), (None, dec!(3), 300)]
        );
        assert_eq!(items(line("us")), vec![(Some("alpha"), dec!(5), 500)]);
    }

    #[test]
    fn a_breakdown_by_dimension_weighs_each_value_by_its_rates() {
        let metric = breakdown_metric();
        let rate = |model: &str, rate: Decimal| crate::domain::price_components::DimensionRate {
            key: vec![model.to_owned()],
            rate,
            label: None,
        };
        let rates = crate::domain::price_components::DimensionRates {
            dimensions: vec!["model".to_owned()],
            rates: vec![rate("large", dec!(10)), rate("small", dec!(1))],
            unit_size: Decimal::ONE,
            on_unmatched: crate::domain::rate_cards::RateCardUnmatched::Ignore,
        };
        let model = UsagePricingModel::DimensionRates(rates.clone());
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[
                row(dec!(1), &[("model", "large")]),
                row(dec!(10), &[("model", "small")]),
            ],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            Some(&rates.card_rates()),
            &[
                row(dec!(1), &[("model", "large"), ("project", "alpha")]),
                row(dec!(10), &[("model", "small"), ("project", "beta")]),
            ],
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].total, 2_000);
        let mut by_value = items(&lines[0]);
        by_value.sort();
        assert_eq!(
            by_value,
            vec![
                (Some("alpha"), dec!(1), 1_000),
                (Some("beta"), dec!(10), 1_000)
            ]
        );
    }

    #[test]
    fn a_breakdown_past_its_listed_values_sums_the_rest_as_others() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[row(dec!(150), &[])],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        let values = (0..150).map(|i| format!("p{i:03}")).collect::<Vec<_>>();
        let usage = values
            .iter()
            .map(|value| row(Decimal::ONE, &[("project", value.as_str())]))
            .collect::<Vec<_>>();
        breakdown_lines(&mut lines, &metric, "project", None, &usage);

        let breakdown = lines[0].usage_breakdown.as_ref().unwrap();
        assert_eq!(breakdown.items.len(), MAX_BREAKDOWN_ITEMS);
        let others = breakdown.items.last().unwrap();
        assert!(others.others && others.value.is_none());
        assert_eq!(others.quantity, dec!(51));
        assert_eq!(breakdown.allocate(15_000).iter().sum::<i64>(), 15_000);
    }

    #[test]
    fn breakdown_quantities_are_converted_without_rounding() {
        let metric = BillableMetric {
            usage_breakdown_key: Some("project".to_owned()),
            ..tokens(UnitConversionRoundingEnum::Up)
        };
        let model = UsagePricingModel::PerUnit { rate: dec!(12) };
        let usage = convert_usage_units(
            &metric,
            UsageData {
                data: vec![row(dec!(1_500_000), &[])],
                period: UsagePeriod {
                    start: june().start.and_time(NaiveTime::MIN),
                    end: june().end.and_time(NaiveTime::MIN),
                },
            },
        );
        let mut lines = usage_lines(&model, metric.id, &usage.data, &june(), 2, |_| None).unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(1_000_000), &[("project", "alpha")]),
                row(dec!(500_000), &[("project", "beta")]),
            ],
        );

        // Two million tokens billed, rounded up from 1.5M: the items keep the usage they carry.
        assert_eq!(lines[0].total, 2_400);
        assert_eq!(
            items(&lines[0]),
            vec![
                (Some("alpha"), dec!(1), 1_600),
                (Some("beta"), dec!(0.5), 800)
            ]
        );
    }
}

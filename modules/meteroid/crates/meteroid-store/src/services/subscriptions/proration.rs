use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, MatchedComponent, ProrationLineItem, RemovedComponent,
};
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_utils::decimals::ToSubunit;
use rust_decimal::Decimal;

/// Per-period advance-billed amount (cents). Returns 0 for arrears/usage/onetime.
/// Mirrors the pattern from calculate_mrr.
pub fn component_advance_amount_cents(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    if matches!(period, SubscriptionFeeBillingPeriod::OneTime) {
        return 0;
    }

    match fee {
        SubscriptionFee::Rate { rate } => rate.to_subunit_opt(precision).unwrap_or(0),
        SubscriptionFee::Recurring {
            rate,
            quantity,
            billing_type,
        } => {
            use crate::domain::enums::BillingType;
            match billing_type {
                BillingType::Advance => {
                    let total = *rate * rust_decimal::Decimal::from(*quantity);
                    total.to_subunit_opt(precision).unwrap_or(0)
                }
                BillingType::Arrears => 0,
            }
        }
        SubscriptionFee::Capacity { rate, .. } => rate.to_subunit_opt(precision).unwrap_or(0),
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => i64::from(*initial_slots) * unit_rate.to_subunit_opt(precision).unwrap_or(0),
        // A pack is never prorated on a plan change: an adjustment line funds no credit, so
        // prorating would charge without a grant or refund a charge whose credit was granted.
        SubscriptionFee::Credit { .. }
        | SubscriptionFee::OneTime { .. }
        | SubscriptionFee::Usage { .. } => 0,
    }
}

/// Total (non-prorated) amount for a one-time fee. One-time charges are billed
/// in full when added mid-period — they are never prorated.
pub fn component_onetime_amount_cents(fee: &SubscriptionFee, precision: u8) -> i64 {
    match fee {
        SubscriptionFee::OneTime { rate, quantity } => (*rate
            * rust_decimal::Decimal::from(*quantity))
        .to_subunit_opt(precision)
        .unwrap_or(0),
        _ => 0,
    }
}

/// Display instance count encoded in a fee: slot count, or recurring/one-time
/// quantity. Flat fees (Rate, Capacity) and usage are a single unit. Used so a
/// prorated adjustment-invoice line shows "N × effective_rate" instead of
/// "1 × total" for a multi-instance component. Only valid for an unscaled fee
/// (scale_fee folds the multiplier into Rate/Capacity rates, losing the count).
pub(crate) fn fee_instance_count(fee: &SubscriptionFee) -> rust_decimal::Decimal {
    use rust_decimal::Decimal;
    match fee {
        SubscriptionFee::Slot { initial_slots, .. } => Decimal::from(*initial_slots),
        SubscriptionFee::Recurring { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::OneTime { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::Credit { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::Rate { .. }
        | SubscriptionFee::Capacity { .. }
        | SubscriptionFee::Usage { .. } => Decimal::ONE,
    }
}

pub use billing_core::{DayFraction, DaySpan};

/// Monthly share of an amount billed every `period`, for comparisons only.
fn per_month(amount: Decimal, period: &SubscriptionFeeBillingPeriod) -> Decimal {
    match period.as_months() {
        0 => Decimal::ZERO,
        months => amount / Decimal::from(months),
    }
}

/// What a credit pack adds per month to its denomination, compared per denomination.
#[derive(Default, Clone, Copy)]
struct PackTotals {
    fee: Decimal,
    credit: Decimal,
    /// `None` within `Some`: rolls over everything left.
    rollover: Option<Option<Decimal>>,
}

impl PackTotals {
    fn add(&mut self, fee: &SubscriptionFee, period: &SubscriptionFeeBillingPeriod) {
        let SubscriptionFee::Credit {
            rate,
            quantity,
            credit_amount,
            rollover,
            ..
        } = fee
        else {
            return;
        };
        let units = Decimal::from(*quantity);
        self.fee += per_month(*rate * units, period);
        if is_period_tied(fee) {
            self.credit += per_month(*credit_amount * units, period);
        }
        if let Some(rollover) = rollover {
            let cap = rollover
                .max_amount
                .map(|max| per_month(max * units, period));
            self.rollover = Some(match (self.rollover, cap) {
                (Some(None), _) | (_, None) => None,
                (Some(Some(total)), Some(cap)) => Some(total + cap),
                (None, Some(cap)) => Some(cap),
            });
        }
    }
}

fn rollover_cmp(new: Option<Option<Decimal>>, old: Option<Option<Decimal>>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (new, old) {
        (None, None) | (Some(None), Some(None)) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(None), Some(Some(_))) => Ordering::Greater,
        (Some(Some(_)), Some(None)) => Ordering::Less,
        (Some(Some(new)), Some(Some(old))) => new.cmp(&old),
    }
}

/// A pack whose credit ends with the billing period (with or without rollover): billed and
/// granted by the share of the period it covers. Any other pack is bought whole.
pub fn is_period_tied(fee: &SubscriptionFee) -> bool {
    matches!(
        fee,
        SubscriptionFee::Credit {
            expiry: Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            once: false,
            ..
        }
    )
}

fn pack_denomination(fee: &SubscriptionFee) -> Option<String> {
    match fee {
        SubscriptionFee::Credit {
            once: false,
            credit_unit,
            ..
        } => Some(
            credit_unit
                .as_ref()
                .map_or_else(String::new, |u| u.code.clone()),
        ),
        _ => None,
    }
}

/// Upgrade, downgrade or lateral, compared per month at the price the plan lists (the MRR basis):
/// the fixed fees in total, then per credit denomination the packs' fees, the credit of the packs
/// tied to the period and their rollover cap, and the billing period's length. Any decrease, a
/// credit denomination removed, or a shorter period is a downgrade, even beside an increase;
/// otherwise any increase is an upgrade. Usage prices are not compared.
pub fn detect_change_direction(
    matched: &[MatchedComponent],
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    precision: u8,
) -> ChangeDirection {
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    // A fixed fee compares by what it bills a period, in advance or in arrears.
    let fee = |fee: &SubscriptionFee, period: &SubscriptionFeeBillingPeriod| {
        let amount = match fee {
            SubscriptionFee::Recurring { rate, quantity, .. }
                if !matches!(period, SubscriptionFeeBillingPeriod::OneTime) =>
            {
                (*rate * Decimal::from(*quantity))
                    .to_subunit_opt(precision)
                    .unwrap_or(0)
            }
            _ => component_advance_amount_cents(fee, period, precision),
        };
        per_month(Decimal::from(amount), period)
    };
    let mut outcomes: Vec<Ordering> = Vec::new();
    let mut old_packs: BTreeMap<String, PackTotals> = BTreeMap::new();
    let mut new_packs: BTreeMap<String, PackTotals> = BTreeMap::new();
    let mut old_period: Option<i32> = None;
    let mut new_period: Option<i32> = None;
    let shortest = |acc: &mut Option<i32>, period: &SubscriptionFeeBillingPeriod| {
        if period.as_months() > 0 {
            *acc = Some(acc.map_or(period.as_months(), |m| m.min(period.as_months())));
        }
    };

    let mut old_side = |f: &SubscriptionFee, p: &SubscriptionFeeBillingPeriod| {
        shortest(&mut old_period, p);
        if let Some(key) = pack_denomination(f) {
            old_packs.entry(key).or_default().add(f, p);
        }
    };
    for m in matched {
        old_side(&m.current_fee, &m.current_period);
    }
    for r in removed {
        old_side(&r.current_fee, &r.current_period);
    }
    let mut new_side = |f: &SubscriptionFee, p: &SubscriptionFeeBillingPeriod| {
        shortest(&mut new_period, p);
        if let Some(key) = pack_denomination(f) {
            new_packs.entry(key).or_default().add(f, p);
        }
    };
    for m in matched {
        new_side(&m.new_fee, &m.new_period);
    }
    for a in added {
        new_side(&a.fee, &a.period);
    }

    // Fixed fees compare in total, whatever products carry them: catalogs bill the base fee of
    // each plan under its own product.
    let fixed = |fee_and_period: &mut dyn Iterator<
        Item = (&SubscriptionFee, &SubscriptionFeeBillingPeriod),
    >| {
        fee_and_period
            .filter(|(f, _)| !matches!(f, SubscriptionFee::Credit { .. }))
            .map(|(f, p)| fee(f, p))
            .sum::<Decimal>()
    };
    let old_fixed = fixed(
        &mut matched
            .iter()
            .map(|m| (&m.current_fee, &m.current_period))
            .chain(removed.iter().map(|r| (&r.current_fee, &r.current_period))),
    );
    let new_fixed = fixed(
        &mut matched
            .iter()
            .map(|m| (&m.new_fee, &m.new_period))
            .chain(added.iter().map(|a| (&a.fee, &a.period))),
    );
    outcomes.push(new_fixed.cmp(&old_fixed));
    for (key, old) in &old_packs {
        match new_packs.get(key) {
            None => outcomes.push(Ordering::Less),
            Some(new) => {
                outcomes.push(new.fee.cmp(&old.fee));
                outcomes.push(new.credit.cmp(&old.credit));
                outcomes.push(rollover_cmp(new.rollover, old.rollover));
            }
        }
    }
    if new_packs.keys().any(|key| !old_packs.contains_key(key)) {
        outcomes.push(Ordering::Greater);
    }
    if let (Some(old), Some(new)) = (old_period, new_period)
        && new < old
    {
        outcomes.push(Ordering::Less);
    }

    if outcomes.contains(&Ordering::Less) {
        ChangeDirection::Downgrade
    } else if outcomes.contains(&Ordering::Greater) {
        ChangeDirection::Upgrade
    } else {
        ChangeDirection::Lateral
    }
}

/// The fee credited back for an old pack tied to the period: its unused time, but never more than
/// its unused paid credit is worth, so spending the credit and then changing plan refunds nothing.
pub fn capped_pack_fee_credit(
    fee_minor: i64,
    unused_time: DayFraction,
    unused_paid: i64,
    paid_face: i64,
) -> i64 {
    let by_time = unused_time.prorate(fee_minor);
    if paid_face <= 0 {
        return by_time;
    }
    by_time.min(DayFraction::new(unused_paid, paid_face).prorate(fee_minor))
}

/// Where a change on a date reads each fee's own period (R-083, R-084): on the subscription's
/// cadence, its current period; on another, the period of the fee's own grid, the latest change of
/// cadence's, else the base grid from the billing start or a free trial's end. Each is the full
/// period its price is for; the fixed term's end cuts what is left of it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnGrid {
    /// The subscription's period holding the change, as its price is for: never cut by the
    /// term's end, a first partial period's full one.
    pub current: (NaiveDate, NaiveDate),
    /// Where the subscription's current period started.
    pub renewed: NaiveDate,
    pub term_end: Option<NaiveDate>,
    pub cadence: crate::domain::enums::BillingPeriodEnum,
    pub billing_start: NaiveDate,
    pub billing_from: NaiveDate,
    pub billing_day: u32,
    pub run: Option<crate::utils::periods::CadenceRun>,
}

impl OwnGrid {
    /// The grids of `sub`, whose current period is `current`, at `change_date`.
    pub(crate) async fn read(
        conn: &mut crate::store::PgConn,
        sub: &crate::domain::Subscription,
        current: (NaiveDate, NaiveDate),
        change_date: NaiveDate,
    ) -> crate::StoreResult<Self> {
        use crate::services::invoice_lines::cadence::{CadenceSubject, cadence_runs_of};
        let run = cadence_runs_of(
            conn,
            CadenceSubject::of_subscription(sub),
            change_date,
            true,
        )
        .await?
        .into_iter()
        .next();
        let billing_start = sub.billing_start_date.unwrap_or(sub.current_period_start);
        let billing_from = crate::services::subscriptions::billing_from_tx(conn, sub).await?;
        let billing_day = u32::from(sub.billing_day_anchor);
        let grid = match &run {
            Some(run) => billing_core::timeline::Grid::from_origin(
                run.origin,
                billing_day,
                sub.period.as_months(),
            ),
            None => crate::utils::periods::base_grid(
                billing_start,
                billing_from,
                billing_day,
                sub.period.as_months(),
            ),
        };
        Ok(OwnGrid {
            current: grid.map_or(current, |grid| priced_over(&grid, current, sub.end_date)),
            renewed: current.0,
            term_end: sub.end_date,
            cadence: sub.period,
            billing_start,
            billing_from,
            billing_day,
            run,
        })
    }

    /// Where what is left of a period ending on `end` stops: there, or at the term's end before.
    pub(crate) fn until(&self, end: NaiveDate) -> NaiveDate {
        self.term_end.map_or(end, |term_end| term_end.min(end))
    }

    /// What is left from `at` of `period`, a fee's own, up to the term's end (R-079, R-084).
    pub(crate) fn left(&self, period: (NaiveDate, NaiveDate), at: NaiveDate) -> DaySpan {
        DaySpan {
            period,
            from: at,
            until: self.until(period.1),
        }
    }

    /// The subscription's current period as it bills, and the share of its price left at `at`.
    pub(crate) fn billed(&self, at: NaiveDate) -> (NaiveDate, NaiveDate, DayFraction) {
        (
            self.current.0.max(self.renewed),
            self.until(self.current.1),
            self.left(self.current, at).fraction(),
        )
    }

    /// The same, renewing on `cadence` over `current`: an amendment moving the cadence.
    pub(crate) fn renewing(
        self,
        current: (NaiveDate, NaiveDate),
        cadence: crate::domain::enums::BillingPeriodEnum,
    ) -> Self {
        OwnGrid {
            current,
            cadence,
            ..self
        }
    }
}

/// The full period `current`, the subscription's as stored, is priced over on `grid` (R-084): a
/// period the term's end cuts, whole; a first partial period, the full one ending with it. Off
/// the grid (a history no rule produced), itself.
fn priced_over(
    grid: &billing_core::timeline::Grid,
    current: (NaiveDate, NaiveDate),
    term_end: Option<NaiveDate>,
) -> (NaiveDate, NaiveDate) {
    match grid.period_containing(current.0) {
        Some(period)
            if period.0 == current.0
                && (period.1 == current.1
                    || (term_end == Some(current.1) && current.1 < period.1)) =>
        {
            grid.nominal(period)
        }
        _ => current,
    }
}

/// The period a fee of `period` bills on at `change_date` (see [`OwnGrid`]).
pub(crate) fn own_period(
    grid: OwnGrid,
    change_date: NaiveDate,
) -> impl Fn(&SubscriptionFeeBillingPeriod) -> (NaiveDate, NaiveDate) + Sync + use<> {
    move |period| match period.as_billing_period_opt() {
        Some(own) if own.as_months() != grid.cadence.as_months() => {
            let months = own.as_months();
            match grid.run {
                Some(run) => {
                    billing_core::timeline::Grid::from_origin(run.origin, grid.billing_day, months)
                }
                None => crate::utils::periods::base_grid(
                    grid.billing_start,
                    grid.billing_from,
                    grid.billing_day,
                    months,
                ),
            }
            .and_then(|own| {
                let period = own.period_containing(change_date)?;
                // A change before the grid starts (during a free trial) reads the current period.
                (period.0 <= change_date).then(|| own.nominal(period))
            })
            .unwrap_or(grid.current)
        }
        _ => grid.current,
    }
}

/// Net override credit/charge pairs that share a `net_key` and a period into a single line,
/// so the adjustment invoice taxes the delta rather than a gross charge beside
/// an untaxed credit. Lines without a key (genuine adds/removes) pass through.
/// Zero-net lines are dropped.
pub fn net_override_lines(lines: &[ProrationLineItem]) -> Vec<ProrationLineItem> {
    let mut netted: Vec<ProrationLineItem> = Vec::new();
    let mut index_by_key: std::collections::HashMap<(String, Option<NaiveDate>), usize> =
        std::collections::HashMap::new();

    for line in lines {
        match &line.net_key {
            // A price moved onto another period is another charge: its credit is a note's (R-080).
            Some(key) => {
                let key = (key.clone(), line.period_end);
                if let Some(&idx) = index_by_key.get(&key) {
                    netted[idx].amount_cents += line.amount_cents;
                    // Prefer the charge (new price) line's identity, and the row it bills.
                    if !line.is_credit {
                        netted[idx].name = line.name.clone();
                        netted[idx].is_prorated = line.is_prorated;
                        netted[idx].product_id = line.product_id;
                        netted[idx].price_component_id = line.price_component_id;
                        netted[idx].sub_component_id = line.sub_component_id;
                        netted[idx].sub_add_on_id = line.sub_add_on_id;
                    }
                } else {
                    index_by_key.insert(key, netted.len());
                    netted.push(line.clone());
                }
            }
            None => netted.push(line.clone()),
        }
    }

    for nl in &mut netted {
        if nl.net_key.is_some() {
            let base = nl
                .name
                .rsplit_once(" (")
                .map(|(b, _)| b.to_string())
                .unwrap_or_else(|| nl.name.clone());
            nl.name = format!("{base} (adjustment)");
            nl.is_credit = nl.amount_cents < 0;
        }
    }

    // An included pack bills nothing but grants its credit.
    netted
        .into_iter()
        .filter(|l| l.amount_cents != 0 || l.pack.is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn rate_fee(amount: i64) -> SubscriptionFee {
        SubscriptionFee::Rate {
            rate: Decimal::new(amount, 0),
        }
    }

    fn usage_fee() -> SubscriptionFee {
        use common_domain::ids::BaseId;
        SubscriptionFee::Usage {
            metric_id: common_domain::ids::BillableMetricId::new(),
            model: crate::domain::UsagePricingModel::PerUnit { rate: Decimal::ONE },
            credit_unit: None,
        }
    }

    fn slot_fee(slots: u32, rate: i64) -> SubscriptionFee {
        SubscriptionFee::Slot {
            unit: "seat".to_string(),
            unit_rate: Decimal::new(rate, 0),
            min_slots: None,
            max_slots: None,
            initial_slots: slots,
        }
    }

    fn monthly() -> SubscriptionFeeBillingPeriod {
        SubscriptionFeeBillingPeriod::Monthly
    }

    fn product_id() -> common_domain::ids::ProductId {
        use common_domain::ids::BaseId;
        common_domain::ids::ProductId::new()
    }

    #[test]
    fn test_downgrade_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Pro".to_string(),
            current_fee: rate_fee(200),
            current_period: monthly(),
            new_name: "Basic".to_string(),
            new_fee: rate_fee(100),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Downgrade);
    }

    #[test]
    fn test_upgrade_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Upgrade);
    }

    #[test]
    fn test_lateral_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Plan A".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Plan B".to_string(),
            new_fee: rate_fee(100),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Lateral);
    }

    /// A fee billed in arrears raised from 30.00 to 50.00 is an upgrade, as one in advance is.
    /// Rules: X5
    #[test]
    fn an_arrears_fee_compares_by_its_amount() {
        use crate::domain::enums::BillingType;
        let arrears = |cents: i64| SubscriptionFee::Recurring {
            rate: Decimal::new(cents, 2),
            quantity: 1,
            billing_type: BillingType::Arrears,
        };
        let matched = |from: i64, to: i64| MatchedComponent {
            product_id: product_id(),
            current_name: "Support".to_string(),
            current_fee: arrears(from),
            current_period: monthly(),
            new_name: "Support".to_string(),
            new_fee: arrears(to),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        };
        assert_eq!(
            detect_change_direction(&[matched(3_000, 5_000)], &[], &[], 2),
            ChangeDirection::Upgrade
        );
        assert_eq!(
            detect_change_direction(&[matched(5_000, 3_000)], &[], &[], 2),
            ChangeDirection::Downgrade
        );
    }

    #[test]
    fn test_direction_with_added_components() {
        let added = vec![AddedComponent {
            name: "New Feature".to_string(),
            fee: rate_fee(100),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&[], &added, &[], 2);
        assert_eq!(direction, ChangeDirection::Upgrade);
    }

    #[test]
    fn test_direction_with_removed_components() {
        let removed = vec![RemovedComponent {
            name: "Old Feature".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            net_key: None,
        }];

        let direction = detect_change_direction(&[], &[], &removed, 2);
        assert_eq!(direction, ChangeDirection::Downgrade);
    }

    /// Switched to quarterly with an origin on April 15, a yearly fee reads its year on the grid from
    /// there (April 15 to April 15), not from the billing start (January 1); a quarterly fee reads
    /// the subscription's current period.
    #[test]
    fn a_fee_off_cadence_after_a_change_of_cadence_reads_its_period_on_the_change_s_grid() {
        let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        let grid = OwnGrid {
            current: (day(7, 15), day(10, 15)),
            renewed: day(7, 15),
            term_end: None,
            cadence: crate::domain::enums::BillingPeriodEnum::Quarterly,
            billing_start: day(1, 1),
            billing_from: day(1, 1),
            billing_day: 15,
            run: Some(crate::utils::periods::CadenceRun {
                switched_on: day(3, 20),
                origin: day(4, 15),
                previous_renewal: day(7, 15),
            }),
        };
        let own = own_period(grid, day(8, 1));
        assert_eq!(
            own(&SubscriptionFeeBillingPeriod::Annual),
            (day(4, 15), NaiveDate::from_ymd_opt(2025, 4, 15).unwrap())
        );
        assert_eq!(
            own(&SubscriptionFeeBillingPeriod::Quarterly),
            (day(7, 15), day(10, 15))
        );
    }

    /// A yearly fee changed on February 5, during a free trial ending February 15, reads the
    /// subscription's current period: its grid starts only at the trial's end.
    #[test]
    fn a_fee_off_cadence_changed_during_a_free_trial_reads_the_current_period() {
        let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        let grid = OwnGrid {
            current: (day(2, 1), day(2, 15)),
            renewed: day(2, 1),
            term_end: None,
            cadence: crate::domain::enums::BillingPeriodEnum::Monthly,
            billing_start: day(2, 1),
            billing_from: day(2, 15),
            billing_day: 15,
            run: None,
        };
        let own = own_period(grid, day(2, 5));
        assert_eq!(
            own(&SubscriptionFeeBillingPeriod::Annual),
            (day(2, 1), day(2, 15))
        );
    }

    fn line(name: &str, amount: i64, is_credit: bool, net_key: Option<&str>) -> ProrationLineItem {
        ProrationLineItem {
            period_end: None,
            name: name.to_string(),
            amount_cents: amount,
            full_period_amount_cents: amount.abs(),
            is_credit,
            is_prorated: true,
            quantity: None,
            unit_price: None,
            product_id: None,
            price_component_id: None,
            net_key: net_key.map(str::to_string),
            sub_component_id: None,
            sub_add_on_id: None,
            charge_type: None,
            pack: None,
        }
    }

    #[test]
    fn test_net_override_price_drop_nets_to_single_credit() {
        // Override 100 -> 50: old credit -100, new charge +50, same key → net -50.
        let netted = net_override_lines(&[
            line("Base (credit)", -10000, true, Some("c1")),
            line("Base (prorated)", 5000, false, Some("c1")),
        ]);
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].amount_cents, -5000);
        assert!(netted[0].is_credit);
        assert_eq!(netted[0].name, "Base (adjustment)");
    }

    #[test]
    fn test_net_override_price_increase_nets_to_single_charge() {
        // Override 50 -> 100: credit -50, charge +100 → net +50 (taxed on the delta).
        let netted = net_override_lines(&[
            line("Base (credit)", -5000, true, Some("c1")),
            line("Base (prorated)", 10000, false, Some("c1")),
        ]);
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].amount_cents, 5000);
        assert!(!netted[0].is_credit);
    }

    #[test]
    fn test_net_override_keeps_unkeyed_lines_separate() {
        // Genuine add + genuine remove (no key) are not netted together.
        let netted = net_override_lines(&[
            line("Add A (prorated)", 3000, false, None),
            line("Remove B (credit)", -2000, true, None),
        ]);
        assert_eq!(netted.len(), 2);
    }

    #[test]
    fn test_net_override_keeps_a_price_on_another_period_apart() {
        // A monthly price moved onto a year: the month's credit and the year's charge stay apart.
        let day = |y, m| NaiveDate::from_ymd_opt(y, m, 1);
        let mut credit = line("Base (credit)", -1590, true, Some("c1"));
        credit.period_end = day(2024, 2);
        let mut charge = line("Base (prorated)", 27891, false, Some("c1"));
        charge.period_end = day(2025, 1);
        let netted = net_override_lines(&[credit, charge]);
        let amounts: Vec<i64> = netted.iter().map(|l| l.amount_cents).collect();
        assert_eq!(amounts, vec![-1590, 27891]);
    }

    #[test]
    fn test_net_override_zero_net_is_dropped() {
        let netted = net_override_lines(&[
            line("Base (credit)", -5000, true, Some("c1")),
            line("Base (prorated)", 5000, false, Some("c1")),
        ]);
        assert!(netted.is_empty());
    }

    #[test]
    fn fee_instance_count_reads_count_from_fee() {
        use crate::domain::enums::BillingType;
        use rust_decimal_macros::dec;

        // Count encoded in the fee.
        assert_eq!(fee_instance_count(&slot_fee(3, 10)), dec!(3));
        assert_eq!(
            fee_instance_count(&SubscriptionFee::Recurring {
                rate: dec!(10),
                quantity: 5,
                billing_type: BillingType::Advance,
            }),
            dec!(5)
        );
        assert_eq!(
            fee_instance_count(&SubscriptionFee::OneTime {
                rate: dec!(10),
                quantity: 2,
            }),
            dec!(2)
        );

        // Flat / countless fees are a single unit (shared match arm: Rate | Capacity | Usage).
        assert_eq!(fee_instance_count(&rate_fee(100)), dec!(1));
        assert_eq!(fee_instance_count(&usage_fee()), dec!(1));
    }

    fn pack(
        rate: i64,
        credit: i64,
        expiry: Option<crate::domain::price_components::CreditExpiry>,
        unit: Option<&str>,
        rollover_cap: Option<i64>,
    ) -> SubscriptionFee {
        use crate::domain::price_components::{CreditExpiry, CreditRollover};
        SubscriptionFee::Credit {
            rate: Decimal::new(rate, 0),
            quantity: 1,
            credit_amount: Decimal::new(credit, 0),
            expiry,
            credit_unit: unit.map(|code| crate::domain::credit_units::CreditUnitRef {
                unit_id: {
                    use common_domain::ids::BaseId;
                    common_domain::ids::CreditUnitId::new()
                },
                code: code.to_owned(),
                precision: 2,
                product_id: None,
            }),
            rollover: rollover_cap.map(|cap| CreditRollover {
                max_amount: Some(Decimal::new(cap, 0)),
                expiry: CreditExpiry::Month(1),
            }),
            once: false,
        }
    }

    fn period_pack(rate: i64, credit: i64) -> SubscriptionFee {
        pack(
            rate,
            credit,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            None,
            None,
        )
    }

    fn matched(current: SubscriptionFee, new: SubscriptionFee) -> MatchedComponent {
        MatchedComponent {
            product_id: product_id(),
            current_name: "Current".to_string(),
            current_fee: current,
            current_period: monthly(),
            new_name: "New".to_string(),
            new_fee: new,
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }
    }

    fn added(fee: SubscriptionFee) -> AddedComponent {
        AddedComponent {
            name: "Added".to_string(),
            fee,
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }
    }

    fn removed(fee: SubscriptionFee) -> RemovedComponent {
        RemovedComponent {
            name: "Removed".to_string(),
            current_fee: fee,
            current_period: monthly(),
            net_key: None,
        }
    }

    #[test]
    fn a_day_fraction_rounds_the_exact_ratio_half_away_from_zero() {
        let f = DayFraction::remaining(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        );
        // 50000 × 16 / 31 = 25806.45…; 55000 × 16 / 31 = 28387.09…
        assert_eq!(f.prorate(50_000), 25_806);
        assert_eq!(f.prorate(55_000), 28_387);
        assert_eq!(f.prorate(-50_000), -25_806);
        // 15 of 30 days of 1 cent is exactly half a cent: away from zero, both ways.
        let half = DayFraction::new(15, 30);
        assert_eq!(half.prorate(1), 1);
        assert_eq!(half.prorate(-1), -1);
        assert_eq!(half.prorate(3), 2);
        assert_eq!(DayFraction::new(40, 30).prorate(3_000), 3_000);
        assert_eq!(DayFraction::new(5, 0).prorate(1_000), 0);
    }

    #[test]
    fn a_pack_fee_credit_is_capped_by_the_unused_paid_credit() {
        let half = DayFraction::new(15, 30);
        // Nothing used: the unused time decides.
        assert_eq!(capped_pack_fee_credit(50_000, half, 50_000, 50_000), 25_000);
        // A fifth left: a fifth of the fee, though half the period is left.
        assert_eq!(capped_pack_fee_credit(50_000, half, 10_000, 50_000), 10_000);
        // Everything spent, then the change: nothing back.
        assert_eq!(capped_pack_fee_credit(50_000, half, 0, 50_000), 0);
        // Never a grant (unpaid): only time.
        assert_eq!(capped_pack_fee_credit(50_000, half, 0, 0), 25_000);
        // Rounding of the cap: 50000 × 1 / 3 = 16666.67.
        assert_eq!(
            capped_pack_fee_credit(50_000, DayFraction::WHOLE, 1, 3),
            16_667
        );
    }

    #[test]
    fn direction_compares_packs_per_denomination() {
        let studio_credits = pack(
            0,
            5_000,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            Some("CREDITS"),
            Some(1_000),
        );
        // Scale -> Studio: the EUR pack goes, Studio's fees are lower.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(124), rate_fee(39))],
                &[added(studio_credits.clone())],
                &[removed(period_pack(500, 550))],
                2
            ),
            ChangeDirection::Downgrade
        );
        // Studio -> Scale: higher fees and a EUR pack, but the CREDITS unit is removed.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(39), rate_fee(124))],
                &[added(period_pack(500, 550))],
                &[removed(studio_credits.clone())],
                2
            ),
            ChangeDirection::Downgrade
        );
        // Free -> Scale: only additions.
        assert_eq!(
            detect_change_direction(&[], &[added(period_pack(500, 550))], &[], 2),
            ChangeDirection::Upgrade
        );
        // Scale -> a bigger EUR pack under another product: compared per denomination.
        assert_eq!(
            detect_change_direction(
                &[],
                &[added(period_pack(1_000, 1_100))],
                &[removed(period_pack(500, 550))],
                2
            ),
            ChangeDirection::Upgrade
        );
        // Same pack fee, less credit.
        assert_eq!(
            detect_change_direction(
                &[matched(period_pack(500, 550), period_pack(500, 500))],
                &[],
                &[],
                2
            ),
            ChangeDirection::Downgrade
        );
        // A lower rollover cap alone is a downgrade.
        let lower_cap = pack(
            0,
            5_000,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            Some("CREDITS"),
            Some(500),
        );
        assert_eq!(
            detect_change_direction(&[matched(studio_credits.clone(), lower_cap)], &[], &[], 2),
            ChangeDirection::Downgrade
        );
        // Mixed: fixed fees up in total, the pack's credit down.
        assert_eq!(
            detect_change_direction(
                &[
                    matched(rate_fee(100), rate_fee(200)),
                    matched(period_pack(500, 550), period_pack(500, 500))
                ],
                &[],
                &[],
                2
            ),
            ChangeDirection::Downgrade
        );
        // A pack removed, nothing else changes.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(100), rate_fee(100))],
                &[],
                &[removed(studio_credits)],
                2
            ),
            ChangeDirection::Downgrade
        );
        // The same plan: lateral.
        assert_eq!(
            detect_change_direction(
                &[matched(period_pack(500, 550), period_pack(500, 550))],
                &[],
                &[],
                2
            ),
            ChangeDirection::Lateral
        );
    }

    #[test]
    fn fixed_fees_compare_in_total_across_products() {
        // Starter's 29 base fee and Pro's 99 under different products: an upgrade.
        assert_eq!(
            detect_change_direction(&[], &[added(rate_fee(99))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Upgrade
        );
        // A recurring fee and a slot fee replaced by a single higher rate: still an upgrade.
        assert_eq!(
            detect_change_direction(
                &[],
                &[added(rate_fee(150))],
                &[removed(rate_fee(29)), removed(slot_fee(5, 10))],
                2
            ),
            ChangeDirection::Upgrade
        );
        // Lower in total: a downgrade.
        assert_eq!(
            detect_change_direction(&[], &[added(slot_fee(2, 10))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Downgrade
        );
        // Same total: lateral.
        assert_eq!(
            detect_change_direction(&[], &[added(rate_fee(29))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Lateral
        );
    }

    #[test]
    fn a_shorter_period_is_a_downgrade_even_at_the_same_monthly_price() {
        let mut m = matched(rate_fee(1_200), rate_fee(100));
        m.current_period = SubscriptionFeeBillingPeriod::Annual;
        assert_eq!(
            detect_change_direction(&[m], &[], &[], 2),
            ChangeDirection::Downgrade
        );
    }
}

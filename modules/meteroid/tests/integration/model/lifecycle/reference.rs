//! The lifecycle reference: one subscription on two plans of a fixed fee, a metered fee and seats,
//! monthly or annual, an add-on, maybe a free trial, a fixed term, a coupon, a minimum or a
//! companion subscription consolidated with it; its life's money computed per day from the rows
//! its actions write, nothing shared with the engine. USD, no tax or credit: the documents'
//! subtotals are what the fees bill, coupons and minimums checked against them.
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, NaiveTime, Utc};

pub type At = DateTime<Utc>;

/// Backdated documents are issued during their day; the next action starts after them.
pub const DOCUMENTS: Duration = Duration::hours(13);

pub fn midnight(day: NaiveDate) -> At {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// The last period end renewed.
pub fn last_renewal() -> NaiveDate {
    NaiveDate::from_ymd_opt(2028, 12, 31).expect("a day")
}

/// `num / den` rounded once, half away from zero (R-001).
fn round(num: i128, den: i128) -> i64 {
    let (q, r) = (num / den, num % den);
    let q = if 2 * r.abs() >= den {
        q + num.signum()
    } else {
        q
    };
    i64::try_from(q).expect("an amount")
}

fn shift(day: NaiveDate, months: i64) -> NaiveDate {
    let by = Months::new(u32::try_from(months.unsigned_abs()).expect("months"));
    if months >= 0 {
        day.checked_add_months(by)
    } else {
        day.checked_sub_months(by)
    }
    .expect("a day")
}

/// The period of `months` holding `day` on the grid through `origin`, before it as after it
/// (R-084); the anchor is the start's day, never past the 28th, so no month clamps it (R-015).
pub fn period(origin: NaiveDate, months: u32, day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let months = i64::from(months.max(1));
    let elapsed = i64::from(day.year() - origin.year()) * 12 + i64::from(day.month())
        - i64::from(origin.month());
    let mut k = elapsed.div_euclid(months);
    loop {
        let start = shift(origin, k * months);
        let end = shift(origin, (k + 1) * months);
        if start > day {
            k -= 1;
        } else if end <= day {
            k += 1;
        } else {
            return (start, end);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanSpec {
    /// The fixed fee per period, in cents.
    pub fee: i64,
    pub arrears: bool,
    /// The cadence of both its fees: 1 or 12.
    pub months: u32,
    /// The metered fee per unit, in whole cents, so a usage line is exact.
    pub rate: i64,
    /// A seat's price per period, in advance on the plan's cadence.
    pub seat: i64,
}

/// An add-on of the catalog: a fee per period and per instance, in advance; plan A offers it,
/// plan B when `on_b`. A plan change to a plan not offering it ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddOnSpec {
    pub price: i64,
    pub months: u32,
    pub on_b: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    Percent(i64),
    /// Cents.
    Fixed(i64),
}

/// A coupon taken at the start: off its first `invoices` period invoices, or all of them. Its
/// expiry closes its redemption only, as Stripe's `redeem_by` (R-089).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouponSpec {
    pub off: Off,
    pub invoices: Option<u32>,
    pub expires: Option<NaiveDate>,
}

/// Plan A's capacity on the metered units: a committed fee and an allowance, prorated together
/// whenever the fee is (a change, an ending, a term's end, a first partial period), and overage
/// past it, billed as consumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacitySpec {
    /// The committed fee per period, in advance.
    pub fee: i64,
    pub included: i64,
    /// Per unit past the allowance, in cents.
    pub overage: i64,
}

/// A free trial to `end`: billing starts then (R-084). Without a card the trial expires there,
/// waiting for a checkout, and bills nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrialSpec {
    pub end: NaiveDate,
    pub card: bool,
    /// The billing day, when the trial's end is not on the start's.
    pub anchor: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub plans: [PlanSpec; 2],
    /// Whether documents finalize on their own; else they stay drafts until finalized.
    pub auto_advance: bool,
    /// The seats the subscription starts with.
    pub seats: i64,
    pub add_on: AddOnSpec,
    pub coupon: Option<CouponSpec>,
    /// A minimum per period, kept across plan changes (an override, R-022): monthly plans only.
    pub minimum: Option<i64>,
    pub trial: Option<TrialSpec>,
    /// A fixed term's end: its fees end there as on a cancellation that day (R-002).
    pub term_end: Option<NaiveDate>,
    /// A second subscription of the customer, a monthly fee in advance from the same day, its
    /// renewals consolidated with this one's; it ends with it.
    pub companion: Option<i64>,
    pub capacity: Option<CapacitySpec>,
}

impl Catalog {
    pub fn offers_add_on(&self, plan: usize) -> bool {
        plan == 0 || self.add_on.on_b
    }

    pub fn billing_start(&self, start: NaiveDate) -> NaiveDate {
        self.trial.map_or(start, |trial| trial.end)
    }
}

pub fn plan_name(plan: usize) -> &'static str {
    ["A", "B"][plan]
}

pub fn fee_line(plan: usize) -> String {
    format!("{} fee", plan_name(plan))
}

pub fn units_line(plan: usize) -> String {
    format!("{} units", plan_name(plan))
}

pub fn seats_line(plan: usize) -> String {
    format!("{} seats", plan_name(plan))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Fixed,
    Usage,
    OneTime,
}

/// A fee row: the lineage it bills under, in force over `[from, to)`.
#[derive(Clone, Debug)]
pub struct Row {
    pub lineage: usize,
    pub kind: RowKind,
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
    pub price: i64,
    pub months: u32,
    /// A capacity's allowance and its overage rate, per unit in cents.
    pub capacity: Option<(i64, i64)>,
}

/// The plan's cadence from `from` on, its grid through `origin`.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub from: NaiveDate,
    pub months: u32,
    pub origin: NaiveDate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheduled {
    Amend(i64),
    Change(usize),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct Sub {
    pub plan: usize,
    pub start: NaiveDate,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub active: bool,
    pub scheduled: Option<Scheduled>,
    /// The ending's date: fees bill the days before it, usage up to `cutoff` (R-016).
    pub ended: Option<NaiveDate>,
    /// The instant usage stops counting: the ending's midnight, or the instant of a cancellation
    /// effective immediately.
    pub cutoff: Option<At>,
    pub fixed: usize,
    pub usage: usize,
    pub seats: usize,
    pub seat_count: i64,
    /// Seats removed from the end of the seats' period then, whatever plan holds them by then: the
    /// ledger counts per seat unit, never per plan.
    pub seats_off: Vec<(NaiveDate, i64)>,
    /// What the seats a change scheduled for the period end sets add to those held when it was
    /// scheduled; else it carries them. The seats bought or removed since carry either way (R-108).
    pub change_seats: Option<i64>,
    /// Plan A's capacity lineage, while on it.
    pub capacity: Option<usize>,
    /// The add-on's lineage and its instances, while attached.
    pub add_on: Option<(usize, i64)>,
    /// A trial ended without a card: the subscription waits for a checkout, billing nothing.
    pub expired: bool,
    /// The periods closed so far, after the billing start: `[from, to)` and the full period's
    /// days.
    pub periods: Vec<(NaiveDate, NaiveDate, i64)>,
}

impl Sub {
    pub fn seats_off(&self) -> i64 {
        self.seats_off.iter().map(|(_, n)| n).sum()
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Start {
        day: NaiveDate,
        catalog: Catalog,
    },
    Usage {
        at: At,
        units: i64,
    },
    Advance {
        at: At,
    },
    /// The plan's fixed fee edited from today on its lineage; at another cadence its metered fee
    /// moves with it, and the plan's grid with them.
    AmendNow {
        at: At,
        plan: usize,
        price: i64,
        months: u32,
        rate: i64,
    },
    AmendAtEnd {
        at: At,
        plan: usize,
        price: i64,
        months: u32,
    },
    ChangeNow {
        at: At,
        to: usize,
    },
    ChangeAtEnd {
        at: At,
        to: usize,
        /// The seats the new plan starts with, set explicitly.
        seats: Option<i64>,
    },
    /// An ad-hoc one-time fee added today.
    OneTime {
        at: At,
        price: i64,
    },
    /// A cancellation dated today: the period ends at today's midnight.
    CancelNow {
        at: At,
    },
    /// A cancellation effective now: today's fees end today, usage counts up to now.
    CancelImmediately {
        at: At,
    },
    /// A cancellation dated an earlier day of the period.
    CancelBackdated {
        at: At,
        day: NaiveDate,
    },
    /// Seats bought today, billed for what is left of the period (R-079).
    BuySeats {
        at: At,
        seats: i64,
    },
    /// Seats removed at the period end.
    RemoveSeats {
        at: At,
        seats: i64,
    },
    AttachAddOn {
        at: At,
        quantity: i64,
    },
    AddOnQuantity {
        at: At,
        quantity: i64,
    },
    DetachAddOn {
        at: At,
    },
    CancelAtEnd {
        at: At,
        end: NaiveDate,
    },
    Void {
        at: At,
        pick: u8,
    },
    Pay {
        at: At,
        pick: u8,
    },
    Finalize {
        at: At,
        pick: u8,
    },
    /// Every draft finalized, in the order written: the life's documents are then all issued.
    FinalizeAll {
        at: At,
    },
}

impl Action {
    pub fn at(&self) -> At {
        match self {
            Self::Start { day, .. } => midnight(*day),
            Self::Usage { at, .. }
            | Self::Advance { at }
            | Self::AmendNow { at, .. }
            | Self::AmendAtEnd { at, .. }
            | Self::ChangeNow { at, .. }
            | Self::ChangeAtEnd { at, .. }
            | Self::OneTime { at, .. }
            | Self::CancelNow { at }
            | Self::CancelImmediately { at }
            | Self::CancelBackdated { at, .. }
            | Self::BuySeats { at, .. }
            | Self::RemoveSeats { at, .. }
            | Self::AttachAddOn { at, .. }
            | Self::AddOnQuantity { at, .. }
            | Self::DetachAddOn { at }
            | Self::CancelAtEnd { at, .. }
            | Self::Void { at, .. }
            | Self::Pay { at, .. }
            | Self::Finalize { at, .. }
            | Self::FinalizeAll { at } => *at,
        }
    }

    pub fn done(&self) -> At {
        match self {
            Self::Advance { at } => *at + DOCUMENTS,
            other => other.at(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub &'static str);

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub now: Option<At>,
    pub catalog: Option<Catalog>,
    pub sub: Option<Sub>,
    pub rows: Vec<Row>,
    pub windows: Vec<Window>,
    pub usage: Vec<(At, i64)>,
    lineages: usize,
    /// A plan change moved the subscription onto a grid of its own.
    own_grid: bool,
}

impl Model {
    pub fn now(&self) -> At {
        self.now.expect("started")
    }

    pub fn catalog(&self) -> &Catalog {
        self.catalog.as_ref().expect("started")
    }

    pub fn sub(&self) -> &Sub {
        self.sub.as_ref().expect("started")
    }

    fn sub_mut(&mut self) -> &mut Sub {
        self.sub.as_mut().expect("started")
    }

    /// The cadence in force now: the plan's, or the one an amendment moved its fees to.
    pub fn months(&self) -> u32 {
        self.months_on(self.now().date_naive())
    }

    fn lineage(&mut self) -> usize {
        self.lineages += 1;
        self.lineages
    }

    /// The grid in force on `day`: the latest plan window from it on, through its origin, or the
    /// start when the origin is not after it (R-084).
    fn grid_on(&self, day: NaiveDate) -> (NaiveDate, u32) {
        let start = self.sub().start;
        let window = self
            .windows
            .iter()
            .rev()
            .find(|window| window.from <= day)
            .or(self.windows.first())
            .expect("a plan window");
        (window.origin.max(start), window.months)
    }

    fn period_end_on(&self, day: NaiveDate) -> (NaiveDate, NaiveDate) {
        let (origin, months) = self.grid_on(day);
        let (start, end) = period(origin, months, day);
        (
            start,
            self.catalog().term_end.map_or(end, |term| end.min(term)),
        )
    }

    /// Whether `day` is in the free trial.
    pub fn trialing(&self, day: NaiveDate) -> bool {
        self.catalog().trial.is_some_and(|trial| day < trial.end)
    }

    fn set_period(&mut self, on: NaiveDate) {
        let (start, end) = if self.trialing(on) {
            (self.sub().start, self.catalog().trial.expect("a trial").end)
        } else {
            self.period_end_on(on)
        };
        let sub = self.sub_mut();
        sub.period_start = start;
        sub.period_end = end;
    }

    /// The period `[period_start, to)` closes: kept for the minimum's true-up (R-022).
    fn close_period(&mut self, to: NaiveDate) {
        let from = self.sub().period_start;
        if from >= to || self.trialing(from) {
            return;
        }
        let (origin, months) = self.grid_on(from);
        let (start, end) = period(origin, months, from);
        self.sub_mut()
            .periods
            .push((from, to, (end - start).num_days()));
    }

    /// Seats removed from a period's end, applied once that day is reached.
    fn remove_seats(&mut self, upto: NaiveDate, inclusive: bool) {
        let mut due: Vec<(NaiveDate, i64)> = self
            .sub()
            .seats_off
            .iter()
            .copied()
            .filter(|(on, _)| *on < upto || (inclusive && *on == upto))
            .collect();
        due.sort();
        self.sub_mut()
            .seats_off
            .retain(|(on, _)| !(*on < upto || (inclusive && *on == upto)));
        for (on, n) in due {
            let count = self.sub().seat_count - n;
            self.seat(on, count);
        }
    }

    fn close(&mut self, lineage: usize, on: NaiveDate) {
        for row in self.rows.iter_mut().filter(|row| row.lineage == lineage) {
            row.to = Some(row.to.map_or(on, |to| to.min(on)));
        }
    }

    fn open(&mut self, lineage: usize, kind: RowKind, from: NaiveDate, price: i64, months: u32) {
        self.rows.push(Row {
            lineage,
            kind,
            from,
            to: None,
            price,
            months,
            capacity: None,
        });
    }

    /// R-084: a change of cadence on `on` renews first on its day when that is a boundary of the
    /// grid in force, else on the new cadence's first boundary after it through the old grid's
    /// next one.
    fn origin(&self, on: NaiveDate, months: u32) -> NaiveDate {
        let (origin, before) = self.grid_on(on);
        if before == months {
            return self.windows.last().map_or(origin, |window| window.origin);
        }
        let (start, end) = period(origin, before, on);
        if start == on {
            on
        } else {
            period(end, months, on).1
        }
    }

    fn rewindow(&mut self, on: NaiveDate, months: u32) {
        // A window opened the day the last one did replaces it, as the store lays them.
        if self.windows.len() > 1 && self.windows.last().is_some_and(|w| w.from == on) {
            self.windows.pop();
        }
        // Before the billing start the grid starts there.
        let origin = match self.catalog().trial {
            Some(trial) if on < trial.end => trial.end,
            _ => self.origin(on, months),
        };
        self.windows.push(Window {
            from: on,
            months,
            origin,
        });
        self.set_period(on);
    }

    fn amend(&mut self, on: NaiveDate, price: i64, months: u32) {
        let (fixed, usage) = (self.sub().fixed, self.sub().usage);
        let rate = self
            .rows
            .iter()
            .rev()
            .find(|r| r.lineage == usage)
            .map_or(0, |r| r.price);
        let moved = months != self.months_on(on);
        self.close(fixed, on);
        self.open(fixed, RowKind::Fixed, on, price, months);
        if moved {
            self.close(usage, on);
            self.open(usage, RowKind::Usage, on, rate, months);
        }
        self.realign(on);
    }

    /// R-084: an amendment is no change of the subscription's cadence; it renews on the shortest
    /// cadence of the fees in force on the grid it was created on, unless a plan change gave it a
    /// grid of its own.
    fn realign(&mut self, on: NaiveDate) {
        let Some(months) = self
            .rows
            .iter()
            .filter(|row| row.kind != RowKind::OneTime)
            .filter(|row| row.from <= on && row.to.is_none_or(|to| on < to))
            .map(|row| row.months)
            .min()
        else {
            return;
        };
        // An amendment realigns an active subscription only, never one in its trial.
        if self.own_grid || self.trialing(on) || months == self.months_on(on) {
            return;
        }
        let origin = self.windows.last().map_or(on, |window| window.origin);
        if self.windows.len() > 1 && self.windows.last().is_some_and(|w| w.from == on) {
            self.windows.pop();
        }
        self.windows.push(Window {
            from: on,
            months,
            origin,
        });
        self.set_period(on);
    }

    /// The seats from `on` on: a row of their lineage at the count's price.
    fn seat(&mut self, on: NaiveDate, count: i64) {
        let (seats, plan) = (self.sub().seats, self.sub().plan);
        let spec = self.catalog().plans[plan];
        self.close(seats, on);
        self.open(seats, RowKind::Fixed, on, spec.seat * count, spec.months);
        self.sub_mut().seat_count = count;
    }

    fn add_on_rows(&mut self, on: NaiveDate, quantity: Option<i64>) {
        if let Some((lineage, _)) = self.sub().add_on {
            self.close(lineage, on);
        }
        let spec = self.catalog().add_on;
        self.sub_mut().add_on = match quantity {
            Some(quantity) => {
                let lineage = match self.sub().add_on {
                    Some((lineage, _)) => lineage,
                    None => self.lineage(),
                };
                self.open(
                    lineage,
                    RowKind::Fixed,
                    on,
                    spec.price * quantity,
                    spec.months,
                );
                Some((lineage, quantity))
            }
            None => None,
        };
    }

    fn end(&mut self, day: NaiveDate, cutoff: At) {
        self.close_period(day);
        let sub = self.sub_mut();
        sub.active = false;
        sub.ended = Some(day);
        sub.cutoff = Some(cutoff);
        sub.scheduled = None;
        sub.seats_off.clear();
    }

    fn months_on(&self, day: NaiveDate) -> u32 {
        self.grid_on(day).1
    }

    fn change(&mut self, on: NaiveDate, to: usize, count: i64) {
        // A change on the start day or in the trial leaves it as if created on the new plan.
        self.own_grid |= self.catalog().plans[to].months != self.months_on(on)
            && on > self.catalog().billing_start(self.sub().start);
        let (fixed, usage, seats) = (self.sub().fixed, self.sub().usage, self.sub().seats);
        self.close(fixed, on);
        self.close(usage, on);
        self.close(seats, on);
        if let Some(capacity) = self.sub_mut().capacity.take() {
            self.close(capacity, on);
        }
        if !self.catalog().offers_add_on(to) {
            self.add_on_rows(on, None);
        }
        let spec = self.catalog().plans[to];
        self.sub_mut().seat_count = count;
        let (fixed, usage, seats) = (self.lineage(), self.lineage(), self.lineage());
        self.open(fixed, RowKind::Fixed, on, spec.fee, spec.months);
        self.open(usage, RowKind::Usage, on, spec.rate, spec.months);
        self.open(seats, RowKind::Fixed, on, spec.seat * count, spec.months);
        let sub = self.sub_mut();
        sub.plan = to;
        sub.fixed = fixed;
        sub.usage = usage;
        sub.seats = seats;
        if to == 0 {
            self.open_capacity(on);
        }
        self.rewindow(on, spec.months);
    }

    fn open_capacity(&mut self, on: NaiveDate) {
        let Some(spec) = self.catalog().capacity else {
            return;
        };
        let lineage = self.lineage();
        let months = self.catalog().plans[0].months;
        self.open(lineage, RowKind::Fixed, on, spec.fee, months);
        if let Some(row) = self.rows.last_mut() {
            row.capacity = Some((spec.included, spec.overage));
        }
        self.sub_mut().capacity = Some(lineage);
    }

    pub fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        if self.sub.as_ref().is_some_and(|sub| sub.active)
            && !matches!(action, Action::Advance { .. })
        {
            self.remove_seats(action.at().date_naive(), true);
        }
        match action {
            Action::Start { day, catalog } => {
                let spec = catalog.plans[0];
                self.catalog = Some(catalog.clone());
                let (fixed, usage, seats) = (self.lineage(), self.lineage(), self.lineage());
                let origin = catalog.billing_start(*day);
                self.windows.push(Window {
                    from: *day,
                    months: spec.months,
                    origin,
                });
                self.sub = Some(Sub {
                    plan: 0,
                    start: *day,
                    period_start: *day,
                    period_end: *day,
                    active: true,
                    scheduled: None,
                    ended: None,
                    cutoff: None,
                    fixed,
                    usage,
                    seats,
                    seat_count: catalog.seats,
                    seats_off: vec![],
                    change_seats: None,
                    capacity: None,
                    add_on: None,
                    expired: false,
                    periods: vec![],
                });
                self.open_capacity(*day);
                self.set_period(*day);
                self.open(fixed, RowKind::Fixed, *day, spec.fee, spec.months);
                self.open(usage, RowKind::Usage, *day, spec.rate, spec.months);
                self.open(
                    seats,
                    RowKind::Fixed,
                    *day,
                    spec.seat * catalog.seats,
                    spec.months,
                );
            }
            Action::Usage { at, units } => self.usage.push((*at, *units)),
            Action::Advance { at } => {
                let end = at.date_naive();
                self.remove_seats(end, false);
                let trial = self.catalog().trial.filter(|trial| trial.end == end);
                let term = self.catalog().term_end == Some(end);
                let ending = self.sub().scheduled == Some(Scheduled::Cancel)
                    || term
                    || trial.is_some_and(|trial| !trial.card);
                if !ending {
                    self.close_period(end);
                }
                match self.sub().scheduled {
                    Some(Scheduled::Cancel) => self.end(end, midnight(end)),
                    _ if term => self.end(end, midnight(end)),
                    // Without a card the trial expires, waiting for a checkout (R-084).
                    _ if trial.is_some_and(|trial| !trial.card) => {
                        self.end(end, midnight(end));
                        self.sub_mut().expired = true;
                    }
                    Some(Scheduled::Change(to)) => {
                        let count = self.sub().seat_count + self.sub().change_seats.unwrap_or(0);
                        self.change(end, to, count);
                    }
                    Some(Scheduled::Amend(price)) => {
                        let months = self.months_on(end);
                        self.amend(end, price, months);
                    }
                    None => {}
                }
                if self.sub().active {
                    self.remove_seats(end, true);
                    self.set_period(end);
                }
                self.sub_mut().scheduled = None;
            }
            Action::AmendNow {
                at, price, months, ..
            } => {
                // Under a minimum an immediate amendment only raises; anything else waits for the
                // period's end.
                let day = at.date_naive();
                let fixed = self.sub().fixed;
                let raises = self
                    .row_on(fixed, day)
                    .is_some_and(|row| row.months == *months && *price > row.price);
                if self.catalog().minimum.is_some() && !raises {
                    self.now = Some(action.done());
                    return Err(Refused("a minimum commitment"));
                }
                self.amend(day, *price, *months)
            }
            Action::AmendAtEnd { .. } | Action::ChangeAtEnd { .. }
                if self
                    .catalog()
                    .term_end
                    .is_some_and(|end| end <= self.sub().period_end) =>
            {
                // A change for a day the subscription has ended by is never written (R-092).
                self.now = Some(action.done());
                return Err(Refused("the subscription ends before the change"));
            }
            Action::AmendAtEnd { price, .. } => {
                self.sub_mut().scheduled = Some(Scheduled::Amend(*price));
            }
            Action::ChangeNow { at, to } => {
                // R-022: a minimum keeps a plan change to the period's end.
                if self.catalog().minimum.is_some() {
                    self.now = Some(action.done());
                    return Err(Refused("a minimum commitment"));
                }
                let count = self.sub().seat_count;
                self.change(at.date_naive(), *to, count);
            }
            Action::ChangeAtEnd { to, seats, .. } => {
                let sub = self.sub_mut();
                sub.scheduled = Some(Scheduled::Change(*to));
                sub.change_seats = seats.map(|count| count - sub.seat_count);
                // Seats set by the change are its own: a removal requested before for its day
                // no longer applies.
                if seats.is_some() {
                    let end = sub.period_end;
                    sub.seats_off.retain(|(on, _)| *on != end);
                }
            }
            Action::OneTime { at, price } => {
                // R-106: refused while a plan change is scheduled; a scheduled amendment stands.
                if matches!(self.sub().scheduled, Some(Scheduled::Change(_))) {
                    self.now = Some(action.done());
                    return Err(Refused("a plan change is scheduled"));
                }
                let lineage = self.lineage();
                let months = self.months_on(at.date_naive());
                self.open(lineage, RowKind::OneTime, at.date_naive(), *price, months);
            }
            Action::CancelNow { at } => self.end(at.date_naive(), midnight(at.date_naive())),
            Action::CancelImmediately { at } => self.end(at.date_naive(), *at),
            Action::CancelBackdated { day, .. } => self.end(*day, midnight(*day)),
            Action::BuySeats { at, seats } => {
                let count = self.sub().seat_count + seats;
                self.seat(at.date_naive(), count);
            }
            Action::RemoveSeats { at, seats } => {
                let day = at.date_naive();
                let seats_row = self.sub().seats;
                let months = self.row_on(seats_row, day).map_or(1, |row| row.months);
                let (origin, _) = self.grid_on(day);
                let mut on = period(origin, months, day).1;
                if let Some(term) = self.catalog().term_end {
                    on = on.min(term);
                }
                self.sub_mut().seats_off.push((on, *seats));
            }
            Action::AttachAddOn { at, .. }
            | Action::AddOnQuantity { at, .. }
            | Action::DetachAddOn { at }
                if matches!(self.sub().scheduled, Some(Scheduled::Change(_))) =>
            {
                // An immediate amendment waits for a scheduled plan change to be cancelled.
                let _ = at;
                self.now = Some(action.done());
                return Err(Refused("a plan change is scheduled"));
            }
            Action::AddOnQuantity { quantity, .. }
                if self.catalog().minimum.is_some()
                    && self
                        .sub()
                        .add_on
                        .is_none_or(|(_, current)| *quantity <= current) =>
            {
                self.now = Some(action.done());
                return Err(Refused("a minimum commitment"));
            }
            Action::DetachAddOn { .. } if self.catalog().minimum.is_some() => {
                self.now = Some(action.done());
                return Err(Refused("a minimum commitment"));
            }
            Action::AttachAddOn { at, quantity } | Action::AddOnQuantity { at, quantity } => {
                self.add_on_rows(at.date_naive(), Some(*quantity));
                self.realign(at.date_naive());
            }
            Action::DetachAddOn { at } => {
                self.add_on_rows(at.date_naive(), None);
                self.realign(at.date_naive());
            }
            Action::CancelAtEnd { .. } => {
                self.sub_mut().scheduled = Some(Scheduled::Cancel);
            }
            Action::Void { .. }
            | Action::Pay { .. }
            | Action::Finalize { .. }
            | Action::FinalizeAll { .. } => {}
        }
        self.now = Some(action.done());
        Ok(())
    }

    fn row_on(&self, lineage: usize, day: NaiveDate) -> Option<&Row> {
        self.rows
            .iter()
            .rev()
            .filter(|row| row.lineage == lineage)
            .find(|row| row.from <= day && row.to.is_none_or(|to| day < to))
    }

    /// Once ended, what the life owes: each run of days at a price, over its own cadence's period
    /// (R-079, R-084, R-002, arrears cut by a cancellation too), seats and add-on instances priced
    /// at their count; each one-time fee once (R-075); each unit before the cutoff at that day's
    /// rate (R-002, R-016).
    pub fn owed(&self) -> Option<i64> {
        let sub = self.sub.as_ref()?;
        let end = sub.ended?;
        let mut lineages: Vec<usize> = self.rows.iter().map(|row| row.lineage).collect();
        lineages.sort_unstable();
        lineages.dedup();
        let mut total = 0;
        let billing_start = self.catalog().billing_start(sub.start);
        let cutoff = sub.cutoff.unwrap_or(midnight(end));
        for lineage in lineages {
            let capacity = self
                .rows
                .iter()
                .find(|row| row.lineage == lineage)
                .and_then(|row| row.capacity);
            let mut run: Option<((NaiveDate, NaiveDate), i64, NaiveDate, NaiveDate)> = None;
            let mut runs = Vec::new();
            // R-084: a run of one cadence keeps the grid of the window it started in.
            let mut grid: Option<(u32, NaiveDate, NaiveDate)> = None;
            let mut day = billing_start;
            while day < end {
                let fixed = self
                    .row_on(lineage, day)
                    .filter(|row| row.kind == RowKind::Fixed);
                if let Some(row) = fixed {
                    let origin = match grid {
                        Some((months, origin, until)) if months == row.months && until == day => {
                            origin
                        }
                        _ => self.grid_on(day).0,
                    };
                    grid = Some((row.months, origin, day + Duration::days(1)));
                    let of = period(origin, row.months, day);
                    let next = day + Duration::days(1);
                    let goes_on = run.is_some_and(|(p, price, _, until)| {
                        p == of && price == row.price && until == day
                    });
                    if goes_on {
                        if let Some(run) = run.as_mut() {
                            run.3 = next;
                        }
                    } else {
                        runs.extend(run.take());
                        run = Some((of, row.price, day, next));
                    }
                }
                day += Duration::days(1);
            }
            runs.extend(run);
            for ((start, stop), price, from, until) in runs {
                let whole = i128::from((stop - start).num_days());
                let left = |at: NaiveDate| {
                    round(
                        i128::from(price) * i128::from((stop - at).num_days()),
                        whole,
                    )
                };
                total += left(from) - left(until);
                // The allowance prorated with its fee; the units past it billed as consumed.
                if let Some((included, rate)) = capacity {
                    let used: i64 = self
                        .usage
                        .iter()
                        .filter(|(at, _)| {
                            *at >= midnight(from.max(billing_start))
                                && *at < midnight(until)
                                && *at < cutoff
                        })
                        .map(|(_, units)| units)
                        .sum();
                    let days = i128::from((until - from).num_days());
                    let over = i128::from(used) * whole - i128::from(included) * days;
                    if over > 0 {
                        total += round(over * i128::from(rate), whole);
                    }
                }
            }
        }
        total += self
            .rows
            .iter()
            .filter(|row| row.kind == RowKind::OneTime && row.from < end)
            .map(|row| row.price)
            .sum::<i64>();
        for (at, units) in &self.usage {
            if *at >= cutoff || *at < midnight(billing_start) {
                continue;
            }
            let day = at.date_naive();
            let rate = self
                .rows
                .iter()
                .rev()
                .find(|row| {
                    row.kind == RowKind::Usage
                        && row.from <= day
                        && row.to.is_none_or(|to| day < to)
                })
                .map_or(0, |row| row.price);
            total += units * rate;
        }
        if let Some(fee) = self.catalog().companion {
            total += companion_owed(sub.start, end, fee);
        }
        Some(total)
    }
}

/// A monthly fee in advance from `start`, each month billed whole, the last one to `end` (R-079).
fn companion_owed(start: NaiveDate, end: NaiveDate, fee: i64) -> i64 {
    let mut total = 0;
    let mut from = start;
    while from < end {
        let next = shift(from, 1);
        let until = next.min(end);
        let whole = i128::from((next - from).num_days());
        let left =
            |at: NaiveDate| round(i128::from(fee) * i128::from((next - at).num_days()), whole);
        total += left(from) - left(until);
        from = next;
    }
    total
}

/// `num / den` rounded once, half away from zero (R-001), for the checks on documents.
pub fn rounded(num: i128, den: i128) -> i64 {
    round(num, den)
}

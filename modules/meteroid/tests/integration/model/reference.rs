//! The reference model: one customer, its subscriptions on Pay as you go and Scale, its credit
//! grants, invoices, payments and credit notes, computed straight from DECISIONS (Untaxed, USD).
//! Every rule cites its decision; nothing here is shared with the engine.
use chrono::{DateTime, Duration, Months, NaiveDate, NaiveTime, Utc};

pub type At = DateTime<Utc>;

pub const PACK_FEE: i64 = 50_000;
pub const PACK_CREDIT: i64 = 55_000;
pub const WELCOME: i64 = 5_000;
pub const PROMO: i64 = 10_000;
const WELCOME_DAYS: i64 = 90;
const PROMO_DAYS: i64 = 25;

pub fn midnight(day: NaiveDate) -> At {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// The end of a subscription's `cycle`-th period: periods are anchored on its start day.
pub fn period_end(start: NaiveDate, cycle: u32) -> NaiveDate {
    start
        .checked_add_months(Months::new(cycle + 1))
        .expect("a later month")
}

/// `amount × part / whole`, rounded half-up once (OQ14).
pub fn prorate(amount: i64, part: i64, whole: i64) -> i64 {
    (2 * amount * part + whole) / (2 * whole)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Payg,
    Scale,
}

impl Plan {
    pub fn other(self) -> Self {
        match self {
            Self::Payg => Self::Scale,
            Self::Scale => Self::Payg,
        }
    }
}

/// Token prices per 1M, in cents, identical on both plans (scenario-timelines §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Lite,
    Standard,
    Max,
}

impl Token {
    pub fn rate(self) -> i64 {
        match self {
            Self::Lite => 200,
            Self::Standard => 1_200,
            Self::Max => 6_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Pack,
    Bonus,
    Welcome,
    TopUp,
    Promo,
    Note,
    Overpay,
}

#[derive(Clone, Debug)]
pub struct Grant {
    pub kind: Kind,
    /// Subscription-scoped plan credit pays that subscription's usage only (R7, I8).
    pub sub: Option<usize>,
    pub paid: bool,
    pub face: i64,
    pub remaining: i64,
    pub effective: At,
    pub expires: Option<At>,
    pub ends: Option<At>,
    pub invoice: Option<usize>,
    /// Taken back by a payment reversal; a replacement payment reinstates it (OQ12).
    pub reversed: i64,
}

impl Grant {
    pub fn lapse(&self) -> Option<At> {
        match (self.expires, self.ends) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn spendable(&self, sub: usize, at: At) -> bool {
        self.remaining > 0
            && self.effective <= at
            && self.lapse().is_none_or(|lapse| lapse > at)
            && self.sub.is_none_or(|s| s == sub)
    }

    /// Usage is paid only by credit effective before the end of its line's dates (founder
    /// 2026-10-20, Orb).
    fn pays_usage_ending(&self, end: At) -> bool {
        self.effective < end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheduled {
    Change(Plan),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct Sub {
    pub plan: Plan,
    pub start: NaiveDate,
    pub period_start: NaiveDate,
    /// The next renewal, or the termination of the period still to invoice.
    pub period_end: NaiveDate,
    pub cycle: u32,
    pub active: bool,
    /// Its current period is still to be invoiced.
    pub open: bool,
    pub scheduled: Option<Scheduled>,
    pub changed: bool,
    /// The day an immediate change split the open period: its usage before then is billed on the
    /// old plan's lines, which end that day.
    pub split: Option<NaiveDate>,
    pub welcome_billed: bool,
    pub pack_invoice: Option<usize>,
    /// Settlements of the open period, by the start of the line they paid; its invoice takes
    /// them over (R10).
    pub settled: Vec<(At, usize, i64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NoteType {
    Balance,
    Refund,
}

#[derive(Clone, Copy, Debug)]
pub enum Sale {
    Pack {
        sub: usize,
        fee: i64,
        credit: i64,
        from: At,
        until: At,
    },
    TopUp(i64),
}

#[derive(Clone, Debug)]
pub struct Payment {
    pub amount: i64,
    pub provider: bool,
    pub reversed: bool,
    pub refunded: i64,
    /// Refunded on a credit note, which nets out of what the invoice collected (OQ12).
    pub note_refunded: i64,
    pub late_pack: bool,
}

#[derive(Clone, Debug)]
pub struct Invoice {
    pub sub: Option<usize>,
    pub subtotal: i64,
    pub promo: i64,
    pub applied: i64,
    pub total: i64,
    /// What the customer owes once credit applied, before any payment.
    pub owed: i64,
    /// Collected for the invoice: money lost to a dispute still paid it.
    pub collected: i64,
    /// Lost to disputes: the invoice stays paid, the merchant no longer holds it.
    pub lost: i64,
    pub void: bool,
    pub sale: Option<Sale>,
    pub welcome: Option<usize>,
    pub draws: Vec<(usize, i64)>,
    /// Its usage lines: the start of each and the usage it billed.
    pub usage_lines: Vec<(At, i64)>,
    pub minted: Vec<usize>,
    pub funded: bool,
    pub notes: Vec<(NoteType, i64)>,
    pub payments: Vec<Payment>,
}

impl Invoice {
    pub fn due(&self) -> i64 {
        (self.owed - self.collected).max(0)
    }

    /// What the merchant still holds of the invoice's money.
    pub fn in_hand(&self) -> i64 {
        self.collected - self.lost
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub sub: usize,
    pub at: At,
    pub amount: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Manual,
    Provider(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Late {
    WithinGrace,
    AfterGrace,
}

#[derive(Clone, Copy, Debug)]
pub struct LateUse {
    pub when: Late,
    pub token: Token,
    pub mtok: i64,
}

/// A concrete operation, resolved against the model's state.
#[derive(Clone, Debug)]
pub enum Action {
    Start {
        day: NaiveDate,
        plan: Plan,
    },
    Usage {
        sub: usize,
        at: At,
        dated: At,
        token: Token,
        mtok: i64,
    },
    Renew {
        at: At,
        subs: Vec<usize>,
    },
    TopUp {
        at: At,
        amount: i64,
        pay: bool,
    },
    Pay {
        at: At,
        invoice: usize,
        how: How,
    },
    Fail {
        at: At,
        invoice: usize,
    },
    /// The provider takes the payment back: a dispute (the invoice stays paid) or a failed
    /// collection (it reopens). Never more than the payment still holds.
    Chargeback {
        at: At,
        invoice: usize,
        payment: usize,
        disputed: bool,
    },
    RefundNote {
        at: At,
        invoice: usize,
        payment: usize,
        amount: i64,
    },
    RefundNoNote {
        at: At,
        invoice: usize,
        payment: usize,
        amount: i64,
    },
    Void {
        at: At,
        invoice: usize,
    },
    Redeem {
        at: At,
    },
    ChangeNow {
        sub: usize,
        day: NaiveDate,
        to: Plan,
        late: Option<LateUse>,
    },
    ChangeAtEnd {
        at: At,
        sub: usize,
        to: Plan,
    },
    CancelNow {
        sub: usize,
        day: NaiveDate,
        late: Option<LateUse>,
    },
    CancelAtEnd {
        at: At,
        sub: usize,
        end: NaiveDate,
    },
}

impl Action {
    pub fn at(&self) -> At {
        match self {
            Self::Start { day, .. } | Self::ChangeNow { day, .. } | Self::CancelNow { day, .. } => {
                midnight(*day)
            }
            Self::Usage { at, .. }
            | Self::Renew { at, .. }
            | Self::TopUp { at, .. }
            | Self::Pay { at, .. }
            | Self::Fail { at, .. }
            | Self::Chargeback { at, .. }
            | Self::RefundNote { at, .. }
            | Self::RefundNoNote { at, .. }
            | Self::Void { at, .. }
            | Self::Redeem { at }
            | Self::ChangeAtEnd { at, .. }
            | Self::CancelAtEnd { at, .. } => *at,
        }
    }

    /// The instant the action's last step happens, after which the next action may start.
    pub fn done(&self) -> At {
        match self {
            Self::Renew { at, .. } => *at + DOCUMENTS,
            Self::ChangeNow { day, .. } | Self::CancelNow { day, .. } => midnight(*day) + DOCUMENTS,
            other => other.at(),
        }
    }
}

/// The settlement grace with an entity grace of 0 h (OQ15); the runner sweeps one minute later.
pub const GRACE: Duration = Duration::minutes(31);
/// Backdated subscription documents are issued at noon of their date; the sweeps closing an
/// action run after them.
pub const NOON: Duration = Duration::hours(12);
pub const DOCUMENTS: Duration = Duration::hours(13);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub &'static str);

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub now: Option<At>,
    pub subs: Vec<Sub>,
    pub grants: Vec<Grant>,
    pub invoices: Vec<Invoice>,
    pub events: Vec<Event>,
    pub promo_redeemed: bool,
}

const FAR: At = DateTime::<Utc>::MAX_UTC;

impl Model {
    pub fn now(&self) -> At {
        self.now.expect("started")
    }

    pub fn x_remaining(&self, invoice: usize) -> i64 {
        self.grants
            .iter()
            .filter(|g| g.kind == Kind::Overpay && g.invoice == Some(invoice))
            .map(|g| g.remaining)
            .sum()
    }

    fn usage(&self, sub: usize, from: At, until: At) -> i64 {
        self.events
            .iter()
            .filter(|e| e.sub == sub && from <= e.at && e.at < until)
            .map(|e| e.amount)
            .sum()
    }

    /// The open period's usage lines as `[start, end)`: an immediate change splits them at its day.
    fn segments(&self, sub: usize, until: At) -> Vec<(At, At)> {
        let start = midnight(self.subs[sub].period_start);
        match self.subs[sub]
            .split
            .map(midnight)
            .filter(|&split| start < split && split < until)
        {
            Some(split) => vec![(start, split), (split, until)],
            None => vec![(start, until)],
        }
    }

    fn covered(&self, sub: usize, segment_start: At) -> i64 {
        self.subs[sub]
            .settled
            .iter()
            .filter(|(start, _, _)| *start == segment_start)
            .map(|(_, _, amount)| amount)
            .sum()
    }

    /// Draw order (R8): promotional before paid, subscription-scoped before customer-wide,
    /// soonest lapse first, then effective date and creation.
    fn rank(&self, g: usize) -> (bool, bool, At, At, usize) {
        let grant = &self.grants[g];
        (
            grant.paid,
            grant.sub.is_none(),
            grant.lapse().unwrap_or(FAR),
            grant.effective,
            g,
        )
    }

    fn grant(&mut self, grant: Grant) -> usize {
        self.grants.push(grant);
        self.grants.len() - 1
    }

    /// The usage of each line (`demands`, by its end) still uncovered when its invoice is issued at
    /// `at`, drawn in draw order: each grant pays the earliest lines it may pay first, which the
    /// fewest later grants can pay.
    fn draw(&mut self, invoice: usize, sub: usize, mut demands: Vec<(At, i64)>, at: At) {
        let mut order: Vec<usize> = (0..self.grants.len())
            .filter(|&g| self.grants[g].spendable(sub, at))
            .collect();
        order.sort_by_key(|&g| self.rank(g));
        for g in order {
            let mut took = 0;
            for (end, demand) in demands.iter_mut() {
                if !self.grants[g].pays_usage_ending(*end) {
                    continue;
                }
                let take = (self.grants[g].remaining - took).min(*demand);
                *demand -= take;
                took += take;
            }
            if took > 0 {
                self.grants[g].remaining -= took;
                self.invoices[invoice].draws.push((g, took));
            }
        }
    }

    /// The usage of the line `[start, end)` dated before `bound` its settlements left.
    fn owed(&self, s: usize, (start, end): (At, At), bound: At) -> i64 {
        (self.usage(s, start, bound.min(end)) - self.covered(s, start)).max(0)
    }

    fn settle_from(&mut self, s: usize, segment_start: At, mut owed: i64, payers: Vec<usize>) {
        for g in payers {
            let take = owed.min(self.grants[g].remaining);
            if take > 0 {
                self.grants[g].remaining -= take;
                self.subs[s].settled.push((segment_start, g, take));
                owed -= take;
            }
        }
    }

    /// Expiry settlement at `bound` (R10, D3): the usage dated before it of every open period a
    /// lapsing grant may pay is paid now, by the grants lapsing then first (promotional, then
    /// paid), then by every other grant spendable just before it, in draw order.
    fn settle_group(&mut self, bound: At, group: Vec<usize>) {
        let before = bound - Duration::microseconds(1);
        for s in 0..self.subs.len() {
            if !self.subs[s].open {
                continue;
            }
            for segment in self.segments(s, midnight(self.subs[s].period_end)) {
                let mut lapsing: Vec<usize> = group
                    .iter()
                    .copied()
                    .filter(|&g| {
                        let grant = &self.grants[g];
                        grant.remaining > 0
                            && grant.effective < bound
                            && grant.pays_usage_ending(segment.1)
                            && grant.sub.is_none_or(|own| own == s)
                    })
                    .collect();
                if lapsing.is_empty() {
                    continue;
                }
                lapsing.sort_by_key(|&g| self.rank(g));
                let mut others: Vec<usize> = (0..self.grants.len())
                    .filter(|g| {
                        !group.contains(g)
                            && self.grants[*g].spendable(s, before)
                            && self.grants[*g].pays_usage_ending(segment.1)
                    })
                    .collect();
                others.sort_by_key(|&g| self.rank(g));
                lapsing.extend(others);
                let owed = self.owed(s, segment, bound);
                self.settle_from(s, segment.0, owed, lapsing);
            }
        }
    }

    /// A plan change or cancellation settles the usage to it from the grants it ends only.
    fn settle_ending(&mut self, s: usize, bound: At, mut ending: Vec<usize>) {
        ending.sort_by_key(|&g| self.rank(g));
        let Some(segment) = self
            .segments(s, midnight(self.subs[s].period_end))
            .into_iter()
            .find(|(start, end)| *start < bound && bound <= *end)
        else {
            return;
        };
        let owed = self.owed(s, segment, bound);
        self.settle_from(s, segment.0, owed, ending);
    }

    /// The expiry sweep at `as_of`: every lapse due by then settles, then what is left lapses.
    pub fn sweep(&mut self, as_of: At) {
        loop {
            let due: Vec<usize> = (0..self.grants.len())
                .filter(|&g| {
                    self.grants[g].remaining > 0
                        && self.grants[g].lapse().is_some_and(|lapse| lapse <= as_of)
                })
                .collect();
            let Some(bound) = due.iter().filter_map(|&g| self.grants[g].lapse()).min() else {
                return;
            };
            let group: Vec<usize> = due
                .into_iter()
                .filter(|&g| self.grants[g].lapse() == Some(bound))
                .collect();
            self.settle_group(bound, group.clone());
            for g in group {
                self.grants[g].remaining = 0;
            }
        }
    }

    /// The sweeps the runner runs, one per lapse, a grace after it, before an action at `until`.
    pub fn sweeps_due(&self, until: At) -> Vec<At> {
        let now = self.now();
        let mut instants: Vec<At> = self
            .grants
            .iter()
            .filter(|g| g.remaining > 0)
            .filter_map(Grant::lapse)
            .map(|lapse| (lapse + GRACE).max(now))
            .filter(|&at| at <= until)
            .collect();
        instants.sort();
        instants.dedup();
        instants
    }

    fn new_invoice(&mut self, sub: Option<usize>) -> usize {
        self.invoices.push(Invoice {
            sub,
            subtotal: 0,
            promo: 0,
            applied: 0,
            total: 0,
            owed: 0,
            collected: 0,
            lost: 0,
            void: false,
            sale: None,
            welcome: None,
            draws: vec![],
            usage_lines: vec![],
            minted: vec![],
            funded: false,
            notes: vec![],
            payments: vec![],
        });
        self.invoices.len() - 1
    }

    /// Fixes the invoice's figures from its draws (R1): promotional before tax, paid after.
    fn close_invoice(&mut self, invoice: usize, subtotal: i64) {
        let (promo, applied) =
            self.invoices[invoice]
                .draws
                .iter()
                .fold((0, 0), |(promo, applied), &(g, amount)| {
                    if self.grants[g].paid {
                        (promo, applied + amount)
                    } else {
                        (promo + amount, applied)
                    }
                });
        let inv = &mut self.invoices[invoice];
        inv.subtotal = subtotal;
        inv.promo = promo;
        inv.applied = applied;
        inv.total = subtotal - promo;
        inv.owed = inv.total - applied;
    }

    /// The welcome (`once`) credit of the first invoice of the subscription billing Scale,
    /// expiring 90 days after the start of the line that bills it (OQ6, OQ9, OQ19).
    fn welcome(&mut self, invoice: usize, sub: usize, line_start: At, issued: At) {
        if self.subs[sub].welcome_billed {
            return;
        }
        self.subs[sub].welcome_billed = true;
        let g = self.grant(Grant {
            kind: Kind::Welcome,
            sub: Some(sub),
            paid: false,
            face: WELCOME,
            remaining: WELCOME,
            effective: issued,
            expires: Some(line_start + Duration::days(WELCOME_DAYS)),
            ends: None,
            invoice: Some(invoice),
            reversed: 0,
        });
        self.invoices[invoice].welcome = Some(g);
    }

    /// Bills the usage of `sub` in [from, until) issued at `issued`, taking its settlements
    /// over and drawing the rest (R10, R8); with an advance pack when it renews on Scale.
    fn bill(
        &mut self,
        sub: usize,
        from: At,
        until: At,
        issued: At,
        pack: Option<(i64, i64, At)>,
    ) -> Option<usize> {
        let usage = self.usage(sub, from, until);
        let demands: Vec<(At, i64)> = self
            .segments(sub, until)
            .into_iter()
            .map(|(start, end)| {
                (
                    end,
                    (self.usage(sub, start, end) - self.covered(sub, start)).max(0),
                )
            })
            .collect();
        let settled = std::mem::take(&mut self.subs[sub].settled);
        if usage == 0 && pack.is_none() && settled.is_empty() {
            return None;
        }
        let invoice = self.new_invoice(Some(sub));
        self.invoices[invoice].usage_lines = self
            .segments(sub, until)
            .into_iter()
            .map(|(start, end)| (start, self.usage(sub, start, end)))
            .collect();
        self.invoices[invoice].draws = settled
            .into_iter()
            .map(|(_, g, amount)| (g, amount))
            .collect();
        self.draw(invoice, sub, demands, issued);
        let fee = pack.map_or(0, |(fee, _, _)| fee);
        self.close_invoice(invoice, usage + fee);
        if let Some((fee, credit, pack_until)) = pack {
            self.invoices[invoice].sale = Some(Sale::Pack {
                sub,
                fee,
                credit,
                from: until,
                until: pack_until,
            });
            self.subs[sub].pack_invoice = Some(invoice);
        }
        Some(invoice)
    }

    pub fn apply(&mut self, action: &Action) -> Result<(), Refused> {
        let result = match action {
            Action::Start { day, plan } => {
                self.start(*day, *plan);
                Ok(())
            }
            Action::Usage {
                sub,
                dated,
                token,
                mtok,
                ..
            } => {
                self.events.push(Event {
                    sub: *sub,
                    at: *dated,
                    amount: token.rate() * mtok,
                });
                Ok(())
            }
            Action::Renew { at, subs } => {
                self.renew(*at, subs);
                Ok(())
            }
            Action::TopUp { at, amount, pay } => {
                let invoice = self.new_invoice(None);
                self.invoices[invoice].sale = Some(Sale::TopUp(*amount));
                self.close_invoice(invoice, *amount);
                if *pay {
                    self.pay(*at, invoice, How::Manual)
                } else {
                    Ok(())
                }
            }
            Action::Pay { at, invoice, how } => self.pay(*at, *invoice, *how),
            Action::Fail { invoice, .. } => {
                if self.invoices[*invoice].due() > 0 {
                    Ok(())
                } else {
                    Err(Refused("nothing due"))
                }
            }
            Action::Chargeback {
                at,
                invoice,
                payment,
                disputed,
            } => {
                let inv = &mut self.invoices[*invoice];
                let payment = &mut inv.payments[*payment];
                payment.reversed = true;
                let held = (payment.amount - payment.refunded - payment.note_refunded).max(0);
                if *disputed {
                    inv.lost += held;
                } else {
                    inv.collected -= held;
                }
                self.fund(*invoice, *at);
                self.overpay(*invoice, *at);
                Ok(())
            }
            Action::RefundNote {
                invoice,
                payment,
                amount,
                ..
            } => self.refund_note(*invoice, *payment, *amount),
            Action::RefundNoNote {
                at,
                invoice,
                payment,
                amount,
            } => {
                // A refund without a note gives back at most the unspent overpayment (OQ12, Q1).
                if *amount > self.x_remaining(*invoice) {
                    return Err(Refused("refund above the unspent overpayment"));
                }
                let inv = &mut self.invoices[*invoice];
                inv.payments[*payment].refunded += amount;
                inv.collected -= amount;
                self.overpay(*invoice, *at);
                Ok(())
            }
            Action::Void { invoice, .. } => self.void(*invoice),
            Action::Redeem { at } => self.redeem(*at),
            Action::ChangeNow { sub, day, to, late } => {
                self.change_now(*sub, *day, *to, *late);
                Ok(())
            }
            Action::ChangeAtEnd { sub, to, .. } => {
                self.subs[*sub].scheduled = Some(Scheduled::Change(*to));
                Ok(())
            }
            Action::CancelNow { sub, day, late } => {
                self.cancel_now(*sub, *day, *late);
                Ok(())
            }
            Action::CancelAtEnd { sub, .. } => {
                self.subs[*sub].scheduled = Some(Scheduled::Cancel);
                Ok(())
            }
        };
        let done = action.done();
        if !matches!(
            action,
            Action::Renew { .. } | Action::ChangeNow { .. } | Action::CancelNow { .. }
        ) {
            self.sweep(done);
        }
        self.now = Some(done);
        result
    }

    fn start(&mut self, day: NaiveDate, plan: Plan) {
        let sub = self.subs.len();
        self.subs.push(Sub {
            plan,
            start: day,
            period_start: day,
            period_end: period_end(day, 0),
            cycle: 0,
            active: true,
            open: true,
            scheduled: None,
            changed: false,
            split: None,
            welcome_billed: false,
            pack_invoice: None,
            settled: vec![],
        });
        if plan == Plan::Scale {
            let at = midnight(day);
            let invoice = self.new_invoice(Some(sub));
            self.invoices[invoice].sale = Some(Sale::Pack {
                sub,
                fee: PACK_FEE,
                credit: PACK_CREDIT,
                from: at,
                until: midnight(period_end(day, 0)),
            });
            self.close_invoice(invoice, PACK_FEE);
            self.subs[sub].pack_invoice = Some(invoice);
            self.welcome(invoice, sub, at, at);
        }
    }

    /// Renewals at `at` (R10, R15, R18): the grants lapsing then pay their period first, the
    /// invoice takes that over and draws the rest; a cancellation's final invoice is issued at
    /// noon, after every renewal of the bound.
    fn renew(&mut self, at: At, subs: &[usize]) {
        let mut finals = vec![];
        for &s in subs {
            let from = midnight(self.subs[s].period_start);
            if self.subs[s].scheduled == Some(Scheduled::Cancel) {
                for grant in self.grants.iter_mut().filter(|g| g.sub == Some(s)) {
                    grant.ends = Some(grant.ends.map_or(at, |ends| ends.min(at)));
                }
                self.subs[s].active = false;
                finals.push((s, from));
                continue;
            }
            self.lapse_at_bound(s, at);
            if let Some(Scheduled::Change(to)) = self.subs[s].scheduled {
                self.subs[s].plan = to;
            }
            let end = period_end(self.subs[s].start, self.subs[s].cycle + 1);
            let pack = (self.subs[s].plan == Plan::Scale).then_some((
                PACK_FEE,
                PACK_CREDIT,
                midnight(end),
            ));
            let invoice = self.bill(s, from, at, at, pack);
            if let Some(invoice) = invoice
                && pack.is_some()
            {
                self.welcome(invoice, s, at, at);
            }
            let sub = &mut self.subs[s];
            sub.period_start = at.date_naive();
            sub.period_end = end;
            sub.cycle += 1;
            sub.scheduled = None;
            sub.changed = false;
            sub.split = None;
            if pack.is_none() {
                sub.pack_invoice = None;
            }
        }
        for (s, from) in finals {
            self.lapse_at_bound(s, at);
            self.bill(s, from, at, at + NOON, None);
            self.subs[s].open = false;
        }
        self.sweep(at + DOCUMENTS);
    }

    fn lapse_at_bound(&mut self, sub: usize, bound: At) {
        let group: Vec<usize> = (0..self.grants.len())
            .filter(|&g| {
                let grant = &self.grants[g];
                grant.remaining > 0
                    && grant.lapse() == Some(bound)
                    && grant.sub.is_none_or(|s| s == sub)
            })
            .collect();
        self.settle_group(bound, group);
    }

    /// Payment of what is due (manual) or of a provider amount, possibly above it (OQ12).
    fn pay(&mut self, at: At, invoice: usize, how: How) -> Result<(), Refused> {
        let due = self.invoices[invoice].due();
        if self.invoices[invoice].void || due == 0 {
            return Err(Refused("nothing due"));
        }
        let (amount, provider) = match how {
            How::Manual => (due, false),
            How::Provider(extra) => (due + extra, true),
        };
        let inv = &mut self.invoices[invoice];
        inv.payments.push(Payment {
            amount,
            provider,
            reversed: false,
            refunded: 0,
            note_refunded: 0,
            late_pack: false,
        });
        inv.collected += amount;
        // The excess is kept before the credit the invoice sold is minted (as built).
        self.overpay(invoice, at);
        let late = self.fund(invoice, at);
        if late {
            self.invoices[invoice]
                .payments
                .last_mut()
                .expect("the payment")
                .late_pack = true;
            self.credit_back_late_pack(invoice, at);
        }
        Ok(())
    }

    /// Credit sold by an invoice is minted when it is paid, at the payment (R12); a reversal
    /// takes back what is unspent and a replacement payment reinstates the same grants (OQ12).
    /// Returns whether the payment funded a period pack after its period (OQ11).
    fn fund(&mut self, invoice: usize, at: At) -> bool {
        let inv = &self.invoices[invoice];
        let Some(sale) = inv.sale else {
            return false;
        };
        let paid = inv.in_hand() >= inv.owed;
        if paid && !inv.funded {
            self.invoices[invoice].funded = true;
            if !self.invoices[invoice].minted.is_empty() {
                for g in self.invoices[invoice].minted.clone() {
                    let grant = &mut self.grants[g];
                    grant.remaining += grant.reversed;
                    grant.reversed = 0;
                }
                return false;
            }
            return self.mint(invoice, sale, at);
        }
        if !paid && inv.funded {
            self.invoices[invoice].funded = false;
            for g in self.invoices[invoice].minted.clone() {
                let grant = &mut self.grants[g];
                grant.reversed += grant.remaining;
                grant.remaining = 0;
            }
        }
        false
    }

    fn mint(&mut self, invoice: usize, sale: Sale, at: At) -> bool {
        match sale {
            Sale::TopUp(amount) => {
                let g = self.grant(Grant {
                    kind: Kind::TopUp,
                    sub: None,
                    paid: true,
                    face: amount,
                    remaining: amount,
                    effective: at,
                    expires: None,
                    ends: None,
                    invoice: Some(invoice),
                    reversed: 0,
                });
                self.invoices[invoice].minted.push(g);
                false
            }
            Sale::Pack {
                sub,
                fee,
                credit,
                until,
                ..
            } => {
                // A discounted pack is a paid grant at cost plus a bonus (D 2026-10-09).
                let ends = None;
                let pack = self.grant(Grant {
                    kind: Kind::Pack,
                    sub: Some(sub),
                    paid: true,
                    face: fee,
                    remaining: fee,
                    effective: at,
                    expires: Some(until),
                    ends,
                    invoice: Some(invoice),
                    reversed: 0,
                });
                self.invoices[invoice].minted.push(pack);
                if credit > fee {
                    let bonus = self.grant(Grant {
                        kind: Kind::Bonus,
                        sub: Some(sub),
                        paid: false,
                        face: credit - fee,
                        remaining: credit - fee,
                        effective: at,
                        expires: Some(until),
                        ends,
                        invoice: Some(invoice),
                        reversed: 0,
                    });
                    self.invoices[invoice].minted.push(bonus);
                }
                at >= until
            }
        }
    }

    /// OQ11: a pack paid after its period was invoiced gets back what it would have paid on time,
    /// so late pays what on time does: its period's invoiced usage drawn at its expiry by its bonus
    /// then paid credit at face (beside credit lapsing then, up to what that drew), up to its fee,
    /// by a note to balance on its line. Its paid credit and bonus are removed; no usage, no note.
    fn credit_back_late_pack(&mut self, invoice: usize, at: At) {
        let Some(Sale::Pack {
            sub,
            fee,
            from,
            until,
            ..
        }) = self.invoices[invoice].sale
        else {
            return;
        };
        let own = self.invoices[invoice].minted.clone();
        let in_period = |start: &At| from <= *start && *start < until;
        let billed: Vec<usize> = (0..self.invoices.len())
            .filter(|&i| {
                let inv = &self.invoices[i];
                !inv.void
                    && inv.sub == Some(sub)
                    && inv.usage_lines.iter().any(|(s, _)| in_period(s))
            })
            .collect();
        let mut usage: i64 = billed
            .iter()
            .flat_map(|&i| &self.invoices[i].usage_lines)
            .filter(|(start, _)| in_period(start))
            .map(|(_, amount)| amount)
            .sum();
        let mut payers: Vec<(usize, i64)> = own.iter().map(|&g| (g, self.grants[g].face)).collect();
        for g in (0..self.grants.len()).filter(|g| !own.contains(g)) {
            let grant = &self.grants[g];
            if grant.lapse() == Some(until) && grant.sub.is_none_or(|s| s == sub) {
                let drew = billed
                    .iter()
                    .flat_map(|&i| &self.invoices[i].draws)
                    .filter(|(drawer, _)| *drawer == g)
                    .map(|(_, amount)| amount)
                    .sum();
                payers.push((g, drew));
            }
        }
        payers.sort_by_key(|&(g, _)| self.rank(g));
        let mut owed = 0;
        for (g, budget) in payers {
            let take = usage.min(budget);
            usage -= take;
            if own.contains(&g) {
                owed += take;
            }
        }
        for &g in &own {
            self.grants[g].remaining = 0;
        }
        let credit = owed.min(fee);
        if credit > 0 {
            self.invoices[invoice]
                .notes
                .push((NoteType::Balance, -credit));
            self.balance_credit(credit, at);
        }
    }

    fn balance_credit(&mut self, amount: i64, at: At) {
        self.grant(Grant {
            kind: Kind::Note,
            sub: None,
            paid: true,
            face: amount,
            remaining: amount,
            effective: at,
            expires: None,
            ends: None,
            invoice: None,
            reversed: 0,
        });
    }

    /// OQ12: every invoice keeps max(0, collected − due) as OVERPAYMENT credit, recomputed on
    /// every payment, reversal and refund; reversed credit is reinstated before any new grant.
    fn overpay(&mut self, invoice: usize, at: At) {
        let inv = &self.invoices[invoice];
        let target = (inv.in_hand() - inv.owed).max(0);
        let mine: Vec<usize> = (0..self.grants.len())
            .filter(|&g| {
                self.grants[g].kind == Kind::Overpay && self.grants[g].invoice == Some(invoice)
            })
            .collect();
        // What the invoice keeps counts the excess already spent: it is never granted twice.
        let kept: i64 = mine
            .iter()
            .map(|&g| self.grants[g].face - self.grants[g].reversed)
            .sum();
        if target > kept {
            let mut more = target - kept;
            for &g in &mine {
                let back = self.grants[g].reversed.min(more);
                self.grants[g].reversed -= back;
                self.grants[g].remaining += back;
                more -= back;
            }
            if more > 0 {
                self.grant(Grant {
                    kind: Kind::Overpay,
                    sub: None,
                    paid: true,
                    face: more,
                    remaining: more,
                    effective: at,
                    expires: None,
                    ends: None,
                    invoice: Some(invoice),
                    reversed: 0,
                });
            }
        } else if target < kept {
            let mut less = kept - target;
            for &g in mine.iter().rev() {
                let take = self.grants[g].remaining.min(less);
                self.grants[g].remaining -= take;
                self.grants[g].reversed += take;
                less -= take;
            }
        }
    }

    /// A refund note on purchased credit is capped at what is still unspent, and so is its cash
    /// (D scenario tests round); the note and its refund cancel out for OQ12.
    fn refund_note(&mut self, invoice: usize, payment: usize, amount: i64) -> Result<(), Refused> {
        let g = self.invoices[invoice].minted[0];
        let capped = amount.min(self.grants[g].remaining);
        if capped <= 0 {
            return Err(Refused("no unspent purchased credit"));
        }
        self.grants[g].remaining -= capped;
        let inv = &mut self.invoices[invoice];
        inv.notes.push((NoteType::Refund, -capped));
        inv.payments[payment].note_refunded += capped;
        Ok(())
    }

    /// A void returns every draw to its grant; a granted `once` credit is taken back, and a
    /// void that took it back whole leaves the once credit unbilled (D settlement round).
    fn void(&mut self, invoice: usize) -> Result<(), Refused> {
        if let Some(w) = self.invoices[invoice].welcome
            && self
                .invoices
                .iter()
                .any(|other| !other.void && other.draws.iter().any(|(g, _)| *g == w))
        {
            return Err(Refused("its granted credit paid other invoices"));
        }
        for (g, amount) in self.invoices[invoice].draws.clone() {
            self.grants[g].remaining += amount;
        }
        self.follow_given_back();
        if let Some(w) = self.invoices[invoice].welcome {
            let whole = self.grants[w].remaining == self.grants[w].face;
            self.grants[w].remaining = 0;
            if whole && let Some(sub) = self.invoices[invoice].sub {
                self.subs[sub].welcome_billed = false;
            }
        }
        let inv = &mut self.invoices[invoice];
        inv.void = true;
        inv.notes.push((NoteType::Balance, -inv.total));
        Ok(())
    }

    /// Credit given back to a grant whose money was reversed is taken back at once (OQ12).
    fn follow_given_back(&mut self) {
        for invoice in 0..self.invoices.len() {
            if self.invoices[invoice].sale.is_some() && !self.invoices[invoice].funded {
                for g in self.invoices[invoice].minted.clone() {
                    let grant = &mut self.grants[g];
                    grant.reversed += grant.remaining;
                    grant.remaining = 0;
                }
            }
            if self
                .grants
                .iter()
                .any(|g| g.kind == Kind::Overpay && g.invoice == Some(invoice))
            {
                let at = self.now();
                self.overpay(invoice, at);
            }
        }
    }

    /// A promotional-credit code: once per customer, customer-wide, expiring 25 days after the
    /// redemption (R19, OQ20).
    fn redeem(&mut self, at: At) -> Result<(), Refused> {
        if self.promo_redeemed {
            return Err(Refused("already redeemed"));
        }
        self.promo_redeemed = true;
        self.grant(Grant {
            kind: Kind::Promo,
            sub: None,
            paid: false,
            face: PROMO,
            remaining: PROMO,
            effective: at,
            expires: Some(at + Duration::days(PROMO_DAYS)),
            ends: None,
            invoice: None,
            reversed: 0,
        });
        Ok(())
    }

    fn pack_grants(&self, sub: usize) -> (Option<usize>, Option<usize>, Option<(usize, i64)>) {
        let Some(invoice) = self.subs[sub].pack_invoice else {
            return (None, None, None);
        };
        let Some(Sale::Pack { fee, .. }) = self.invoices[invoice].sale else {
            return (None, None, None);
        };
        let minted = &self.invoices[invoice].minted;
        let find = |kind| {
            minted
                .iter()
                .copied()
                .find(|&g| self.grants[g].kind == kind)
        };
        (find(Kind::Pack), find(Kind::Bonus), Some((invoice, fee)))
    }

    /// The old pack settles the usage to `day` (ending grants only), then its unused fee comes
    /// back capped by its unused paid credit, on a credit note to balance (OQ1, R15, R18).
    fn end_pack(&mut self, sub: usize, day: NaiveDate) {
        let (pack, bonus, sale) = self.pack_grants(sub);
        let bound = midnight(day);
        let ending: Vec<usize> = [bonus, pack].into_iter().flatten().collect();
        self.settle_ending(sub, bound, ending.clone());
        if let (Some(p), Some((invoice, fee))) = (pack, sale) {
            let period = (self.subs[sub].period_end - self.subs[sub].period_start).num_days();
            let left = (self.subs[sub].period_end - day).num_days();
            let grant = &self.grants[p];
            let credit = prorate(fee, left, period).min(prorate(fee, grant.remaining, grant.face));
            if credit > 0 {
                self.grants[p].remaining -= credit;
                self.invoices[invoice]
                    .notes
                    .push((NoteType::Balance, -credit));
                self.balance_credit(credit, bound);
            }
        }
        for g in ending {
            self.grants[g].ends = Some(bound);
        }
    }

    fn late_event(&mut self, sub: usize, day: NaiveDate, late: LateUse) {
        self.events.push(Event {
            sub,
            at: midnight(day) - Duration::hours(1),
            amount: late.token.rate() * late.mtok,
        });
    }

    /// An immediate plan change (R15): the old pack ends at the change, the new plan's advance
    /// fees are prorated by whole days on an adjustment invoice issued at noon.
    fn change_now(&mut self, sub: usize, day: NaiveDate, to: Plan, late: Option<LateUse>) {
        self.subs[sub].split = Some(day);
        if self.subs[sub].plan == Plan::Scale {
            self.end_pack(sub, day);
        }
        self.subs[sub].plan = to;
        self.subs[sub].changed = true;
        if to == Plan::Scale {
            let s = &self.subs[sub];
            let (period, left) = (
                (s.period_end - s.period_start).num_days(),
                (s.period_end - day).num_days(),
            );
            let until = midnight(s.period_end);
            let issued = midnight(day) + NOON;
            let invoice = self.new_invoice(Some(sub));
            let fee = prorate(PACK_FEE, left, period);
            self.invoices[invoice].sale = Some(Sale::Pack {
                sub,
                fee,
                credit: prorate(PACK_CREDIT, left, period),
                from: midnight(day),
                until,
            });
            self.close_invoice(invoice, fee);
            self.subs[sub].pack_invoice = Some(invoice);
            self.welcome(invoice, sub, midnight(day), issued);
        } else {
            self.subs[sub].pack_invoice = None;
        }
        self.after_ending(sub, day, late);
    }

    /// An immediate cancellation (R18, OQ1): the pack settles and its fee comes back, every grant
    /// of the subscription ends then; the final invoice is issued at noon.
    fn cancel_now(&mut self, sub: usize, day: NaiveDate, late: Option<LateUse>) {
        if self.subs[sub].plan == Plan::Scale {
            self.end_pack(sub, day);
        }
        let bound = midnight(day);
        for grant in self.grants.iter_mut().filter(|g| g.sub == Some(sub)) {
            grant.ends = Some(grant.ends.map_or(bound, |ends| ends.min(bound)));
        }
        self.subs[sub].active = false;
        self.subs[sub].scheduled = None;
        let from = midnight(self.subs[sub].period_start);
        self.subs[sub].period_end = day;
        self.lapse_at_bound(sub, bound);
        self.bill(sub, from, bound, bound + NOON, None);
        self.subs[sub].open = false;
        self.after_ending(sub, day, late);
    }

    /// A late event within the settlement grace is settled by the ending grants, one ingested
    /// after it is billed beyond credit (OQ15); after an issued final invoice neither is billed.
    fn after_ending(&mut self, sub: usize, day: NaiveDate, late: Option<LateUse>) {
        let bound = midnight(day);
        if let Some(late) = late.filter(|l| l.when == Late::WithinGrace) {
            self.late_event(sub, day, late);
        }
        self.sweep(bound + GRACE);
        if let Some(late) = late.filter(|l| l.when == Late::AfterGrace) {
            self.late_event(sub, day, late);
        }
        self.sweep(bound + DOCUMENTS);
    }
}

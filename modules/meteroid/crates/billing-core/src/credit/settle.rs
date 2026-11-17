//! Credit settlement of one document: promotional credit off the charges after
//! discount and before tax (R-021, R1), the lines it moves taxed again by their rates, then paid
//! credit on what is left with tax. Drafts, previews and forecasts call [`settle`]; finalize runs
//! the same steps with the ledger's draws in between.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use common_domain::ids::CreditGrantId;

use crate::allocation::allocate_covered;
use crate::credit::SettleError;
use crate::credit::allocation::{Allocation, Demand, Grant, Overflow, Pool, Request, allocate};
use crate::credit::owed::{Settled, UsageCharge, UsageOwed, after_settlements, assign_settled};
use crate::document::{Document, Header, Line, LineKind, Rate, Rates, apply_rates};
use crate::totals::{Totals, amount_due};

fn demand_of(line: &Line) -> Option<Demand> {
    let (usage, dated_before) = match line.kind {
        LineKind::Credit => return None,
        LineKind::Fee => (false, None),
        LineKind::Usage { dated_before } => (true, Some(dated_before)),
    };
    Some(Demand {
        subscription: line.subscription,
        product: line.product,
        usage,
        amount_minor: 0,
        dated_before,
    })
}

fn same_charge(left: &Demand, right: &Demand) -> bool {
    left.usage == right.usage
        && left.product == right.product
        && left.subscription == right.subscription
        && left.dated_before == right.dated_before
}

/// A negative line (an untaxed proration credit) still nets what the charges beside it may take:
/// credit drawn on a document owing nothing would come back as paid.
fn netting_base(line: &Line, base: i64) -> i64 {
    if line.figures.subtotal < 0 {
        base.min(line.figures.subtotal)
    } else {
        base
    }
}

/// A taxed line's taxable amount before its promotional credit, in the figures it was quoted in.
pub fn taxed_base(line: &Line, tax_inclusive: bool) -> i64 {
    let share = line.promotional.unwrap_or(0);
    if tax_inclusive {
        line.taxable + line.tax + share
    } else {
        line.taxable + share
    }
}

/// What promotional credit may take off `lines` before tax, each contributing `base`, and the
/// demand each line falls under: one per subscription, product and kind of charge (R4, D4).
pub fn pretax_request(
    lines: &[Line],
    base: impl Fn(usize, &Line) -> i64,
) -> Result<(Request, Vec<Option<usize>>), Overflow> {
    let mut demands: Vec<Demand> = Vec::new();
    let mut of_line = Vec::with_capacity(lines.len());
    let mut requested = 0_i64;
    for (index, line) in lines.iter().enumerate() {
        let Some(key) = demand_of(line) else {
            of_line.push(None);
            continue;
        };
        let amount = base(index, line);
        requested = requested.checked_add(amount).ok_or(Overflow)?;
        let index = match demands.iter().position(|demand| same_charge(demand, &key)) {
            Some(index) => index,
            None => {
                demands.push(key);
                demands.len() - 1
            }
        };
        demands[index].amount_minor = demands[index]
            .amount_minor
            .checked_add(amount)
            .ok_or(Overflow)?;
        of_line.push(Some(index));
    }
    for demand in &mut demands {
        demand.amount_minor = demand.amount_minor.max(0);
    }
    Ok((
        Request {
            requested_minor: requested.max(0),
            demands,
        },
        of_line,
    ))
}

/// [`pretax_request`] for an invoice's own lines, its negative lines netting the charges.
pub fn invoice_request(
    lines: &[Line],
    base: impl Fn(usize, &Line) -> i64,
) -> Result<(Request, Vec<Option<usize>>), Overflow> {
    pretax_request(lines, |index, line| netting_base(line, base(index, line)))
}

/// One demand per line, each at most the share `shown` (by line id) gave it: once cash was taken
/// against a total, no line ends up costing less than it did, so the cash never overpays.
pub fn pretax_request_within(
    lines: &[Line],
    tax_inclusive: bool,
    shown: &HashMap<String, i64>,
) -> Result<(Request, Vec<Option<usize>>), Overflow> {
    let mut demands = Vec::new();
    let mut of_line = Vec::with_capacity(lines.len());
    let mut net = 0_i64;
    for line in lines {
        let Some(demand) = demand_of(line) else {
            of_line.push(None);
            continue;
        };
        let base = taxed_base(line, tax_inclusive);
        net = net.checked_add(netting_base(line, base)).ok_or(Overflow)?;
        let cap = shown.get(line.id.as_str()).copied().unwrap_or(0);
        demands.push(Demand {
            amount_minor: base.min(cap).max(0),
            ..demand
        });
        of_line.push(Some(demands.len() - 1));
    }
    let requested_minor = demands
        .iter()
        .try_fold(0_i64, |sum, demand| sum.checked_add(demand.amount_minor))
        .ok_or(Overflow)?
        .min(net.max(0));
    Ok((
        Request {
            requested_minor,
            demands,
        },
        of_line,
    ))
}

/// Each line's share of what `allocation` covers of its demand, pro rata to its `base` (R-073).
pub fn line_shares(
    lines: &[Line],
    of_line: &[Option<usize>],
    allocation: &Allocation,
    base: impl Fn(usize, &Line) -> i64,
) -> Vec<i64> {
    let mut shares = vec![0_i64; lines.len()];
    for (demand, covered) in allocation.covered.iter().enumerate() {
        let members = of_line
            .iter()
            .enumerate()
            .filter(|(_, of)| **of == Some(demand))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let weights = members
            .iter()
            .map(|index| base(*index, &lines[*index]).max(0))
            .collect::<Vec<_>>();
        for (index, share) in members
            .into_iter()
            .zip(allocate_covered(&weights, *covered))
        {
            shares[index] = share;
        }
    }
    shares
}

/// The usage charges of `request`, each line at its `base` and the rates it was taxed with: paid
/// credit that settled usage is placed on them tax included.
pub fn usage_charges(
    lines: &[Line],
    of_line: &[Option<usize>],
    request: &Request,
    tax_inclusive: bool,
    base: impl Fn(usize, &Line) -> i64,
) -> Vec<UsageCharge> {
    lines
        .iter()
        .zip(of_line)
        .enumerate()
        .filter_map(|(index, (line, demand))| {
            let demand = (*demand)?;
            request.demands.get(demand)?.usage.then(|| UsageCharge {
                demand,
                owed: UsageOwed {
                    product: line.product,
                    base_minor: base(index, line).max(0),
                    tax_inclusive,
                    taxes: line
                        .taxes
                        .iter()
                        .map(|tax| Rate {
                            rate: tax.rate,
                            name: tax.name.clone(),
                        })
                        .collect(),
                },
            })
        })
        .collect()
}

/// The unpaid part of a document paid credit may cover, and per subscription, product and kind of
/// charge what its lines still cost, tax included; no credit pays a credit purchase (R1, R4).
pub fn paid_request(
    lines: &[Line],
    total: i64,
    already_paid: i64,
) -> Result<(Request, Vec<Option<usize>>), Overflow> {
    let credit_purchase = lines
        .iter()
        .filter(|line| line.kind == LineKind::Credit)
        .try_fold(0_i64, |sum, line| sum.checked_add(line.total.max(0)))
        .ok_or(Overflow)?;
    let mut demands: Vec<Demand> = Vec::new();
    let mut of_line = Vec::with_capacity(lines.len());
    for line in lines {
        let Some(key) = demand_of(line) else {
            of_line.push(None);
            continue;
        };
        match demands.iter().position(|demand| same_charge(demand, &key)) {
            Some(index) => {
                demands[index].amount_minor = demands[index]
                    .amount_minor
                    .checked_add(line.total)
                    .ok_or(Overflow)?;
                of_line.push(Some(index));
            }
            None => {
                demands.push(Demand {
                    amount_minor: line.total,
                    ..key
                });
                of_line.push(Some(demands.len() - 1));
            }
        }
    }
    for demand in &mut demands {
        demand.amount_minor = demand.amount_minor.max(0);
    }
    Ok((
        Request {
            requested_minor: total
                .saturating_sub(already_paid.max(0))
                .min(total.saturating_sub(credit_purchase))
                .max(0),
            demands,
        },
        of_line,
    ))
}

/// `request`, asking at most `cap`.
pub fn capped(mut request: Request, cap: i64) -> Request {
    request.requested_minor = request.requested_minor.min(cap).max(0);
    request
}

/// Per grant, what it paid of each line (by id), and of no line under `None`: each demand's part
/// spread over its lines by `base` (R-073).
pub fn line_allocation(
    lines: &[Line],
    of_line: &[Option<usize>],
    request: &Request,
    allocation: &Allocation,
    base: impl Fn(usize, &Line) -> i64,
) -> Vec<(CreditGrantId, Option<String>, i64)> {
    let mut out = Vec::new();
    for (grant_id, columns) in &allocation.placed {
        let mut placed: BTreeMap<Option<String>, i64> = BTreeMap::new();
        for (demand, amount) in columns.iter().enumerate() {
            if *amount <= 0 {
                continue;
            }
            if demand >= request.demands.len() {
                *placed.entry(None).or_default() += amount;
                continue;
            }
            let members: Vec<usize> = of_line
                .iter()
                .enumerate()
                .filter(|(_, of)| **of == Some(demand))
                .map(|(index, _)| index)
                .collect();
            let weights: Vec<i64> = members
                .iter()
                .map(|index| base(*index, &lines[*index]).max(0))
                .collect();
            let shares = allocate_covered(&weights, *amount);
            let mut left = *amount;
            for (index, share) in members.into_iter().zip(shares) {
                if share > 0 {
                    *placed.entry(Some(lines[index].id.clone())).or_default() += share;
                    left -= share;
                }
            }
            if left > 0 {
                *placed.entry(None).or_default() += left;
            }
        }
        out.extend(
            placed
                .into_iter()
                .filter(|(_, amount)| *amount > 0)
                .map(|(line, amount)| (*grant_id, line, amount)),
        );
    }
    out
}

/// The lines promotional credit moving taxed again, and which of them were netted (R-031).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repriced {
    pub retaxed: Vec<usize>,
    pub netted: Vec<bool>,
}

fn sum(mut values: impl Iterator<Item = i64>) -> Result<i64, Overflow> {
    values.try_fold(0_i64, |sum, value| sum.checked_add(value).ok_or(Overflow))
}

impl Document {
    /// What promotional credit may take off the document; within what `shown` gave each line once
    /// cash was taken against it.
    pub fn promotional_request(
        &self,
        shown: Option<&HashMap<String, i64>>,
    ) -> Result<(Request, Vec<Option<usize>>), Overflow> {
        let tax_inclusive = self.tax_inclusive;
        match shown {
            Some(shown) => pretax_request_within(&self.lines, tax_inclusive, shown),
            None => invoice_request(&self.lines, |_, line| taxed_base(line, tax_inclusive)),
        }
    }

    pub fn usage_charges(&self, request: &Request, of_line: &[Option<usize>]) -> Vec<UsageCharge> {
        let tax_inclusive = self.tax_inclusive;
        usage_charges(&self.lines, of_line, request, tax_inclusive, |_, line| {
            taxed_base(line, tax_inclusive)
        })
    }

    pub fn shares(&self, of_line: &[Option<usize>], allocation: &Allocation) -> Vec<i64> {
        let tax_inclusive = self.tax_inclusive;
        line_shares(&self.lines, of_line, allocation, |_, line| {
            taxed_base(line, tax_inclusive)
        })
    }

    /// Lands the promotional credit `held` on the lines and taxes again the ones it moves. The
    /// lines' shares add up to the draw (I3) and no line's taxable amount is lost (I23).
    pub fn take_promotional(
        &mut self,
        of_line: &[Option<usize>],
        held: &Allocation,
        rates: &Rates,
    ) -> Result<(Vec<i64>, Option<Repriced>), SettleError> {
        let shares = self.shares(of_line, held);
        if sum(shares.iter().copied())? != held.total()? {
            return Err(SettleError::Unbalanced);
        }
        let tax_inclusive = self.tax_inclusive;
        let before = sum(self
            .lines
            .iter()
            .map(|line| taxed_base(line, tax_inclusive)))?;
        let repriced = self.reprice(&shares, rates)?;
        let after = sum(self
            .lines
            .iter()
            .map(|line| taxed_base(line, tax_inclusive)))?;
        if before != after {
            return Err(SettleError::Unbalanced);
        }
        Ok((shares, repriced))
    }

    /// Moves each line to its share and re-taxes the lines it moves by their rates, each rate
    /// rounded once over them (OQ14). The header is what the lines add up to, the discount stays
    /// and promotional credit is the rest. None when no share moved.
    pub fn reprice(&mut self, shares: &[i64], rates: &Rates) -> Result<Option<Repriced>, Overflow> {
        // A draft stored by an earlier release may carry an untaxed line's undiscounted price.
        for line in &mut self.lines {
            line.total = line.taxable + line.tax;
        }
        let share_of = |index: usize, line: &Line| {
            shares
                .get(index)
                .copied()
                .unwrap_or_else(|| line.promotional.unwrap_or(0))
        };
        let moved = self
            .lines
            .iter()
            .enumerate()
            .any(|(index, line)| line.promotional.unwrap_or(0) != share_of(index, line));
        if !moved {
            return Ok(None);
        }
        let retaxed: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(index, line)| {
                line.promotional.unwrap_or(0) != share_of(*index, line)
                    || (!line.taxes.is_empty() && !line.taxed_at_redemption)
            })
            .map(|(index, _)| index)
            .collect();
        let mut priced: Vec<Line> = retaxed
            .iter()
            .map(|index| {
                let line = &self.lines[*index];
                let share = share_of(*index, line);
                let base = taxed_base(line, self.tax_inclusive);
                let mut priced = if self.tax_inclusive {
                    line.clone().into_quote()
                } else {
                    line.clone()
                };
                priced.taxable = base - share;
                priced.promotional = (share > 0).then_some(share);
                priced
            })
            .collect();
        let priced_rates: Vec<&[Rate]> = retaxed.iter().map(|index| rates.of(*index)).collect();
        let applied = apply_rates(&mut priced, &priced_rates, self.tax_inclusive);
        for (index, line) in retaxed.iter().zip(priced) {
            self.lines[*index] = line;
        }
        let subtotal = sum(self.lines.iter().map(|line| line.figures.subtotal))?;
        let subtotal_recurring = sum(self
            .lines
            .iter()
            .filter(|line| line.recurring)
            .map(|line| line.figures.subtotal))?;
        let tax = sum(self.lines.iter().map(|line| line.tax))?;
        let total = sum(self.lines.iter().map(|line| line.total))?;
        let promotional = subtotal
            .checked_sub(self.discount)
            .and_then(|left| left.checked_add(tax))
            .and_then(|left| left.checked_sub(total))
            .ok_or(Overflow)?;
        self.header = Header {
            subtotal,
            subtotal_recurring,
            tax,
            promotional,
            total,
        };
        Ok(Some(Repriced {
            retaxed,
            netted: applied.netted,
        }))
    }

    /// What paid credit may cover once promotional credit is off: the unpaid part of `total`,
    /// at most what is due less the cash already committed to it.
    pub fn paid_request(
        &self,
        total: i64,
        already_paid: i64,
        committed: i64,
    ) -> Result<(Request, Vec<Option<usize>>), Overflow> {
        let (request, of_line) = paid_request(&self.lines, total, already_paid)?;
        let due = amount_due(total, already_paid, 0);
        Ok((capped(request, due.saturating_sub(committed)), of_line))
    }

    /// [`line_allocation`] of paid credit, by what each line still costs.
    pub fn paid_line_allocation(
        &self,
        of_line: &[Option<usize>],
        request: &Request,
        paid: &Allocation,
    ) -> Vec<(CreditGrantId, Option<String>, i64)> {
        line_allocation(&self.lines, of_line, request, paid, |_, line| line.total)
    }
}

/// The grants a document may draw and the settlements it would keep, as finalizing at `at` would
/// find them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    /// Spendable credit: an expired grant not rolled over yet stands in for its rollover.
    pub drawable: Vec<Grant>,
    /// What grants hold of the open settlements of the periods the document bills (R10).
    pub settled: Vec<Settled>,
    pub at: DateTime<Utc>,
}

/// What one document holds from a pool: the settlements it keeps, then what it draws afresh.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub kept: Allocation,
    pub fresh: Allocation,
}

impl Drawn {
    pub fn held(&self) -> Result<Allocation, Overflow> {
        self.kept.clone().then(self.fresh.clone())
    }
}

/// What finalizing at `grants.at` would hold from `pool` for `request`: the open settlements it
/// keeps (paid ones placed first, so promotional credit never pays usage they paid), then fresh
/// draws on the rest.
pub fn estimate(
    grants: &Grants,
    pool: Pool,
    request: &Request,
    charges: &[UsageCharge],
) -> Result<Drawn, SettleError> {
    if request.requested_minor <= 0 {
        return Ok(Drawn::default());
    }
    let (ahead, kept) = if grants.settled.is_empty() {
        (request.clone(), Allocation::default())
    } else {
        let in_pool = |pool: Pool| {
            grants
                .settled
                .iter()
                .filter(|settled| pool.admits(settled.grant.classification))
                .cloned()
                .collect::<Vec<_>>()
        };
        let ahead = match pool {
            Pool::Promotional => after_settlements(request, charges, &in_pool(Pool::Paid))?,
            Pool::All | Pool::Paid => request.clone(),
        };
        let kept = assign_settled(&in_pool(pool), &usage_only(&ahead)?)?;
        (ahead, kept)
    };
    let rest = ahead.remainder(&kept)?;
    let fresh = if rest.requested_minor <= 0 {
        Allocation::default()
    } else {
        let drawable = returned_to_drawable(grants, pool, &kept)?;
        allocate(&drawable, &rest, grants.at, pool)?
    };
    Ok(Drawn { kept, fresh })
}

/// The drawable grants once what each settlement of `pool` held beyond what the document keeps
/// is back on its grant, as finalizing returns it before drawing: spendable again unless lapsed.
fn returned_to_drawable(
    grants: &Grants,
    pool: Pool,
    kept: &Allocation,
) -> Result<Vec<Grant>, Overflow> {
    let mut drawable = grants.drawable.clone();
    let mut returned: Vec<(&Grant, i64)> = Vec::new();
    for settled in grants
        .settled
        .iter()
        .filter(|settled| pool.admits(settled.grant.classification))
    {
        match returned
            .iter_mut()
            .find(|(grant, _)| grant.id == settled.grant.id)
        {
            Some((_, held)) => *held = held.checked_add(settled.amount).ok_or(Overflow)?,
            None => returned.push((&settled.grant, settled.amount)),
        }
    }
    for (grant, held) in returned {
        let kept_of = kept
            .draws
            .iter()
            .filter(|(id, _)| *id == grant.id)
            .try_fold(0_i64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(Overflow)?;
        let excess = held.checked_sub(kept_of).ok_or(Overflow)?;
        let back = Grant {
            remaining_minor: excess,
            ..grant.clone()
        };
        if !back.spendable_at(grants.at) {
            continue;
        }
        match drawable.iter_mut().find(|drawn| drawn.id == grant.id) {
            Some(drawn) => {
                drawn.remaining_minor =
                    drawn.remaining_minor.checked_add(excess).ok_or(Overflow)?;
            }
            None => drawable.push(back),
        }
    }
    Ok(drawable)
}

/// What settled credit may keep of `request`: its usage, never a fee billed beside it (R10).
fn usage_only(request: &Request) -> Result<Request, Overflow> {
    let usage = request
        .demands
        .iter()
        .filter(|demand| demand.usage)
        .try_fold(0_i64, |sum, demand| {
            sum.checked_add(demand.amount_minor.max(0))
        })
        .ok_or(Overflow)?;
    Ok(Request {
        requested_minor: usage.min(request.requested_minor.max(0)),
        demands: request
            .demands
            .iter()
            .map(|demand| Demand {
                amount_minor: if demand.usage { demand.amount_minor } else { 0 },
                ..*demand
            })
            .collect(),
    })
}

/// Which total paid credit pays: the lines' fold (R-076), or the document's header as promotional
/// credit moving left it (a stored or merged document).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TotalBasis {
    Folded,
    Header,
}

/// Cash already against the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cash {
    pub already_paid: i64,
    /// Charges pending or settled against it: paid credit never pays what they will.
    pub committed: i64,
    /// The promotional credit each line showed when cash was taken against it.
    pub shown: Option<HashMap<String, i64>>,
    pub total: TotalBasis,
}

impl Cash {
    pub fn none(total: TotalBasis) -> Self {
        Self {
            already_paid: 0,
            committed: 0,
            shown: None,
            total,
        }
    }
}

/// One pool's part of a settlement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSettlement {
    pub request: Request,
    pub of_line: Vec<Option<usize>>,
    pub drawn: Drawn,
    pub held: Allocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    /// The document with its promotional credit on its lines, those it moved taxed again.
    pub document: Document,
    pub repriced: Option<Repriced>,
    pub promotional: PoolSettlement,
    /// Each line's promotional credit.
    pub shares: Vec<i64>,
    pub paid: PoolSettlement,
    /// The settled lines' fold (R-076).
    pub totals: Totals,
    /// The paid credit applied.
    pub applied: i64,
}

/// Settles `document` against `grants` in one pass: promotional credit before tax, tax by `rates`
/// on what is left, then paid credit. The same inputs give the same settlement, so an
/// amount-dependent rate is resolved again on the result and settled anew.
pub fn settle(
    document: &Document,
    grants: &Grants,
    rates: &Rates,
    cash: &Cash,
) -> Result<Settlement, SettleError> {
    let mut document = document.clone();
    let (request, of_line) = document.promotional_request(cash.shown.as_ref())?;
    let charges = document.usage_charges(&request, &of_line);
    let drawn = estimate(grants, Pool::Promotional, &request, &charges)?;
    let held = drawn.held()?;
    let (shares, repriced) = document.take_promotional(&of_line, &held, rates)?;
    let promotional = PoolSettlement {
        request,
        of_line,
        drawn,
        held,
    };

    let totals = document.totals();
    let total = match cash.total {
        TotalBasis::Folded => totals.total,
        TotalBasis::Header => document.header.total,
    };
    let (request, of_line) = document.paid_request(total, cash.already_paid, cash.committed)?;
    let drawn = estimate(grants, Pool::Paid, &request, &[])?;
    let held = drawn.held()?;
    let applied = held.total()?;
    Ok(Settlement {
        document,
        repriced,
        promotional,
        shares,
        paid: PoolSettlement {
            request,
            of_line,
            drawn,
            held,
        },
        totals,
        applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::allocation::{Application, Classification, Scope};
    use crate::document::{Figures, LineTax};
    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 15, 0, 0, 0).unwrap()
    }

    fn line(id: &str, amount: i64, kind: LineKind) -> Line {
        Line {
            id: id.to_owned(),
            kind,
            subscription: None,
            product: None,
            recurring: !matches!(kind, LineKind::Usage { .. }),
            figures: Figures {
                subtotal: amount,
                unit_price: None,
                sub_lines: Vec::new(),
            },
            quote: None,
            taxable: amount,
            tax: 0,
            total: amount,
            tax_rate: Decimal::ZERO,
            taxes: Vec::new(),
            promotional: None,
            taxed_at_redemption: kind == LineKind::Credit,
        }
    }

    fn usage() -> LineKind {
        LineKind::Usage {
            dated_before: Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        }
    }

    /// A line taxed by the engine at `rate`.
    fn taxed(id: &str, amount: i64, rate: Decimal) -> Line {
        let mut taxed = line(id, amount, LineKind::Fee);
        let tax = crate::rounding::round_to_minor(Decimal::from(amount) * rate);
        taxed.tax_rate = rate;
        taxed.tax = tax;
        taxed.total = amount + tax;
        taxed.taxes = vec![LineTax {
            rate,
            name: "VAT".to_owned(),
            amount: tax,
        }];
        taxed
    }

    fn rates_of(lines: &[Line]) -> Rates {
        Rates(
            lines
                .iter()
                .map(|line| {
                    line.taxes
                        .iter()
                        .map(|tax| Rate {
                            rate: tax.rate,
                            name: tax.name.clone(),
                        })
                        .collect()
                })
                .collect(),
        )
    }

    fn document(lines: Vec<Line>, discount: i64, tax_inclusive: bool) -> Document {
        let mut document = Document {
            lines,
            discount,
            tax_inclusive,
            subscription: None,
            header: Header::default(),
        };
        let totals = document.totals();
        document.header = Header {
            subtotal: totals.subtotal,
            subtotal_recurring: totals.subtotal_recurring,
            tax: totals.tax,
            promotional: totals.promotional,
            total: totals.total,
        };
        document
    }

    fn grant(id: u128, remaining: i64, classification: Classification) -> Grant {
        Grant {
            id: Uuid::from_u128(id).into(),
            scope: Scope {
                subscription: None,
                application: Application::Invoice,
                products: Vec::new(),
            },
            remaining_minor: remaining,
            priority: 100,
            classification,
            effective_at: at() - chrono::Duration::days(30),
            pays_usage_from: at() - chrono::Duration::days(30),
            lapses_at: None,
            created_at: at() - chrono::Duration::days(30),
        }
    }

    fn grants(drawable: Vec<Grant>) -> Grants {
        Grants {
            drawable,
            settled: Vec::new(),
            at: at(),
        }
    }

    /// Each rate's tax over the lines, as the breakdown states it (I24).
    fn breakdown(lines: &[Line]) -> BTreeMap<String, i64> {
        let mut rows = BTreeMap::new();
        for line in lines {
            for tax in &line.taxes {
                *rows
                    .entry(format!("{}:{}", tax.name, tax.rate))
                    .or_default() += tax.amount;
            }
        }
        rows
    }

    #[test]
    fn credit_purchases_are_never_eligible_and_usage_is_its_own_demand() {
        let lines = [
            line("fee", 3_000, LineKind::Fee),
            line("usage", 1_000, usage()),
            line("pack", 5_000, LineKind::Credit),
            line("shortfall", 500, LineKind::Fee),
        ];
        let (request, of_line) = pretax_request(&lines, |_, l| l.taxable).unwrap();
        assert_eq!(request.requested_minor, 4_500);
        assert_eq!(
            request
                .demands
                .iter()
                .map(|d| (d.usage, d.amount_minor))
                .collect::<Vec<_>>(),
            vec![(false, 3_500), (true, 1_000)]
        );
        assert_eq!(of_line, vec![Some(0), Some(1), None, Some(0)]);
    }

    #[test]
    fn a_document_owing_nothing_requests_no_promotional_credit() {
        let untaxed_credit = |id: &str, amount: i64| Line {
            taxable: 0,
            ..line(id, amount, LineKind::Fee)
        };
        let downgrade = [
            untaxed_credit("a", -5_110),
            untaxed_credit("b", -1_290),
            line("c", 1_497, LineKind::Fee),
            line("d", 516, LineKind::Fee),
        ];
        let (request, _) = invoice_request(&downgrade, |_, l| l.taxable).unwrap();
        assert_eq!(request.requested_minor, 0);

        let upgrade = [
            untaxed_credit("a", -1_497),
            untaxed_credit("b", -516),
            line("c", 5_110, LineKind::Fee),
            line("d", 1_290, LineKind::Fee),
        ];
        let (request, of_line) = invoice_request(&upgrade, |_, l| l.taxable).unwrap();
        assert_eq!(
            request.requested_minor, 4_387,
            "the net, not the 6,400 charged"
        );
        let allocation = Allocation {
            draws: vec![(Uuid::from_u128(1).into(), 4_387)],
            covered: vec![4_387],
            placed: Vec::new(),
        };
        assert_eq!(
            line_shares(&upgrade, &of_line, &allocation, |_, l| l.taxable),
            vec![0, 0, 3_503, 884]
        );
    }

    /// Promotional credit moving from a 20% line to a 5.5% one re-taxes both by their rates.
    #[test]
    fn a_share_moving_between_rates_re_taxes_both_lines() {
        let mut standard = taxed("standard", 10_000, dec!(0.20));
        standard.taxable = 7_000;
        standard.promotional = Some(3_000);
        standard.tax = 1_400;
        standard.total = 8_400;
        let reduced = taxed("reduced", 10_000, dec!(0.055));
        let mut document = document(vec![standard, reduced], 0, false);
        let rates = rates_of(&document.lines);
        document
            .reprice(&[0, 3_000], &rates)
            .unwrap()
            .expect("moved");
        assert_eq!(document.lines[0].tax, 2_000);
        assert_eq!(document.lines[1].tax, 385);
        let header = document.header;
        assert_eq!(
            (header.promotional, header.tax, header.total),
            (3_000, 2_385, 19_385)
        );
    }

    /// 0.01 moving from a 20% line to a 5.5% one: 20% is rounded once over its two lines (OQ14).
    #[test]
    fn a_share_moving_across_rates_rounds_each_rate_once_over_its_lines() {
        let at_rate = |id: &str, amount: i64, rate: Decimal, taxable: i64, tax: i64| {
            let mut item = taxed(id, amount, rate);
            item.taxable = taxable;
            item.promotional = (amount > taxable).then_some(amount - taxable);
            item.tax = tax;
            item.taxes[0].amount = tax;
            item.total = taxable + tax;
            item
        };
        let mut document = document(
            vec![
                at_rate("a", 10, dec!(0.20), 5, 1),
                at_rate("b", 7, dec!(0.20), 7, 1),
                at_rate("c", 1_000, dec!(0.055), 1_000, 55),
            ],
            0,
            false,
        );
        let rates = rates_of(&document.lines);
        document
            .reprice(&[4, 0, 1], &rates)
            .unwrap()
            .expect("moved");
        assert_eq!(
            document
                .lines
                .iter()
                .map(|line| (line.taxable, line.tax))
                .collect::<Vec<_>>(),
            vec![(6, 1), (7, 2), (999, 55)]
        );
        let header = document.header;
        assert_eq!(
            (header.promotional, header.tax, header.total),
            (5, 58, 1_070)
        );
    }

    /// 9.99 quoted at 20% with a 1.00 coupon: the share moving from 3.33 to 0.10 leaves 8.89.
    #[test]
    fn a_moving_share_on_a_discounted_tax_inclusive_line_keeps_every_cent() {
        let mut quoted = line("a", 999, LineKind::Fee);
        quoted.tax_rate = dec!(0.2);
        quoted.taxable = 999 - 100 - 333;
        quoted.promotional = Some(333);
        let rates = vec![Rate {
            rate: dec!(0.2),
            name: "VAT".to_owned(),
        }];
        apply_rates(std::slice::from_mut(&mut quoted), &[rates.as_slice()], true);
        let mut document = document(vec![quoted], 0, true);
        let line = &document.lines[0];
        document.discount = line.figures.subtotal - line.net_promotional() + line.tax - line.total;
        let repriced = document
            .reprice(&[10], &Rates(vec![rates]))
            .unwrap()
            .expect("moved");
        assert_eq!(repriced.netted, vec![true]);
        assert_eq!(document.lines[0].total, 889);
        let header = document.header;
        assert_eq!(header.total, 889);
        assert_eq!(
            header.subtotal - document.discount - header.promotional + header.tax,
            header.total
        );
        assert_eq!(document.lines[0].promotional, Some(10));
    }

    /// A line the promotional credit took whole keeps its rate, so giving the credit back taxes it.
    #[test]
    fn a_line_taken_whole_is_taxed_again_when_the_credit_is_gone() {
        let mut taken = taxed("a", 3_000, dec!(0.20));
        taken.taxable = 0;
        taken.tax = 0;
        taken.total = 0;
        taken.taxes[0].amount = 0;
        taken.promotional = Some(3_000);
        let mut document = document(vec![taken], 0, false);
        let rates = rates_of(&document.lines);
        document.reprice(&[0], &rates).unwrap().expect("moved");
        let header = document.header;
        assert_eq!(
            (header.promotional, header.tax, header.total),
            (0, 600, 3_600)
        );
    }

    /// 100.00 at 20% and 50.00 at 5.5%, 30.00 of promotional credit and 40.00 of paid credit: the
    /// shares add up to the draw (I3), the taxable amounts to subtotal less discount less
    /// promotional credit (I23), the breakdown to the tax (I24).
    #[test]
    fn a_settlement_balances_its_draws_lines_and_taxes() {
        let document = document(
            vec![
                taxed("standard", 10_000, dec!(0.20)),
                taxed("reduced", 5_000, dec!(0.055)),
            ],
            0,
            false,
        );
        let rates = rates_of(&document.lines);
        let grants = grants(vec![
            grant(1, 3_000, Classification::Promotional),
            grant(2, 4_000, Classification::Paid),
        ]);
        let settlement =
            settle(&document, &grants, &rates, &Cash::none(TotalBasis::Folded)).unwrap();

        let promotional = settlement.promotional.held.total().unwrap();
        assert_eq!(promotional, 3_000);
        assert_eq!(settlement.shares.iter().sum::<i64>(), promotional, "I3");
        assert_eq!(settlement.shares, vec![2_000, 1_000]);

        let lines = &settlement.document.lines;
        let totals = settlement.totals;
        assert_eq!(
            lines.iter().map(|line| line.taxable).sum::<i64>(),
            totals.subtotal - totals.discount - totals.promotional,
            "I23"
        );
        assert_eq!(breakdown(lines).values().sum::<i64>(), totals.tax, "I24");
        assert_eq!(totals.tax, 1_600 + 220);
        assert_eq!(totals.total, 15_000 - 3_000 + 1_820);
        assert_eq!(settlement.applied, 4_000);
        assert_eq!(
            settlement
                .document
                .paid_line_allocation(
                    &settlement.paid.of_line,
                    &settlement.paid.request,
                    &settlement.paid.held,
                )
                .iter()
                .map(|(_, _, amount)| amount)
                .sum::<i64>(),
            4_000,
            "paid line allocations add up to the paid draw"
        );
    }

    /// A draw its lines cannot carry is refused, never kept with no line holding it (I3).
    #[test]
    fn a_draw_beyond_what_its_lines_carry_is_unbalanced() {
        let document = document(vec![taxed("standard", 10_000, dec!(0.20))], 0, false);
        let rates = rates_of(&document.lines);
        let grants = grants(vec![grant(1, 3_000, Classification::Promotional)]);
        let settlement =
            settle(&document, &grants, &rates, &Cash::none(TotalBasis::Folded)).unwrap();

        let mut held = settlement.promotional.held.clone();
        held.draws[0].1 += 500;
        let mut again = document.clone();
        assert_eq!(
            again.take_promotional(&settlement.promotional.of_line, &held, &rates),
            Err(SettleError::Unbalanced)
        );
    }

    /// An amount-dependent rate is resolved again on the settled amounts and settled anew: the
    /// shares stay those taken before tax, and settling again changes nothing.
    #[test]
    fn a_threshold_rate_re_enters_and_settles_the_same() {
        let document = document(vec![taxed("seats", 10_000, dec!(0.20))], 0, false);
        let grants = grants(vec![grant(1, 6_000, Classification::Promotional)]);
        let cash = Cash::none(TotalBasis::Folded);

        let first = settle(&document, &grants, &rates_of(&document.lines), &cash).unwrap();
        // Below a threshold the engine resolves 10% on what is left to tax.
        assert_eq!(first.totals.subtotal - first.totals.promotional, 4_000);
        let lower = Rates(vec![vec![Rate {
            rate: dec!(0.10),
            name: "VAT".to_owned(),
        }]]);
        let second = settle(&document, &grants, &lower, &cash).unwrap();
        let again = settle(&document, &grants, &lower, &cash).unwrap();
        assert_eq!(second, again, "idempotent on the same inputs");
        assert_eq!(
            second.shares, first.shares,
            "the shares are taken before tax"
        );
        assert_eq!(second.document.lines[0].tax, 400);
        assert_eq!(second.totals.total, 4_400);
    }

    /// 30.03 of paid credit settled 50.00 of usage at 20% ahead of the invoice: the invoice keeps
    /// it, and promotional credit takes only the usage it left before tax.
    #[test]
    fn promotional_credit_never_pays_usage_a_kept_settlement_paid() {
        let mut metered = taxed("usage", 5_000, dec!(0.20));
        metered.kind = usage();
        metered.recurring = false;
        let document = document(vec![metered], 0, false);
        let grants = Grants {
            drawable: vec![grant(1, 10_000, Classification::Promotional)],
            settled: vec![Settled {
                window: None,
                grant: grant(2, 0, Classification::Paid),
                amount: 3_003,
            }],
            at: at(),
        };
        let settlement = settle(
            &document,
            &grants,
            &rates_of(&document.lines),
            &Cash::none(TotalBasis::Folded),
        )
        .unwrap();
        assert_eq!(settlement.shares, vec![2_498]);
        // 25.02 left with 5.00 of tax: the settlement pays it, the cent over is not kept.
        assert_eq!(settlement.totals.total, 3_002);
        assert_eq!(settlement.paid.drawn.kept.total().unwrap(), 3_002);
    }

    /// 20.00 of paid credit settled a period's usage, and the invoice bills 30.00 of fees and
    /// 10.00 of that usage: it keeps 10.00 of the settlement for the usage, never for the fees;
    /// the other 10.00 goes back to its grant, so the fees draw the wallet's 25.00 and 5.00 of it
    /// afresh, as finalizing returns then draws them.
    /// Rules: R10
    #[test]
    fn a_kept_settlement_pays_usage_only() {
        let mut metered = line("usage", 1_000, usage());
        metered.recurring = false;
        let document = document(vec![line("fee", 3_000, LineKind::Fee), metered], 0, false);
        let grants = Grants {
            drawable: vec![grant(1, 2_500, Classification::Paid)],
            settled: vec![Settled {
                window: None,
                grant: grant(2, 0, Classification::Paid),
                amount: 2_000,
            }],
            at: at(),
        };
        let settlement = settle(
            &document,
            &grants,
            &Rates(vec![Vec::new(), Vec::new()]),
            &Cash::none(TotalBasis::Folded),
        )
        .unwrap();
        assert_eq!(
            settlement.paid.drawn.kept.draws,
            vec![(Uuid::from_u128(2).into(), 1_000)]
        );
        assert_eq!(
            settlement.paid.drawn.fresh.draws,
            vec![
                (Uuid::from_u128(1).into(), 2_500),
                (Uuid::from_u128(2).into(), 500)
            ]
        );
        assert_eq!(settlement.applied, 4_000);
    }

    /// G (10.00, lapsed) and H (30.00, spent) settled 40.00 of usage at G's lapse; the invoice
    /// bills 32.00 of it after a coupon, and a 40.00 fee. It keeps G's 10.00 and 22.00 of H's;
    /// the 8.00 H gets back is spendable, so the fee draws it: 72.00 requested, 40.00 drawn.
    /// Rules: R10
    #[test]
    fn excess_returned_to_a_spendable_grant_pays_the_fee() {
        let lapsed = Grant {
            lapses_at: Some(at() - chrono::Duration::days(5)),
            ..grant(1, 0, Classification::Paid)
        };
        let spent = grant(2, 0, Classification::Paid);
        let demand = |usage: bool, amount_minor: i64| Demand {
            subscription: None,
            product: None,
            usage,
            amount_minor,
            dated_before: usage.then(|| at() - chrono::Duration::days(1)),
        };
        let request = Request {
            requested_minor: 7_200,
            demands: vec![demand(true, 3_200), demand(false, 4_000)],
        };
        let grants = Grants {
            drawable: Vec::new(),
            settled: vec![
                Settled {
                    window: None,
                    grant: lapsed.clone(),
                    amount: 1_000,
                },
                Settled {
                    window: None,
                    grant: spent.clone(),
                    amount: 3_000,
                },
            ],
            at: at(),
        };
        let drawn = estimate(&grants, Pool::Paid, &request, &[]).unwrap();
        assert_eq!(
            drawn.kept.draws,
            vec![(lapsed.id, 1_000), (spent.id, 2_200)]
        );
        assert_eq!(drawn.fresh.draws, vec![(spent.id, 800)]);
        assert_eq!(drawn.held().unwrap().total().unwrap(), 4_000);
    }

    #[test]
    fn paid_credit_never_pays_a_credit_purchase_nor_what_was_paid() {
        let lines = [
            line("fee", 3_000, LineKind::Fee),
            line("pack", 5_000, LineKind::Credit),
        ];
        let (request, _) = paid_request(&lines, 8_000, 0).unwrap();
        assert_eq!(request.requested_minor, 3_000);
        let (request, _) = paid_request(&lines, 8_000, 6_000).unwrap();
        assert_eq!(request.requested_minor, 2_000);
    }

    #[test]
    fn cash_taken_caps_each_line_at_the_credit_it_showed() {
        let mut fee = line("fee", 3_000, LineKind::Fee);
        fee.taxable = 2_000;
        fee.promotional = Some(1_000);
        let shown = HashMap::from([("fee".to_owned(), 400)]);
        let (request, _) = pretax_request_within(&[fee], false, &shown).unwrap();
        assert_eq!(request.requested_minor, 400);
    }
}

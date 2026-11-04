//! A customer's pending documents estimated in billing order against one in-memory copy of its
//! grants, so no preview spends credit a document billed earlier will draw: every draft first
//! (they finalize first), then each live subscription's next invoice by date, then subscription.
use crate::StoreResult;
use crate::domain::credit::{
    CreditApplication, CreditGrantSource, GrantClassification, GrantScope,
};
use crate::domain::credit_groups::{ACCOUNT_BALANCE, FREE_CREDIT, PURCHASED_CREDIT};
use crate::domain::credit_outlook::{
    BilledFirst, CreditDeduction, CreditGroup, CreditGroupKind, UpcomingChange,
    UpcomingInvoiceForecast,
};
use crate::domain::{Invoice, LineItem, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::credit::allocation::{
    Allocation, CreditDemand, CreditRequest, GrantCandidate, GrantPool, allocate, assign,
};
use crate::repositories::credit::customer_credit::{
    BONUS_CREDIT, UsageCharge, estimate_candidates_tx, estimate_kept_tx, grant_scope,
    list_drawable_tx, normalized_currency, settlement_holders_tx, source,
};
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::credit::value_application::EstimateContext;
use crate::services::credit::{invalid, precondition};
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::invoices::upcoming::NextInvoiceBasis;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::customer_credit::{
    CreditUsageSettlementRow, CustomerCreditGrantRow, ScopedGrantRow,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::query::credit_forecast::{list_once_credit_owners, list_scheduled_stops};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

/// The customer's grants of one denomination as a forecast spends them.
#[derive(Debug, Clone)]
pub struct CreditBook {
    pub denomination: String,
    grants: Vec<(ScopedGrantRow, GrantCandidate)>,
}

impl CreditBook {
    /// Expired credit not rolled over yet stands in for what it rolls over to
    /// (`estimate_candidates_tx`).
    pub(crate) async fn load_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        denomination: &str,
        at: DateTime<Utc>,
    ) -> StoreResult<Self> {
        let denomination = normalized_currency(denomination);
        let rows = list_drawable_tx(conn, tenant_id, customer_id, &denomination).await?;
        let candidates = estimate_candidates_tx(conn, tenant_id, &rows, at).await?;
        Ok(Self {
            denomination,
            grants: rows.into_iter().zip(candidates).collect(),
        })
    }

    pub(crate) fn candidates(&self) -> Vec<GrantCandidate> {
        self.grants
            .iter()
            .map(|(_, candidate)| candidate.clone())
            .collect()
    }

    pub(crate) fn row(&self, grant_id: CreditGrantId) -> Option<&ScopedGrantRow> {
        self.grants
            .iter()
            .find(|(row, _)| row.grant.id == grant_id)
            .map(|(row, _)| row)
    }

    pub(crate) fn spend(&mut self, allocation: &Allocation) -> StoreResult<()> {
        for (grant_id, amount) in &allocation.draws {
            let (_, grant) = self
                .grants
                .iter_mut()
                .find(|(_, grant)| grant.id == *grant_id)
                .ok_or_else(|| precondition("a credit forecast drew a grant it never read"))?;
            grant.remaining_minor = grant
                .remaining_minor
                .checked_sub(*amount)
                .filter(|left| *left >= 0)
                .ok_or_else(|| invalid("a credit forecast drew more than a grant holds"))?;
        }
        Ok(())
    }
}

/// What one document holds from a pool of one denomination.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub denomination: String,
    pub pool: GrantPool,
    /// Drawn afresh: it leaves the book once the document is estimated.
    pub fresh: Allocation,
    /// Everything the document holds: the open settlements it keeps, then `fresh`.
    pub held: Allocation,
}

#[derive(Debug, Default)]
struct BooksState {
    books: Vec<CreditBook>,
    drawn: Vec<Drawn>,
}

/// The books of one forecast, read lazily per denomination. The estimates of the document being
/// computed each record what they drew; a later estimate of the same pool replaces an earlier one
/// (a promotional estimate repriced after tax).
#[derive(Debug)]
pub(crate) struct CreditBooks {
    tenant_id: TenantId,
    customer_id: CustomerId,
    at: DateTime<Utc>,
    state: Mutex<BooksState>,
}

impl CreditBooks {
    pub(crate) fn new(tenant_id: TenantId, customer_id: CustomerId, at: DateTime<Utc>) -> Self {
        Self {
            tenant_id,
            customer_id,
            at,
            state: Mutex::new(BooksState::default()),
        }
    }

    fn state(&self) -> StoreResult<MutexGuard<'_, BooksState>> {
        self.state
            .lock()
            .map_err(|_| precondition("a credit forecast was interrupted"))
    }

    pub(crate) async fn candidates_tx(
        &self,
        conn: &mut PgConn,
        denomination: &str,
    ) -> StoreResult<Vec<GrantCandidate>> {
        let denomination = normalized_currency(denomination);
        let loaded = self
            .state()?
            .books
            .iter()
            .find(|book| book.denomination == denomination)
            .map(CreditBook::candidates);
        if let Some(candidates) = loaded {
            return Ok(candidates);
        }
        let book = CreditBook::load_tx(
            conn,
            self.tenant_id,
            self.customer_id,
            &denomination,
            self.at,
        )
        .await?;
        let candidates = book.candidates();
        let mut state = self.state()?;
        if !state
            .books
            .iter()
            .any(|book| book.denomination == denomination)
        {
            state.books.push(book);
        }
        Ok(candidates)
    }

    pub(crate) fn record(&self, drawn: Drawn) -> StoreResult<()> {
        let mut state = self.state()?;
        state.drawn.retain(|other| {
            !(other.denomination == drawn.denomination && other.pool == drawn.pool)
        });
        state.drawn.push(drawn);
        Ok(())
    }

    /// Ends the document being estimated: what it drew afresh leaves the books.
    pub(crate) fn close_document(&self) -> StoreResult<Vec<Drawn>> {
        let mut state = self.state()?;
        let drawn = std::mem::take(&mut state.drawn);
        for entry in drawn.iter().filter(|entry| !entry.fresh.draws.is_empty()) {
            state
                .books
                .iter_mut()
                .find(|book| book.denomination == entry.denomination)
                .ok_or_else(|| precondition("a credit forecast drew credit it never read"))?
                .spend(&entry.fresh)?;
        }
        Ok(drawn)
    }

    /// Forgets the document being estimated, as if it drew nothing.
    pub(crate) fn abandon_document(&self) -> StoreResult<()> {
        self.state()?.drawn.clear();
        Ok(())
    }

    /// Whether customer-wide credit is left in any of `denominations`; a denomination not read
    /// yet still holds all of its own.
    pub(crate) fn shared_left(&self, denominations: &[String]) -> StoreResult<bool> {
        let state = self.state()?;
        Ok(denominations.iter().any(|denomination| {
            state
                .books
                .iter()
                .find(|book| book.denomination == *denomination)
                .is_none_or(|book| {
                    book.grants.iter().any(|(_, grant)| {
                        grant.scope.subscription_id.is_none() && grant.remaining_minor > 0
                    })
                })
        }))
    }

    pub(crate) fn into_books(self) -> StoreResult<Vec<CreditBook>> {
        self.state
            .into_inner()
            .map(|state| state.books)
            .map_err(|_| precondition("a credit forecast was interrupted"))
    }
}

/// `estimate_tx` on a forecast's book: the open settlements the invoice keeps, then fresh draws
/// on what the documents billed before it left.
pub(crate) async fn estimate_on_book(
    conn: &mut PgConn,
    book: &CreditBooks,
    context: &EstimateContext<'_>,
    pool: GrantPool,
    request: &CreditRequest,
    charges: &[UsageCharge],
) -> StoreResult<Allocation> {
    let denomination = normalized_currency(context.currency);
    if request.requested_minor <= 0 {
        book.record(Drawn {
            denomination,
            pool,
            fresh: Allocation::default(),
            held: Allocation::default(),
        })?;
        return Ok(Allocation::default());
    }
    let (ahead, kept) = estimate_kept_tx(
        conn,
        context.tenant_id,
        context.currency,
        context.invoice_id,
        &context.periods,
        pool,
        request,
        charges,
    )
    .await?;
    let candidates = book.candidates_tx(conn, &denomination).await?;
    let fresh = allocate(&candidates, &ahead.remainder(&kept)?, context.at, pool)?;
    let held = kept.then(fresh.clone())?;
    book.record(Drawn {
        denomination,
        pool,
        fresh,
        held: held.clone(),
    })?;
    Ok(held)
}

/// What a draft's stored credit-priced lines show covered, drawn on the book as finalizing draws
/// them: the open settlements of their periods first, then fresh credit.
async fn estimate_draft_units(
    conn: &mut PgConn,
    book: &CreditBooks,
    invoice: &Invoice,
    at: DateTime<Utc>,
) -> StoreResult<()> {
    let overflow = || invalid("credit usage exceeds the supported range");
    let owners = LineOwners::load_tx(conn, invoice.tenant_id, &invoice.line_items).await?;
    // Per unit: what each subscription's lines show covered by the end of their dates, and the
    // periods they bill.
    type Shown = Vec<((SubscriptionId, DateTime<Utc>), i64)>;
    type Periods = Vec<(SubscriptionId, NaiveDate)>;
    let mut units: Vec<(String, Shown, Periods)> = Vec::new();
    for line in &invoice.line_items {
        let Some(block) = line.credit_unit.as_ref().filter(|block| block.covered > 0) else {
            continue;
        };
        let subscription_id = line_subscription(line, invoice.subscription_id, &owners)
            .ok_or_else(|| precondition("usage priced in a credit unit names no subscription"))?;
        let dated_before = line.usage_dated_before();
        let code = normalized_currency(&block.code);
        let index = match units.iter().position(|(unit, _, _)| *unit == code) {
            Some(index) => index,
            None => {
                units.push((code, Vec::new(), Vec::new()));
                units.len() - 1
            }
        };
        let (_, shown, periods) = &mut units[index];
        match shown
            .iter_mut()
            .find(|(key, _)| *key == (subscription_id, dated_before))
        {
            Some((_, covered)) => {
                *covered = covered.checked_add(block.covered).ok_or_else(overflow)?
            }
            None => shown.push(((subscription_id, dated_before), block.covered)),
        }
        periods.push((subscription_id, line.start_date));
    }
    for (code, shown, mut periods) in units {
        periods.sort_by_key(|(subscription_id, start)| (subscription_id.as_uuid(), *start));
        periods.dedup();
        let mut kept_settlements = Vec::new();
        for (subscription_id, period_start) in periods {
            for settlement in CreditUsageSettlementRow::list_for_period(
                conn,
                invoice.tenant_id,
                subscription_id,
                period_start,
                &code,
            )
            .await?
            {
                if settlement.invoice_id.is_none() || settlement.invoice_id == Some(invoice.id) {
                    kept_settlements.push(settlement.id.as_uuid());
                }
            }
        }
        let demands = shown
            .iter()
            .map(|((subscription_id, dated_before), covered)| CreditDemand {
                subscription_id: Some(*subscription_id),
                product_id: None,
                usage: true,
                amount_minor: *covered,
                dated_before: Some(*dated_before),
            })
            .collect::<Vec<_>>();
        let shown = CreditRequest {
            requested_minor: demands
                .iter()
                .try_fold(0_i64, |sum, demand| sum.checked_add(demand.amount_minor))
                .ok_or_else(overflow)?,
            demands,
        };
        // The settlements are placed as finalizing places them, then the rest is drawn.
        let held = settlement_holders_tx(conn, invoice.tenant_id, &kept_settlements).await?;
        let kept = assign(&held, &shown)?;
        let candidates = book.candidates_tx(conn, &code).await?;
        let fresh = allocate(&candidates, &shown.remainder(&kept)?, at, GrantPool::All)?;
        book.record(Drawn {
            denomination: code,
            pool: GrantPool::All,
            held: kept.then(fresh.clone())?,
            fresh,
        })?;
    }
    Ok(())
}

/// Drafts a forecast reads, in finalization order; past this many, later ones are left out.
const MAX_DRAFTS: i64 = 50;
/// Next invoices of other subscriptions estimated before the one asked for (or before an outlook's
/// horizon); past this many, the forecast is reported truncated.
const MAX_EARLIER_DOCUMENTS: usize = 50;
/// Next invoices a forecast of every subscription estimates, as many as the portal lists.
pub const MAX_FORECAST_SUBSCRIPTIONS: usize = 100;

/// Whether finalizing the draft may draw credit. A subscription's recurring or final draft is
/// priced again from current usage, so even one with nothing on it yet may.
pub(crate) fn draws_credit(invoice: &Invoice) -> bool {
    (invoice.invoice_type.bills_subscription_usage() && invoice.subscription_id.is_some())
        || invoice
            .line_items
            .iter()
            .any(|line| line.amount_subtotal > 0 || line.credit_unit.is_some())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForecastDocumentKind {
    Draft(InvoiceId),
    Upcoming(SubscriptionId),
}

/// Where a forecast stops: once a document is estimated, before the documents dated on or after a
/// day, or never.
#[derive(Debug, Clone, Copy)]
pub enum ForecastUntil {
    Document(ForecastDocumentKind),
    Before(NaiveDate),
    All,
}

#[derive(Debug, Clone)]
pub struct ForecastDocument {
    pub kind: ForecastDocumentKind,
    /// Several for a consolidated draft.
    pub subscription_ids: Vec<SubscriptionId>,
    pub invoice_date: NaiveDate,
    pub currency: String,
    /// With its credit already estimated on the book.
    pub content: ComputedInvoiceContent,
    pub drawn: Vec<Drawn>,
    /// The subscription a next invoice bills, as read for it.
    pub details: Option<Box<SubscriptionDetails>>,
    pub change: Option<UpcomingChange>,
}

#[derive(Debug)]
pub struct CustomerCreditForecast {
    pub documents: Vec<ForecastDocument>,
    /// Live subscriptions whose next invoice could not be computed (metering unavailable): the
    /// documents after them were estimated as if they drew nothing.
    pub unavailable: Vec<SubscriptionId>,
    pub books: Vec<CreditBook>,
    /// Another live subscription may draw customer-wide credit, before or after the documents,
    /// or some documents were left out.
    pub shared: bool,
    /// Documents billed before the last one were left out, past a bound or because they could not
    /// be estimated: what they draw is missing.
    pub truncated: bool,
}

impl CustomerCreditForecast {
    /// Whether the credit the document at `index` draws depends on other documents: it draws
    /// customer-wide credit that an earlier one draws too, or that documents not estimated may.
    pub(crate) fn estimate_shared(&self, labels: &GrantLabels, index: usize) -> bool {
        labels.draws_shared(&self.documents[index].drawn)
            && (self.shared
                || self.documents[..index]
                    .iter()
                    .any(|earlier| labels.draws_shared(&earlier.drawn)))
    }

    /// The documents billed before the one at `index` that draw customer-wide credit, one per
    /// subscription they bill.
    pub(crate) fn billed_first(&self, labels: &GrantLabels, index: usize) -> Vec<BilledFirst> {
        let mut billed_first = Vec::new();
        for earlier in &self.documents[..index] {
            if !labels.draws_shared(&earlier.drawn) {
                continue;
            }
            let invoice_id = match earlier.kind {
                ForecastDocumentKind::Draft(id) => Some(id),
                ForecastDocumentKind::Upcoming(_) => None,
            };
            let owners = if earlier.subscription_ids.is_empty() {
                vec![None]
            } else {
                earlier.subscription_ids.iter().copied().map(Some).collect()
            };
            for subscription_id in owners {
                billed_first.push(BilledFirst {
                    invoice_id,
                    subscription_id,
                    plan_name: subscription_id.and_then(|id| labels.plan_name(id)),
                    invoice_date: earlier.invoice_date,
                });
            }
        }
        billed_first
    }
}

/// A live subscription's place in billing order, read from its row alone: its period end, or the
/// earlier day a cancellation or a pause bills its final invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BillingSlot {
    pub subscription_id: SubscriptionId,
    pub invoice_date: NaiveDate,
}

impl BillingSlot {
    fn key(&self) -> (NaiveDate, uuid::Uuid) {
        (self.invoice_date, self.subscription_id.as_uuid())
    }
}

/// The live subscriptions billed before `until`, in billing order, at most `cap` of them; true
/// when some were left out.
pub(crate) fn billed_before(
    mut slots: Vec<BillingSlot>,
    until: ForecastUntil,
    target: Option<BillingSlot>,
    cap: usize,
) -> (Vec<BillingSlot>, bool) {
    slots.retain(|slot| Some(slot.subscription_id) != target.map(|t| t.subscription_id));
    slots.retain(|slot| match until {
        ForecastUntil::Document(ForecastDocumentKind::Upcoming(_)) => {
            target.is_some_and(|target| slot.key() < target.key())
        }
        ForecastUntil::Document(ForecastDocumentKind::Draft(_)) => false,
        ForecastUntil::Before(horizon) => slot.invoice_date < horizon,
        ForecastUntil::All => true,
    });
    slots.sort_by_key(BillingSlot::key);
    let capped = slots.len() > cap;
    if capped {
        slots.truncate(cap);
    }
    (slots, capped)
}

enum Pending {
    Draft(Box<Invoice>),
    Upcoming(Box<SubscriptionDetails>, Box<NextInvoiceBasis>),
}

impl Services {
    /// A draft as finalizing will bill it, its credit estimated on `book`: a subscription's draft
    /// priced again from current usage and unit coverage when `reprice`, any other as it stands.
    async fn estimate_draft_tx(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        book: &CreditBooks,
        at: DateTime<Utc>,
        reprice: bool,
    ) -> StoreResult<ComputedInvoiceContent> {
        let repriced =
            reprice && invoice.subscription_id.is_some() && invoice.manually_edited_at.is_none();
        let content = self
            .invoice_content_on(
                conn,
                invoice,
                &None,
                repriced,
                CreditUnitCover::Estimate {
                    invoice_id: Some(invoice.id),
                },
                Some(book),
            )
            .await?;
        if !(repriced && invoice.invoice_type.bills_subscription_usage()) {
            estimate_draft_units(conn, book, invoice, at).await?;
        }
        Ok(content)
    }

    /// `subject` is the subscription a `Document(Upcoming(_))` forecast is for, as the caller
    /// read it; it is forecast even when not live (a preview of a subscription about to start).
    /// Only the documents billed before it are read and estimated, at most a bounded number.
    pub(crate) async fn forecast_customer_credit(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        until: ForecastUntil,
        subject: Option<&SubscriptionDetails>,
        at: DateTime<Utc>,
    ) -> StoreResult<CustomerCreditForecast> {
        let rows =
            InvoiceRow::list_customer_drafts(conn, tenant_id, customer_id, MAX_DRAFTS + 1).await?;
        let mut truncated = rows.len() as i64 > MAX_DRAFTS;
        let drafts = rows
            .into_iter()
            .take(MAX_DRAFTS as usize)
            .map(Invoice::try_from)
            .filter(|invoice| !matches!(invoice, Ok(invoice) if !draws_credit(invoice)))
            .collect::<StoreResult<Vec<_>>>()?;
        let live = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id).await?;
        let stops = list_scheduled_stops(conn, tenant_id, customer_id).await?;
        let shared_credit =
            CustomerCreditGrantRow::list_available_for_customers(conn, tenant_id, &[customer_id])
                .await?
                .into_iter()
                .filter(|(scoped, _)| scoped.grant.subscription_id.is_none())
                .map(|(_, currency)| normalized_currency(&currency))
                .collect::<Vec<_>>();
        let target = match until {
            ForecastUntil::Document(ForecastDocumentKind::Upcoming(id)) => Some(id),
            _ => None,
        };
        let slots = live
            .iter()
            .filter_map(|row| {
                let period_end = row.current_period_end?;
                let stop = stops
                    .iter()
                    .filter(|(id, _)| *id == row.id)
                    .map(|(_, at)| at.date())
                    .filter(|day| *day <= period_end)
                    .min();
                Some(BillingSlot {
                    subscription_id: row.id,
                    invoice_date: stop.unwrap_or(period_end),
                })
            })
            .collect::<Vec<_>>();
        let slots_others = slots
            .iter()
            .filter(|slot| Some(slot.subscription_id) != target)
            .count();
        let others = drafts.len() + slots_others;
        // Nothing billed before the subscription can draw what it draws: it is estimated alone.
        let alone =
            target.is_some() && drafts.is_empty() && (others == 0 || shared_credit.is_empty());

        let mut unavailable = Vec::new();
        let mut pending: Vec<(ForecastDocumentKind, NaiveDate, Pending)> = Vec::new();
        if !alone {
            for draft in drafts {
                let kind = ForecastDocumentKind::Draft(draft.id);
                let last = matches!(until, ForecastUntil::Document(until) if until == kind);
                pending.push((kind, draft.invoice_date, Pending::Draft(Box::new(draft))));
                if last {
                    break;
                }
            }
        }
        let target_basis = match subject.filter(|subject| Some(subject.subscription.id) == target) {
            Some(subject) => Some((
                subject.clone(),
                self.next_invoice_basis(conn, subject).await?,
            )),
            None => None,
        };
        if !alone {
            let (earlier, capped) = billed_before(
                slots,
                until,
                target_basis.as_ref().map(|(details, basis)| BillingSlot {
                    subscription_id: details.subscription.id,
                    invoice_date: basis.invoice_date,
                }),
                match until {
                    ForecastUntil::All => MAX_FORECAST_SUBSCRIPTIONS,
                    ForecastUntil::Document(_) | ForecastUntil::Before(_) => MAX_EARLIER_DOCUMENTS,
                },
            );
            truncated |= capped;
            for slot in earlier {
                let subscription_id = slot.subscription_id;
                let loaded = match self
                    .store
                    .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                    .await
                {
                    Ok(details) => self
                        .next_invoice_basis(conn, &details)
                        .await
                        .map(|basis| (details, basis)),
                    Err(error) => Err(error),
                };
                match loaded {
                    Ok((details, basis)) => pending.push((
                        ForecastDocumentKind::Upcoming(subscription_id),
                        basis.invoice_date,
                        Pending::Upcoming(Box::new(details), Box::new(basis)),
                    )),
                    Err(error) => {
                        log::warn!(
                            "next invoice of subscription {subscription_id} left out of the credit forecast: {error:?}"
                        );
                        unavailable.push(subscription_id);
                    }
                }
            }
        }
        if let Some((details, basis)) = target_basis {
            pending.push((
                ForecastDocumentKind::Upcoming(details.subscription.id),
                basis.invoice_date,
                Pending::Upcoming(Box::new(details), Box::new(basis)),
            ));
        }
        if truncated {
            log::warn!(
                "credit forecast of customer {customer_id} left documents out past its bound"
            );
        }

        let book = CreditBooks::new(tenant_id, customer_id, at);
        let mut documents = Vec::new();
        for (kind, invoice_date, document) in pending {
            let estimated = match document {
                Pending::Draft(invoice) => {
                    let read = match self
                        .estimate_draft_tx(conn, &invoice, &book, at, true)
                        .await
                    {
                        Ok(content) => {
                            LineOwners::load_tx(conn, invoice.tenant_id, &content.invoice_lines)
                                .await
                                .map(|owners| (content, owners))
                        }
                        Err(error) => Err(error),
                    };
                    // Another subscription's draft that cannot be estimated is left out: what it
                    // draws is missing, so its subscription's figures and shared credit are not exact.
                    let (content, owners) = match read {
                        Ok(read) => read,
                        Err(error)
                            if !matches!(until, ForecastUntil::Document(until) if until == kind)
                                && invoice
                                    .subscription_id
                                    .is_some_and(|owner| target != Some(owner)) =>
                        {
                            log::warn!(
                                "draft {} left out of the credit forecast: {error:?}",
                                invoice.id
                            );
                            book.abandon_document()?;
                            truncated = true;
                            unavailable.extend(invoice.subscription_id);
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    let mut subscription_ids = content
                        .invoice_lines
                        .iter()
                        .filter_map(|line| {
                            line_subscription(line, invoice.subscription_id, &owners)
                        })
                        .chain(invoice.subscription_id)
                        .collect::<Vec<_>>();
                    subscription_ids.sort_by_key(|id| id.as_uuid());
                    subscription_ids.dedup();
                    ForecastDocument {
                        kind,
                        subscription_ids,
                        invoice_date,
                        currency: invoice.currency.clone(),
                        content,
                        drawn: Vec::new(),
                        details: None,
                        change: None,
                    }
                }
                Pending::Upcoming(details, basis) => {
                    let subscription_id = details.subscription.id;
                    // Other subscriptions only draw customer-wide credit before this one: once it
                    // is spent, they cannot change what this one draws.
                    if target.is_some_and(|target| target != subscription_id)
                        && !book.shared_left(&shared_credit)?
                    {
                        continue;
                    }
                    match self
                        .compute_next_invoice_on(conn, &basis, Some(&book))
                        .await
                    {
                        Ok(content) => ForecastDocument {
                            kind,
                            subscription_ids: vec![subscription_id],
                            invoice_date,
                            currency: details.subscription.currency.clone(),
                            content,
                            drawn: Vec::new(),
                            change: basis.change.clone(),
                            details: Some(details),
                        },
                        Err(error) if target != Some(subscription_id) => {
                            log::warn!(
                                "next invoice of subscription {subscription_id} left out of the credit forecast: {error:?}"
                            );
                            book.abandon_document()?;
                            unavailable.push(subscription_id);
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
            };
            let drawn = book.close_document()?;
            documents.push(ForecastDocument { drawn, ..estimated });
        }
        let unavailable_any = !unavailable.is_empty();
        Ok(CustomerCreditForecast {
            documents,
            unavailable,
            books: book.into_books()?,
            shared: !shared_credit.is_empty()
                && (truncated || slots_others > usize::from(target.is_none()) || unavailable_any),
            truncated,
        })
    }

    /// The customer's grants once every draft finalizing before `invoice` drew its own; None when
    /// none does, so the ledger stands. Past `MAX_DRAFTS`, earlier drafts are estimated as they
    /// stand rather than priced again from metering, so none is ever left out.
    pub(crate) async fn books_before_draft(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        at: DateTime<Utc>,
    ) -> StoreResult<Option<CreditBooks>> {
        // Earlier drafts only take credit away: without any left, their draws are all empty.
        if !CustomerCreditGrantRow::any_unspent_for_customer(
            conn,
            invoice.tenant_id,
            invoice.customer_id,
        )
        .await?
        {
            return Ok(None);
        }
        let drafts = InvoiceRow::list_customer_drafts_before(
            conn,
            invoice.tenant_id,
            invoice.customer_id,
            invoice.id,
        )
        .await?;
        let book = CreditBooks::new(invoice.tenant_id, invoice.customer_id, at);
        let mut earlier_drafts = 0_i64;
        for row in drafts {
            let earlier = Invoice::try_from(row)?;
            if !draws_credit(&earlier) {
                continue;
            }
            let reprice = earlier_drafts < MAX_DRAFTS;
            earlier_drafts += 1;
            self.estimate_draft_tx(conn, &earlier, &book, at, reprice)
                .await?;
            book.close_document()?;
        }
        Ok((earlier_drafts > 0).then_some(book))
    }

    /// A subscription's next invoice, with the credit it draws once every document billed before
    /// it drew its own, and the grants that pay it.
    pub(crate) async fn forecast_upcoming_invoice(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        at: DateTime<Utc>,
    ) -> StoreResult<UpcomingInvoiceForecast> {
        let subscription_id = details.subscription.id;
        let kind = ForecastDocumentKind::Upcoming(subscription_id);
        let forecast = self
            .forecast_customer_credit(
                conn,
                details.subscription.tenant_id,
                details.subscription.customer_id,
                ForecastUntil::Document(kind),
                Some(details),
                at,
            )
            .await?;
        let index = forecast
            .documents
            .iter()
            .position(|document| document.kind == kind)
            .ok_or_else(|| precondition("the subscription's next invoice was not forecast"))?;
        let labels = GrantLabels::load_tx(
            conn,
            details.subscription.tenant_id,
            &forecast.books,
            &forecast.documents[..=index],
        )
        .await?;
        let document = &forecast.documents[index];
        let credit = labels.deductions(&document.drawn)?;
        Ok(UpcomingInvoiceForecast {
            content: document.content.clone(),
            invoice_date: document.invoice_date,
            change: document.change.clone(),
            credit_estimate_shared: forecast.estimate_shared(&labels, index),
            billed_first: forecast.billed_first(&labels, index),
            credit,
        })
    }
}

/// The grants a forecast's documents drew, with the plan names their labels need.
pub(crate) struct GrantLabels {
    grants: HashMap<CreditGrantId, ScopedGrantRow>,
    plan_names: HashMap<SubscriptionId, String>,
    once: HashSet<CreditGrantId>,
}

impl GrantLabels {
    pub(crate) async fn load_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        books: &[CreditBook],
        documents: &[ForecastDocument],
    ) -> StoreResult<Self> {
        let mut grants = HashMap::new();
        let mut missing = Vec::new();
        for (grant_id, _) in documents
            .iter()
            .flat_map(|document| &document.drawn)
            .flat_map(|drawn| &drawn.held.draws)
        {
            match books.iter().find_map(|book| book.row(*grant_id)) {
                Some(row) => {
                    grants.insert(*grant_id, row.clone());
                }
                None => missing.push(*grant_id),
            }
        }
        missing.sort_by_key(|id| id.as_uuid());
        missing.dedup();
        for row in CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &missing).await? {
            grants.insert(row.grant.id, row);
        }
        let mut plan_names: HashMap<SubscriptionId, String> = documents
            .iter()
            .filter_map(|document| document.details.as_deref())
            .map(|details| {
                (
                    details.subscription.id,
                    details.subscription.plan_name.clone(),
                )
            })
            .collect();
        let mut unnamed = grants
            .values()
            .filter_map(|row| row.grant.subscription_id)
            .chain(
                documents
                    .iter()
                    .flat_map(|document| document.subscription_ids.iter().copied()),
            )
            .filter(|id| !plan_names.contains_key(id))
            .collect::<Vec<_>>();
        unnamed.sort_by_key(|id| id.as_uuid());
        unnamed.dedup();
        plan_names.extend(plan_names_tx(conn, tenant_id, &unnamed).await?);
        let once = once_grants_tx(conn, tenant_id, grants.values().map(|row| &row.grant)).await?;
        Ok(Self {
            grants,
            plan_names,
            once,
        })
    }

    fn plan_name(&self, subscription_id: SubscriptionId) -> Option<String> {
        self.plan_names.get(&subscription_id).cloned()
    }

    fn row(&self, grant_id: CreditGrantId) -> StoreResult<&ScopedGrantRow> {
        self.grants
            .get(&grant_id)
            .ok_or_else(|| precondition("a forecast drew a grant it cannot describe"))
    }

    /// Whether any of `drawn` came from credit any of the customer's subscriptions may use.
    pub(crate) fn draws_shared(&self, drawn: &[Drawn]) -> bool {
        drawn
            .iter()
            .flat_map(|drawn| &drawn.held.draws)
            .any(|(grant_id, amount)| {
                *amount > 0
                    && self
                        .grants
                        .get(grant_id)
                        .is_some_and(|row| row.grant.subscription_id.is_none())
            })
    }

    pub(crate) fn deductions(&self, drawn: &[Drawn]) -> StoreResult<Vec<CreditDeduction>> {
        let mut deductions = Vec::new();
        for drawn in drawn {
            for (grant_id, amount) in drawn.held.draws.iter().filter(|(_, amount)| *amount > 0) {
                let row = self.row(*grant_id)?;
                deductions.push(CreditDeduction {
                    grant_id: *grant_id,
                    denomination: drawn.denomination.clone(),
                    group: row_group(
                        row,
                        &drawn.denomination,
                        self.once.contains(grant_id),
                        |id| self.plan_name(id),
                    )?,
                    before_tax: drawn.pool == GrantPool::Promotional,
                    amount_minor: *amount,
                });
            }
        }
        Ok(deductions)
    }
}

pub async fn plan_names_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_ids: &[SubscriptionId],
) -> StoreResult<HashMap<SubscriptionId, String>> {
    if subscription_ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(
        SubscriptionRow::list_subscriptions_by_ids(conn, &tenant_id, subscription_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|row| (row.subscription.id, row.plan_name))
            .collect(),
    )
}

/// The included grants among `grants` that a one-time (welcome) credit funded, read from the line
/// that minted each: its component or add-on grants its credit once.
pub async fn once_grants_tx<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    grants: impl IntoIterator<Item = &'a CustomerCreditGrantRow>,
) -> StoreResult<HashSet<CreditGrantId>> {
    let included = grants
        .into_iter()
        .filter(|grant| grant.source_kind == source::INCLUDED_CREDIT)
        .filter_map(|grant| Some((grant.id, grant.invoice_id?, grant.line_local_id.clone()?)))
        .collect::<Vec<_>>();
    if included.is_empty() {
        return Ok(HashSet::new());
    }
    let mut invoice_ids = included.iter().map(|(_, id, _)| *id).collect::<Vec<_>>();
    invoice_ids.sort_by_key(|id| id.as_uuid());
    invoice_ids.dedup();
    let mut lines: HashMap<InvoiceId, Vec<LineItem>> = HashMap::new();
    for invoice in InvoiceRow::list_by_ids(conn, invoice_ids).await? {
        if invoice.tenant_id == tenant_id {
            lines.insert(
                invoice.id,
                serde_json::from_value(invoice.line_items)
                    .map_err(|_| invalid("an invoice's lines are unreadable"))?,
            );
        }
    }
    let minted_by = included
        .iter()
        .filter_map(|(grant_id, invoice_id, local_id)| {
            let line = lines
                .get(invoice_id)?
                .iter()
                .find(|line| line.local_id == *local_id)?;
            Some((*grant_id, line.sub_component_id, line.sub_add_on_id))
        })
        .collect::<Vec<_>>();
    let components = minted_by
        .iter()
        .filter_map(|(_, component, _)| *component)
        .collect::<Vec<_>>();
    let add_ons = minted_by
        .iter()
        .filter_map(|(_, _, add_on)| *add_on)
        .collect::<Vec<_>>();
    let (components, add_ons) =
        list_once_credit_owners(conn, tenant_id, &components, &add_ons).await?;
    Ok(minted_by
        .into_iter()
        .filter(|(_, component, add_on)| {
            component.is_some_and(|id| components.contains(&id))
                || add_on.is_some_and(|id| add_ons.contains(&id))
        })
        .map(|(grant_id, _, _)| grant_id)
        .collect())
}

/// The group a stored grant is shown in; `once` when a one-time credit funded it.
pub fn row_group(
    row: &ScopedGrantRow,
    denomination: &str,
    once: bool,
    plan_name: impl Fn(SubscriptionId) -> Option<String>,
) -> StoreResult<CreditGroup> {
    let grant = &row.grant;
    let classification = GrantClassification::from_db_str(&grant.classification)?;
    let scope = grant_scope(grant, &row.product_ids)?;
    Ok(credit_group(
        CreditGrantSource::from_db_str(&grant.source_kind)?,
        classification,
        once || (classification == GrantClassification::Promotional
            && grant.reason == BONUS_CREDIT),
        &scope,
        scope.subscription_id.and_then(plan_name).as_deref(),
        crate::domain::credit_units::DenominationCode::parse(&normalized_currency(denomination))
            .is_ok_and(|code| code.is_unit()),
    ))
}

/// How credit is shown, mirroring `credit_groups` (P4): plan credit as included with its plan,
/// bought credit as prepaid, given credit (a purchase's bonus, a welcome credit: `bonus`) as
/// bonus, and money that pays any charge as the account balance.
pub(crate) fn credit_group(
    source: CreditGrantSource,
    classification: GrantClassification,
    bonus: bool,
    scope: &GrantScope,
    plan_name: Option<&str>,
    credit_unit: bool,
) -> CreditGroup {
    let any_charge = !credit_unit
        && scope.subscription_id.is_none()
        && scope.application == CreditApplication::Invoice
        && scope.product_ids.is_empty();
    let kind = match source {
        _ if bonus => CreditGroupKind::Bonus,
        CreditGrantSource::Included | CreditGrantSource::Rollover => CreditGroupKind::Included,
        CreditGrantSource::Coupon => CreditGroupKind::Bonus,
        CreditGrantSource::Purchase => CreditGroupKind::Prepaid,
        CreditGrantSource::Manual if any_charge => CreditGroupKind::AccountBalance,
        CreditGrantSource::Manual => match classification {
            GrantClassification::Paid => CreditGroupKind::Prepaid,
            GrantClassification::Promotional => CreditGroupKind::Bonus,
        },
        CreditGrantSource::CustomerBalance
        | CreditGrantSource::CreditNote
        | CreditGrantSource::Overpayment
        | CreditGrantSource::NegativeInvoice
        | CreditGrantSource::InvoiceVoid
        | CreditGrantSource::UsageSettlementReturn => CreditGroupKind::AccountBalance,
    };
    let label = match kind {
        CreditGroupKind::Included => format!("Included with {}", plan_name.unwrap_or("plan")),
        CreditGroupKind::Prepaid => PURCHASED_CREDIT.to_owned(),
        CreditGroupKind::Bonus => FREE_CREDIT.to_owned(),
        CreditGroupKind::AccountBalance => ACCOUNT_BALANCE.to_owned(),
    };
    let plan = scope
        .subscription_id
        .map(|_| plan_name.unwrap_or("its subscription's").to_owned());
    let pays_for = match (
        plan,
        scope.application == CreditApplication::Usage || credit_unit,
    ) {
        (Some(plan), true) => format!("Pays {plan} usage"),
        (Some(plan), false) => format!("Pays {plan} charges"),
        (None, true) => "Pays usage on any subscription".to_owned(),
        (None, false) => "Pays any invoice".to_owned(),
    };
    CreditGroup {
        kind,
        label,
        subscription_id: scope.subscription_id,
        pays_for,
    }
}

impl ServicesEdge {
    /// The subscription's next invoice (with the changes scheduled by its date), its credit drawn
    /// in billing order across the customer's pending documents.
    pub async fn upcoming_invoice_forecast(
        &self,
        details: &SubscriptionDetails,
    ) -> StoreResult<UpcomingInvoiceForecast> {
        let mut conn = self.store.get_conn().await?;
        self.services
            .forecast_upcoming_invoice(&mut conn, details, Utc::now())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::credit::CreditApplication;

    fn scope(subscription: bool, application: CreditApplication) -> GrantScope {
        GrantScope {
            subscription_id: subscription.then(SubscriptionId::new),
            application,
            product_ids: Vec::new(),
        }
    }

    #[test]
    fn credit_is_grouped_by_what_it_is_and_pays_for() {
        use CreditApplication::{Invoice, Usage};
        use GrantClassification::{Paid, Promotional};
        let group = |source, classification, bonus, scope: &GrantScope| {
            let group = credit_group(source, classification, bonus, scope, Some("Scale"), false);
            (group.kind, group.label, group.pays_for)
        };
        let pack = scope(true, Usage);
        assert_eq!(
            group(CreditGrantSource::Included, Promotional, false, &pack),
            (
                CreditGroupKind::Included,
                "Included with Scale".to_owned(),
                "Pays Scale usage".to_owned()
            )
        );
        assert_eq!(
            group(CreditGrantSource::Included, Promotional, true, &pack).0,
            CreditGroupKind::Bonus,
            "a welcome credit"
        );
        assert_eq!(
            group(CreditGrantSource::Purchase, Promotional, true, &pack).1,
            "Free credit"
        );
        assert_eq!(
            group(CreditGrantSource::Purchase, Paid, false, &pack).0,
            CreditGroupKind::Prepaid
        );
        let shared = scope(false, Invoice);
        assert_eq!(
            group(CreditGrantSource::Coupon, Promotional, false, &shared),
            (
                CreditGroupKind::Bonus,
                "Free credit".to_owned(),
                "Pays any invoice".to_owned()
            )
        );
        for source in [
            CreditGrantSource::CreditNote,
            CreditGrantSource::CustomerBalance,
            CreditGrantSource::Manual,
        ] {
            assert_eq!(
                group(source, Paid, false, &shared).0,
                CreditGroupKind::AccountBalance
            );
        }
        assert_eq!(
            credit_group(
                CreditGrantSource::Manual,
                Promotional,
                false,
                &shared,
                None,
                true
            )
            .kind,
            CreditGroupKind::Bonus,
            "credit in a unit is not money"
        );
        assert_eq!(
            group(CreditGrantSource::Manual, Paid, false, &scope(false, Usage)),
            (
                CreditGroupKind::Prepaid,
                "Purchased credit".to_owned(),
                "Pays usage on any subscription".to_owned()
            )
        );
    }

    /// However many subscriptions the customer has, a forecast reads and estimates at most a
    /// bounded number billed before the one asked for (or at all), in billing order.
    #[test]
    fn a_forecast_reads_a_bounded_number_of_earlier_documents() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 3, d).unwrap();
        let slots = (0..10_000)
            .map(|n| BillingSlot {
                subscription_id: SubscriptionId::new(),
                invoice_date: day(1 + (n % 28) as u32),
            })
            .collect::<Vec<_>>();
        let target = BillingSlot {
            subscription_id: SubscriptionId::new(),
            invoice_date: day(20),
        };
        let until = ForecastUntil::Document(ForecastDocumentKind::Upcoming(target.subscription_id));
        let (earlier, capped) = billed_before(slots.clone(), until, Some(target), 50);
        assert_eq!(earlier.len(), 50);
        assert!(capped);
        assert!(
            earlier
                .windows(2)
                .all(|pair| pair[0].key() <= pair[1].key())
        );
        assert!(earlier.iter().all(|slot| slot.key() < target.key()));

        let (all, capped) = billed_before(
            slots.clone(),
            ForecastUntil::All,
            None,
            MAX_FORECAST_SUBSCRIPTIONS,
        );
        assert_eq!(
            all.len(),
            100,
            "a forecast of every subscription is bounded too"
        );
        assert!(capped);
        assert!(all.windows(2).all(|pair| pair[0].key() <= pair[1].key()));

        let (before, _) = billed_before(slots.clone(), ForecastUntil::Before(day(2)), None, 50);
        assert!(before.iter().all(|slot| slot.invoice_date < day(2)));
        let (few, capped) = billed_before(slots[..3].to_vec(), until, Some(target), 50);
        assert!(!capped);
        assert!(few.len() <= 3);
    }

    #[test]
    fn a_book_spends_what_documents_drew_and_never_below_zero() {
        use chrono::TimeZone;
        let at = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        let grant = GrantCandidate {
            id: CreditGrantId::new(),
            scope: scope(false, CreditApplication::Invoice),
            remaining_minor: 1_000,
            priority: 100,
            classification: GrantClassification::Paid,
            effective_at: at,
            pays_usage_from: at,
            lapses_at: None,
            created_at: at,
        };
        let mut book = CreditBook {
            denomination: "USD".to_owned(),
            grants: Vec::new(),
        };
        let row = |id: CreditGrantId| ScopedGrantRow {
            grant: CustomerCreditGrantRow {
                id,
                ..test_grant_row(at)
            },
            product_ids: Vec::new(),
        };
        book.grants.push((row(grant.id), grant.clone()));
        let drew = |amount| Allocation {
            draws: vec![(grant.id, amount)],
            covered: Vec::new(),
            placed: Vec::new(),
        };
        book.spend(&drew(700)).unwrap();
        assert_eq!(book.candidates()[0].remaining_minor, 300);
        assert!(book.spend(&drew(301)).is_err());
        assert!(
            book.spend(&Allocation {
                draws: vec![(CreditGrantId::new(), 1)],
                covered: Vec::new(),
                placed: Vec::new(),
            })
            .is_err()
        );
    }

    fn test_grant_row(at: DateTime<Utc>) -> CustomerCreditGrantRow {
        CustomerCreditGrantRow {
            id: CreditGrantId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            account_id: common_domain::ids::WalletId::new(),
            amount: 1_000,
            remaining: 1_000,
            priority: 100,
            classification: "PAID".to_owned(),
            effective_at: at,
            expires_at: None,
            source_kind: "MANUAL".to_owned(),
            source_id: uuid::Uuid::now_v7(),
            invoice_id: None,
            line_local_id: None,
            credit_note_id: None,
            reason: String::new(),
            reference: None,
            created_by: None,
            created_at: at,
            subscription_id: None,
            application: "INVOICE".to_owned(),
            ends_at: None,
            end_reason: None,
            lapses_at: None,
            cost_minor: None,
            cost_currency: None,
            rollover: None,
            pays_usage_from: None,
        }
    }
}

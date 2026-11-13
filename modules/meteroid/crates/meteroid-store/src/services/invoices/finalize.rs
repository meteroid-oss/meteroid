use crate::StoreResult;
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{
    CouponLineItem, DetailedInvoice, Invoice, InvoiceLinesPatch, LineItem, SubscriptionDetails,
};
use crate::errors::StoreError;
use crate::repositories::credit::allocation::{Allocation, CreditRequest, GrantPool};
use crate::repositories::credit::customer_credit as credit;
use crate::repositories::credit::line_allocation;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::Services;
use crate::services::credit::promotional::{LineScope, TaxedDocument};
use crate::services::credit::scope::LineOwners;
use crate::services::credit::{billing_windows, expiry_settlement, metrics, value_application};
use crate::services::invoice_lines::component::PinnedPrices;
use crate::services::invoice_lines::credit_units::CreditUnitCover;
use crate::services::invoice_lines::invoice_lines::BilledSpan;
use crate::services::utils::format_document_number;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{AppliedCouponId, BaseId, InvoiceId, SubscriptionId, TenantId};
use common_utils::decimals::ToUnit;
use diesel_models::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRow};
use diesel_models::customers::CustomerRow;
use diesel_models::invoices::{InvoiceRow, InvoiceRowIdentityPatch, InvoiceRowLinesPatch};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::lock::CustomerLock;
use diesel_models::{DbResult, PgConn};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// When a finalization issues its invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAt {
    /// Today; an invoice of a backdated subscription is issued on its stored date (at noon), and
    /// a manual invoice keeps its operator-chosen dates.
    Now,
    /// At this past moment: dated then (a manual invoice keeps its dates) and paid from the credit
    /// as it stood then.
    At(DateTime<Utc>),
}

/// Whether an issued invoice is collected and sent to the customer. A replay of past billing
/// suppresses both: its invoices record history, they ask for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceDelivery {
    Normal,
    Suppressed,
}

impl InvoiceDelivery {
    pub(crate) async fn apply_tx(
        self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        if self == Self::Suppressed {
            diesel_models::replay_holds::hold_invoice_delivery(conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }
        Ok(())
    }
}

/// The issue moment a finalization resolved, once, before anything is drawn.
#[derive(Debug, Clone, Copy)]
struct Issue {
    finalized_at: NaiveDateTime,
    /// The day the invoice is dated, its due date rebased on it; None keeps the stored dates.
    issued_on: Option<NaiveDate>,
    /// When credit is drawn and expiries are due: the issue moment, never later than now.
    credit_at: DateTime<Utc>,
    /// The present: the clock, or the explicit moment a replay issues at.
    now: DateTime<Utc>,
}

impl Issue {
    /// `stored_date` is the draft's invoice date; `manual` invoices keep theirs.
    fn resolve(
        issue: IssueAt,
        stored_date: NaiveDate,
        manual: bool,
        backdated: bool,
        now: DateTime<Utc>,
    ) -> StoreResult<Self> {
        let (at, issued_on) = match issue {
            IssueAt::At(at) if at > now => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "an invoice cannot be issued at a future moment".to_owned(),
                )));
            }
            IssueAt::At(at) => (at, (!manual).then(|| at.date_naive())),
            IssueAt::Now if backdated => (
                stored_date
                    .and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(NaiveTime::MIN))
                    .and_utc(),
                None,
            ),
            IssueAt::Now if manual => (now, None),
            IssueAt::Now => (now, Some(now.date_naive())),
        };
        Ok(Self {
            finalized_at: at.naive_utc(),
            issued_on,
            credit_at: at.min(now),
            now: match issue {
                IssueAt::At(at) => at,
                IssueAt::Now => now,
            },
        })
    }

    fn credit_time(&self) -> credit::MovementTime {
        credit::MovementTime {
            at: self.credit_at,
            now: self.now,
        }
    }
}

impl Services {
    /// Operator / API finalize of a single invoice (the gRPC `FinalizeInvoice` action).
    ///
    /// A recurring subscription period draft is settled exactly like the automated grace-period
    /// finalize — see [`Services::settle_recurring_invoice_tx`]. Every other invoice keeps its
    /// stored lines and can never be `Close`d.
    ///
    /// Finalizing during the grace window deliberately gives up the days still left in it: the
    /// recompute re-queries usage here, so the period settles against everything ingested up to
    /// this call and forfeits only what arrives after.
    ///
    /// The exception is a draft whose billing content an operator edited (`manually_edited_at`):
    /// finalizing must issue exactly what they previewed. A refresh would regenerate the usage
    /// lines and derive the total from coupons alone, dropping a manually set discount from the
    /// total while the discount column keeps it — an invoice that doesn't reconcile.
    pub async fn finalize_invoice(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.finalize_invoice_at(actor, id, tenant_id, IssueAt::Now, InvoiceDelivery::Normal)
            .await
    }

    pub async fn finalize_invoice_at(
        &self,
        actor: Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        issue: IssueAt,
        delivery: InvoiceDelivery,
    ) -> StoreResult<DetailedInvoice> {
        // Usage is read before the invoice is locked, once this call began: the period settles on
        // what was ingested by then, as it would under the lock a moment later.
        let mut conn = self.store.get_conn().await?;
        let actor = &actor;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                // Lock before classifying so this races neither a concurrent draft edit nor a
                // concurrent finalize; the helpers below re-lock re-entrantly.
                let invoice: Invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, id)
                    .await?
                    .invoice
                    .try_into()?;

                delivery.apply_tx(conn, tenant_id, id).await?;
                if invoice.invoice_type.bills_subscription_usage()
                    && invoice.subscription_id.is_some()
                    && invoice.manually_edited_at.is_none()
                {
                    services
                        .settle_recurring_invoice_at_tx(
                            conn, actor, id, tenant_id, true, &None, issue,
                        )
                        .await
                } else {
                    services
                        .finalize_invoice_at_tx(conn, actor, id, tenant_id, false, &None, issue)
                        .await
                }
            }
            .scope_boxed()
        })
        .await
    }

    /// Finalize an invoice from its stored lines, or recompute them first when
    /// `refresh_invoice_lines` is set. Never `Close`s: an empty recurring draft becomes a €0
    /// invoice. The safe default for every finalize that is not a period settlement.
    pub async fn finalize_invoice_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
    ) -> StoreResult<DetailedInvoice> {
        self.finalize_invoice_at_tx(
            conn,
            actor,
            id,
            tenant_id,
            refresh_invoice_lines,
            subscription_details_for_refresh,
            IssueAt::Now,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_invoice_at_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
        issue: IssueAt,
    ) -> StoreResult<DetailedInvoice> {
        self.finalize_or_settle_invoice_tx(
            conn,
            actor,
            id,
            tenant_id,
            refresh_invoice_lines,
            false,
            subscription_details_for_refresh,
            issue,
        )
        .await
    }

    /// Settle a subscription period: finalize it, or terminally `Close` it when the period has
    /// nothing to bill (no number assigned, so no gap in the sequence and no €0 facture).
    ///
    /// The only entry point that may `Close`. A `Close` is irreversible — never voided, never
    /// re-billed — so callers MUST pass lines reflecting the period's *current* usage: either
    /// `refresh_invoice_lines = true`, or lines assembled fresh in this same transaction (the
    /// consolidated parent). Called with an arbitrary invoice's stale stored lines, it would
    /// silently discard a settled period's usage.
    pub async fn settle_recurring_invoice_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
    ) -> StoreResult<DetailedInvoice> {
        self.settle_recurring_invoice_at_tx(
            conn,
            actor,
            id,
            tenant_id,
            refresh_invoice_lines,
            subscription_details_for_refresh,
            IssueAt::Now,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn settle_recurring_invoice_at_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
        issue: IssueAt,
    ) -> StoreResult<DetailedInvoice> {
        self.finalize_or_settle_invoice_tx(
            conn,
            actor,
            id,
            tenant_id,
            refresh_invoice_lines,
            true,
            subscription_details_for_refresh,
            issue,
        )
        .await
    }

    /// Resolves the identity one last time and writes it to the row. Called at every terminal
    /// transition out of `Draft`: past that point the read paths stop overlaying, so whatever is
    /// stored is what shows — and for an issued document it must never move again. An overridden
    /// draft keeps the operator's edit. Pass `seller` when the caller already holds the entity.
    async fn freeze_invoice_identity(
        &self,
        tx: &mut PgConn,
        invoice: &Invoice,
        seller: Option<&crate::domain::InvoicingEntity>,
    ) -> StoreResult<()> {
        if !invoice.tracks_live_customer() && !invoice.tracks_live_seller() {
            return Ok(());
        }

        let tenant_id = invoice.tenant_id;
        let mut frozen = invoice.clone();

        if invoice.tracks_live_customer() {
            let customer: crate::domain::Customer =
                CustomerRow::find_by_id(tx, &invoice.customer_id, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(std::convert::TryInto::try_into)?;
            frozen.overlay_live_customer(&customer);
        }

        if invoice.tracks_live_seller() {
            match seller {
                Some(seller) => frozen.overlay_live_seller(seller),
                None => {
                    let row = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                        tx,
                        invoice.invoicing_entity_id,
                        tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    frozen.overlay_live_seller(&row.into());
                }
            }
        }

        InvoiceRowIdentityPatch {
            customer_details: serde_json::to_value(&frozen.customer_details).map_err(|e| {
                StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
            })?,
            seller_details: serde_json::to_value(&frozen.seller_details).map_err(|e| {
                StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
            })?,
        }
        .update_identity(tx, invoice.id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    /// Shared body for [`Services::finalize_invoice_tx`] and
    /// [`Services::settle_recurring_invoice_tx`]: increments the invoice number counter and applies
    /// attached coupons. `may_close_empty_recurring` gates the terminal-`Close` path.
    #[allow(clippy::too_many_arguments)]
    async fn finalize_or_settle_invoice_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        may_close_empty_recurring: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
        issue: IssueAt,
    ) -> StoreResult<DetailedInvoice> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let invoice_lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, id).await?;
            let lock = &invoice_lock.lock;
            diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
            let invoice: Invoice = invoice_lock.invoice.try_into()?;
            let stored_lines_empty = invoice.line_items.is_empty();

            // A consolidated child is billed via its parent; finalizing it too would double-bill.
            invoice.ensure_not_consolidated_child("finalize")?;

            // Finalization is reachable concurrently from checkout acceptance, the settlement
            // handler and scheduled events. Re-running it on an already-finalized invoice would
            // rewrite its lines (`update_lines` has no status filter), re-apply credit/balance
            // deductions, burn an invoice number and emit a duplicate `invoice.finalized`.
            // The FOR UPDATE above serializes racing finalizers onto this check.
            if !invoice.can_edit() {
                return InvoiceRow::find_detailed_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::into)
                    .and_then(std::convert::TryInto::try_into);
            }

            // Settle's freshness contract, enforced in every build: recomputed here, or a consolidated
            // parent's lines assembled fresh in this transaction. Settling a subscription period from
            // stale stored lines silently discards that period's usage, so it must never run.
            if may_close_empty_recurring
                && !refresh_invoice_lines
                && invoice.subscription_id.is_some()
            {
                return Err(
                Report::new(StoreError::InvoiceComputationError).attach(format!(
                    "settle_recurring_invoice_tx called with stale stored lines for invoice {id}"
                )),
            );
            }

            // Propagate errors: this gates emission-date stamping, so a swallowed failure would
            // mis-date an imported invoice.
            let backdate_invoices = if let Some(subscription_id) = invoice.subscription_id {
                InvoiceRow::get_subscription_backdate_flag(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            } else {
                false
            };
            let issue = Issue::resolve(
                issue,
                invoice.invoice_date,
                invoice.manual,
                backdate_invoices,
                common_domain::clock::now(),
            )?;

            if refresh_invoice_lines && let Some(subscription_id) = invoice.subscription_id {
                let subscription =
                    diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
                        conn,
                        &tenant_id,
                        subscription_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                crate::services::invoice_lines::cadence::record_cadence_origins_tx(
                    conn,
                    lock,
                    crate::services::invoice_lines::cadence::CadenceSubject::of_row(
                        &subscription.subscription,
                    ),
                    invoice.invoice_date,
                )
                .await?;
            }

            // Credit-priced usage draws its unit wallets here, under the invoice lock and before
            // tax: only what they leave uncovered becomes money.
            let mut patch = self
                .build_invoice_lines_patch(
                    conn,
                    &invoice,
                    subscription_details_for_refresh,
                    refresh_invoice_lines,
                    CreditUnitCover::Draw {
                        invoice_id: invoice.id,
                        at: issue.credit_time(),
                    },
                )
                .await?;
            // Metering read before the lock is served only for the query it answered: a pass that
            // asked for any other (inputs changed since) is priced again before anything is drawn.
            self.ensure_usage_served()?;
            let applied_coupons_amounts = patch.applied_coupons.clone();
            let refreshed_lines_empty = patch.line_items.is_empty();

            // Only an already-empty draft may Close; lines that recomputed away mean the recompute
            // disagrees with what was billed (metering outage), and Close has no inverse — so refuse.
            if refreshed_lines_empty
                && !stored_lines_empty
                && invoice.invoice_type.bills_subscription_usage()
                && may_close_empty_recurring
            {
                return Err(
                Report::new(StoreError::InvoiceComputationError).attach(format!(
                    "Refusing to close invoice {id}: it has stored lines but recomputed to nothing"
                )),
            );
            }

            let will_close = refreshed_lines_empty
                && invoice.invoice_type.bills_subscription_usage()
                && may_close_empty_recurring;
            if !will_close {
                self.apply_customer_credit_tx(
                    conn,
                    lock,
                    &invoice,
                    &mut patch,
                    refresh_invoice_lines,
                    issue.credit_time(),
                )
                .await?;
            }

            let row_patch: InvoiceRowLinesPatch = patch.try_into()?;

            row_patch
                .update_lines(id, tenant_id, conn)
                .await
                .map(|_| ())
                .map_err(Into::<Report<StoreError>>::into)?;

            // A recurring period (or final window) that settled with nothing to bill is Closed rather
            // than finalized. Only Recurring and Final invoices are period-anchored; `may_close_empty_recurring` restricts the
            // transition to callers holding a fresh view of the period (see settle_recurring_invoice_tx).
            if will_close {
                // Close is terminal too, so freeze before leaving Draft — otherwise the row keeps
                // whatever stale snapshot it happened to hold, with nothing left to overlay it.
                self.freeze_invoice_identity(conn, &invoice, None).await?;

                InvoiceRow::close(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                log::info!(
                    "Closed empty recurring invoice {} (nothing to bill at end of grace period).",
                    id
                );

                // `close` updates exactly one Draft row or errors, so the re-read is authoritative.
                let closed: DetailedInvoice = InvoiceRow::find_detailed_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::into)
                    .and_then(std::convert::TryInto::try_into)?;

                // Terminal, non-revenue counterpart to the earlier `invoice.created`.
                let invoice_event = (&closed.invoice).into();
                self.store
                    .internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::invoice_closed(invoice_event)],
                    )
                    .await?;

                return Ok(closed);
            }

            // A negative total (e.g. a downgrade adjustment) is owed to the customer as credit in
            // the invoice currency.
            if row_patch.total < 0 {
                let owed = row_patch.total.checked_neg().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "negative invoice total overflow".into(),
                    ))
                })?;
                credit::fund_tx(
                    conn,
                    &*self.store.hooks.credit,
                    credit::CreditFunding {
                        tenant_id,
                        customer_id: invoice.customer_id,
                        currency: invoice.currency.clone(),
                        scope: GrantScope::customer_wide(),
                        amount_minor: owed,
                        classification: GrantClassification::Paid,
                        priority: credit::DEFAULT_PRIORITY,
                        effective_at: issue.credit_at,
                        expires_at: None,
                        occurred_at: issue.credit_at,
                        now: issue.now,
                        source_kind: credit::source::NEGATIVE_INVOICE,
                        source_id: invoice.id.as_uuid(),
                        cost: Some(CreditCost::at_face(owed, &invoice.currency)),
                        rollover: None,
                        invoice_id: Some(invoice.id),
                        line_local_id: None,
                        credit_note_id: None,
                        reason: "Credit from a negative-total invoice".to_owned(),
                        reference: None,
                        created_by: actor.as_uuid(),
                        pays_usage_from: None,
                    },
                )
                .await?;
            }

            let invoice_details = self
                .increment_and_finalize(conn, lock, actor, invoice, applied_coupons_amounts, &issue)
                .await?;

            Ok(invoice_details)
        })
        .await
    }

    /// Draws the invoice's customer credit: promotional credit before tax, re-taxing the lines
    /// whose share moved since the estimate, then paid credit on the unpaid remainder of the total
    /// (after cash already committed). A replay reuses the credit the invoice already drew.
    pub(super) async fn apply_customer_credit_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice: &Invoice,
        patch: &mut InvoiceLinesPatch,
        refreshed: bool,
        time: credit::MovementTime,
    ) -> StoreResult<()> {
        let already_paid = patch
            .total
            .saturating_sub(patch.amount_due)
            .saturating_sub(patch.applied_credits)
            .max(0);
        let committed =
            value_application::committed_cash_minor(conn, invoice.tenant_id, invoice.id).await?;
        let billing = self
            .credit_billing_tx(conn, lock, invoice, patch, refreshed, time)
            .await?;

        let discount = patch.subtotal - patch.promotional_credits + patch.tax_amount - patch.total;
        let owners = LineOwners::load_tx(conn, invoice.tenant_id, &patch.line_items).await?;
        let line_scope = LineScope {
            owners: &owners,
            subscription_id: invoice.subscription_id,
        };
        let mut document = TaxedDocument {
            lines: &mut patch.line_items,
            subtotal: &mut patch.subtotal,
            subtotal_recurring: &mut patch.subtotal_recurring,
            discount,
            tax_inclusive: invoice.tax_inclusive,
            tax_amount: &mut patch.tax_amount,
            tax_breakdown: &mut patch.tax_breakdown,
            promotional_credits: &mut patch.promotional_credits,
            total: &mut patch.total,
        };
        // Cash taken against the draft as shown never ends up paying for more than it.
        let (request, of_line) = if already_paid > 0 || committed > 0 {
            let shown: std::collections::HashMap<&str, i64> = invoice
                .line_items
                .iter()
                .map(|line| (line.local_id.as_str(), line.promotional_credit.unwrap_or(0)))
                .collect();
            document.pretax_request_within(&shown, line_scope)?
        } else {
            document.pretax_request(line_scope)?
        };
        let charges = document.usage_charges(&request, &of_line);
        let promotional = self
            .draw_credit_tx(
                conn,
                lock,
                invoice,
                &billing,
                GrantPool::Promotional,
                &request,
                &charges,
                time,
            )
            .await?;
        let shares = document.shares(&of_line, &promotional);
        document.reprice(&shares)?;

        patch.applied_credits = 0;
        patch.amount_due = billing_core::amount_due(patch.total, already_paid, 0);
        let (request, of_line) = value_application::credit_request_of_lines(
            &patch.line_items,
            &owners,
            invoice.subscription_id,
            patch.total,
            already_paid,
        )?;
        let request = request.capped(patch.amount_due.saturating_sub(committed));
        let paid = self
            .draw_credit_tx(
                conn,
                lock,
                invoice,
                &billing,
                GrantPool::Paid,
                &request,
                &[],
                time,
            )
            .await?;
        line_allocation::record_line_allocation_tx(
            conn,
            invoice.tenant_id,
            invoice.id,
            None,
            &patch.line_items,
            &of_line,
            &request,
            &paid,
            |line| line.amount_total,
        )
        .await?;
        value_application::record_applied_credit(patch, paid.total()?)
    }

    /// Readies the credit draws of an invoice. One that bills metered usage takes the customer's
    /// grant locks first, before looking at any settlement: credit that expired mid-period pays
    /// that usage under those locks, once, whether the expiry sweep or this invoice comes first.
    async fn credit_billing_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice: &Invoice,
        patch: &InvoiceLinesPatch,
        refreshed: bool,
        time: credit::MovementTime,
    ) -> StoreResult<CreditBilling> {
        if !invoice.invoice_type.bills_subscription_usage() {
            return Ok(CreditBilling::Plain);
        }
        let children: Vec<Invoice>;
        let billed = match invoice.subscription_id {
            Some(subscription_id) => vec![BilledInvoice {
                subscription_id,
                anchor: invoice.billing_period_start,
                cycle_index: invoice.cycle_index,
                span: BilledSpan::of(&invoice.invoice_type),
                lines: &patch.line_items,
            }],
            // A consolidated invoice bills the periods of the drafts merged into it.
            None => {
                children =
                    InvoiceRow::list_consolidated_children(conn, invoice.tenant_id, invoice.id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .map(Invoice::try_from)
                        .collect::<Result<Vec<_>, _>>()?;
                children
                    .iter()
                    .filter_map(|child| {
                        Some(BilledInvoice {
                            subscription_id: child.subscription_id?,
                            anchor: child.billing_period_start,
                            cycle_index: child.cycle_index,
                            span: BilledSpan::of(&child.invoice_type),
                            lines: &child.line_items,
                        })
                    })
                    .collect()
            }
        };
        if billed.is_empty() {
            return Ok(CreditBilling::Plain);
        }
        let mut grants = credit::lock_grants_tx(conn, lock, &invoice.currency).await?;
        if grants.is_empty() {
            return Ok(CreditBilling::Plain);
        }
        // The window taxes only price what lapsed credit settles; without a lapse none is asked.
        let settles_expiries = expiry_settlement::has_lapsed_expiries(&grants, time.at);
        let mut windows = Vec::new();
        let mut periods = Vec::new();
        for billed in &billed {
            let (billed_windows, billed_periods) = self
                .billed_windows_tx(
                    conn,
                    invoice.tenant_id,
                    &invoice.currency,
                    billed,
                    settles_expiries,
                )
                .await?;
            windows.extend(billed_windows);
            periods.extend(billed_periods);
        }
        periods.sort_by_key(|(subscription_id, start)| (subscription_id.as_uuid(), *start));
        periods.dedup();
        if periods.is_empty() {
            return Ok(CreditBilling::Plain);
        }
        if refreshed {
            metrics::estimate_drift(
                value_application::usage_gross_subtotal(
                    &invoice.line_items,
                    invoice.tax_inclusive,
                )?,
                value_application::usage_gross_subtotal(&patch.line_items, invoice.tax_inclusive)?,
            );
        }
        let mut pinned = PinnedPrices::default();
        pinned.add_lines(&patch.line_items, invoice.tax_inclusive);
        // Its credit-priced lines were covered by their unit wallets before money is drawn: what
        // they cover is what an expiring currency grant's overage is measured against.
        let mut unit_cover: std::collections::HashMap<(SubscriptionId, NaiveDate, String), i64> =
            std::collections::HashMap::new();
        for line in &patch.line_items {
            let (Some(block), Some(subscription_id)) = (
                line.credit_unit.as_ref(),
                line.subscription_id.or(invoice.subscription_id),
            ) else {
                continue;
            };
            let covered = unit_cover
                .entry((
                    subscription_id,
                    line.start_date,
                    credit::normalized_currency(&block.code),
                ))
                .or_insert(0);
            *covered = covered.saturating_add(block.covered);
        }
        expiry_settlement::settle_due_expiries_tx(
            &self.store,
            self.usage_client.as_ref(),
            conn,
            invoice.tenant_id,
            invoice.customer_id,
            &mut grants,
            &windows,
            &pinned,
            &credit::normalized_currency(&invoice.currency),
            expiry_settlement::UnitCover::Invoiced(&unit_cover),
            time,
        )
        .await?;
        Ok(CreditBilling::Periods {
            grants,
            periods: credit::BilledPeriods { periods },
        })
    }

    /// Draws `pool`'s share of `request` for the invoice, taking its periods' settlements over
    /// first when it bills any; `charges` are the usage charges of a promotional request.
    #[allow(clippy::too_many_arguments)]
    async fn draw_credit_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice: &Invoice,
        billing: &CreditBilling,
        pool: GrantPool,
        request: &CreditRequest,
        charges: &[credit::UsageCharge],
        time: credit::MovementTime,
    ) -> StoreResult<Allocation> {
        match billing {
            CreditBilling::Plain => {
                credit::draw_for_invoice_tx(
                    conn,
                    &*self.store.hooks.credit,
                    lock,
                    invoice.id,
                    &invoice.currency,
                    pool,
                    request,
                    time,
                )
                .await
            }
            CreditBilling::Periods { grants, periods } => {
                credit::net_settlements_then_draw_tx(
                    conn,
                    &*self.store.hooks.credit,
                    lock,
                    invoice.id,
                    &invoice.currency,
                    grants,
                    periods,
                    pool,
                    request,
                    charges,
                    time,
                )
                .await
            }
        }
    }

    /// The usage windows a subscription's recurring invoice bills, and the periods of them and of
    /// its usage lines, whose settlements it takes over.
    async fn billed_windows_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        currency: &str,
        billed: &BilledInvoice<'_>,
        with_taxes: bool,
    ) -> StoreResult<(
        Vec<billing_windows::PlannedWindow>,
        Vec<(SubscriptionId, NaiveDate)>,
    )> {
        let details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, billed.subscription_id)
            .await?;
        let cycle_index = billed
            .cycle_index
            .or(details.subscription.cycle_index)
            .unwrap_or(0);
        let currency = credit::normalized_currency(currency);
        let mut windows =
            billing_windows::windows_at(conn, &details, billed.anchor, cycle_index, billed.span)
                .await?
                .into_iter()
                .filter(|window| window.bills_money_in(&currency))
                .collect::<Vec<_>>();
        // Expired paid credit settles the usage with its tax, as the invoice will bill it.
        if with_taxes {
            billing_windows::resolve_window_taxes(
                &self.store,
                conn,
                &details,
                &mut windows,
                common_domain::clock::now(),
            )
            .await?;
        }
        let periods = windows
            .iter()
            .map(|window| window.period.start)
            .chain(
                billed
                    .lines
                    .iter()
                    .filter(|line| line.metric_id.is_some())
                    .map(|line| line.start_date),
            )
            .map(|start| (billed.subscription_id, start))
            .collect::<Vec<_>>();
        Ok((windows, periods))
    }

    async fn increment_and_finalize(
        &self,
        tx: &mut PgConn,
        lock: &CustomerLock,
        actor: &Actor,
        invoice: Invoice,
        applied_coupons_amounts: Vec<CouponLineItem>,
        issue: &Issue,
    ) -> StoreResult<DetailedInvoice> {
        let tenant_id = invoice.tenant_id;
        let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
            tx,
            invoice.invoicing_entity_id,
            invoice.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let seller: crate::domain::InvoicingEntity = invoicing_entity.clone().into();

        // A draft has no emission date — it anchors on `billing_period_start` until now. A live
        // finalization stamps invoice_date with the issue day and rebases due_at on it, so a draft
        // that outlived its period (grace, late usage import) is not issued already overdue.
        // Backdated subscriptions (historical imports) and manual invoices (operator-chosen
        // dates) keep their stored dates.
        let (invoice_date, due_at) = match issue.issued_on {
            Some(day) => invoice.dates_if_issued_on(day),
            None => (invoice.invoice_date, invoice.due_at),
        };

        let new_invoice_number = format_document_number(
            invoicing_entity.next_invoice_number,
            &invoicing_entity.invoice_number_pattern,
            invoice_date,
        );

        let payment_reference = new_invoice_number
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        self.freeze_invoice_identity(tx, &invoice, Some(&seller))
            .await?;

        let _ =
            refresh_applied_coupons(tx, lock, &invoice.currency, &applied_coupons_amounts).await?;

        let applied_coupons_json = serde_json::to_value(&applied_coupons_amounts)
            .map_err(|e| StoreError::SerdeError("Failed to serialize coupons".to_string(), e))?;

        let finalized_at = issue.finalized_at;

        InvoiceRow::finalize(
            tx,
            invoice.id,
            invoice.tenant_id,
            new_invoice_number,
            payment_reference,
            applied_coupons_json,
            finalized_at,
            invoice_date,
            due_at,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        InvoicingEntityRow::update_invoicing_entity_number(
            tx,
            invoice.invoicing_entity_id,
            invoice.tenant_id,
            invoicing_entity.next_invoice_number,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let final_invoice: DetailedInvoice =
            InvoiceRow::find_detailed_by_id(tx, invoice.tenant_id, invoice.id)
                .await
                .map_err(Into::into)
                .and_then(std::convert::TryInto::try_into)?;

        crate::services::credit::mint::mint_included_credit_tx(
            tx,
            &*self.store.hooks.credit,
            &final_invoice.invoice,
            issue.credit_time(),
        )
        .await?;

        // Lines a plan change ended while this invoice was a draft are settled now that it is issued.
        let final_invoice = if self
            .settle_plan_change_endings_for_invoice_tx(tx, &final_invoice.invoice)
            .await?
        {
            InvoiceRow::find_detailed_by_id(tx, invoice.tenant_id, invoice.id)
                .await
                .map_err(Into::into)
                .and_then(std::convert::TryInto::try_into)?
        } else {
            final_invoice
        };

        let invoice_event = (&final_invoice.invoice).into();
        self.store
            .internal
            .record_outbox_batch_tx(
                tx,
                tenant_id,
                actor,
                vec![OutboxEvent::invoice_finalized(invoice_event)],
            )
            .await?;

        Ok(final_invoice)
    }
}

impl crate::services::ServicesEdge {
    pub async fn finalize_invoice_at(
        &self,
        actor: Actor,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        issue: IssueAt,
        delivery: InvoiceDelivery,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .finalize_invoice_at(actor, invoice_id, tenant_id, issue, delivery)
            .await
    }
}

/// A voided invoice consumed none of its coupons: what its finalization recorded is given back,
/// so its period billed again (or the next one) is discounted as it would have been (Stripe).
pub(crate) async fn release_applied_coupons(
    tx_conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
    applied_coupons_amounts: &[CouponLineItem],
) -> DbResult<()> {
    let ids: Vec<AppliedCouponId> = applied_coupons_amounts
        .iter()
        .map(|c| c.applied_coupon_id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let precision = crate::constants::Currencies::resolve_currency_precision(currency).unwrap_or(2);
    for detailed in AppliedCouponDetailedRow::list_by_ids_for_update(tx_conn, lock, &ids).await? {
        let amount_delta = detailed
            .coupon
            .recurring_value
            .is_some_and(|x| x >= 1)
            .then(|| {
                applied_coupons_amounts
                    .iter()
                    .find(|x| x.applied_coupon_id == detailed.applied_coupon.id)
                    .map(|x| x.consumed_value().to_unit(precision))
            })
            .flatten()
            .filter(|amount| *amount > rust_decimal::Decimal::ZERO);
        AppliedCouponRow::release_state(tx_conn, lock, detailed.applied_coupon.id, amount_delta)
            .await?;
    }
    Ok(())
}

async fn refresh_applied_coupons(
    tx_conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
    applied_coupons_amounts: &[CouponLineItem],
) -> DbResult<Vec<AppliedCouponId>> {
    let applied_coupons_ids: Vec<AppliedCouponId> = applied_coupons_amounts
        .iter()
        .map(|c| c.applied_coupon_id)
        .collect();

    let applied_coupons_detailed =
        AppliedCouponDetailedRow::list_by_ids_for_update(tx_conn, lock, &applied_coupons_ids)
            .await?;

    for applied_coupon_detailed in applied_coupons_detailed {
        let amount_delta = if applied_coupon_detailed
            .coupon
            .recurring_value
            .is_some_and(|x| x >= 1)
        {
            // Never panic on an unknown/unsupported currency during finalize (billing must not
            // fail); fall back to precision 2 via the single currency source.
            let precision =
                crate::constants::Currencies::resolve_currency_precision(currency).unwrap_or(2);

            applied_coupons_amounts
                .iter()
                .find(|x| x.applied_coupon_id == applied_coupon_detailed.applied_coupon.id)
                .map(|x| x.consumed_value().to_unit(precision))
        } else {
            None
        };

        AppliedCouponRow::refresh_state(
            tx_conn,
            applied_coupon_detailed.applied_coupon.id,
            amount_delta,
        )
        .await?;
    }

    Ok(applied_coupons_ids)
}

/// How an invoice draws its credit: plainly, or taking over the settlements of the usage periods
/// it bills under the customer's grant locks.
enum CreditBilling {
    Plain,
    Periods {
        grants: credit::LockedGrants,
        periods: credit::BilledPeriods,
    },
}

/// A subscription's recurring or final invoice whose usage periods an invoice bills: the invoice
/// itself, or each draft merged into a consolidated one.
struct BilledInvoice<'a> {
    subscription_id: SubscriptionId,
    anchor: NaiveDate,
    cycle_index: Option<u32>,
    span: BilledSpan,
    lines: &'a [LineItem],
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, 9, 15, 0).unwrap()
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn a_live_finalization_issues_today_and_draws_now() {
        let issue = Issue::resolve(IssueAt::Now, day(1), false, false, fixed_now()).unwrap();
        assert_eq!(issue.issued_on, Some(day(8)));
        assert_eq!(issue.finalized_at, fixed_now().naive_utc());
        assert_eq!(issue.credit_at, fixed_now());
    }

    #[test]
    fn stored_dates_survive_and_a_backdated_invoice_draws_at_its_date() {
        let manual = Issue::resolve(IssueAt::Now, day(1), true, false, fixed_now()).unwrap();
        assert_eq!(manual.issued_on, None);
        assert_eq!(manual.credit_at, fixed_now());

        let backdated = Issue::resolve(IssueAt::Now, day(1), false, true, fixed_now()).unwrap();
        let noon = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        assert_eq!(backdated.issued_on, None);
        assert_eq!(backdated.finalized_at, noon.naive_utc());
        assert_eq!(backdated.credit_at, noon);

        // Dated today, before noon: the credit is never read ahead of the clock.
        let today = Issue::resolve(IssueAt::Now, day(8), false, true, fixed_now()).unwrap();
        assert_eq!(today.credit_at, fixed_now());
    }

    #[test]
    fn an_explicit_moment_dates_the_invoice_and_its_credit() {
        let at = Utc.with_ymd_and_hms(2026, 9, 3, 18, 30, 0).unwrap();
        let issue = Issue::resolve(IssueAt::At(at), day(1), false, true, fixed_now()).unwrap();
        assert_eq!(issue.issued_on, Some(at.date_naive()));
        assert_eq!(issue.finalized_at, at.naive_utc());
        assert_eq!(issue.credit_at, at);

        let manual = Issue::resolve(IssueAt::At(at), day(1), true, false, fixed_now()).unwrap();
        assert_eq!(manual.issued_on, None);
        assert_eq!(manual.credit_at, at);

        let future = fixed_now() + chrono::Duration::seconds(1);
        assert!(Issue::resolve(IssueAt::At(future), day(1), false, false, fixed_now()).is_err());
    }
}

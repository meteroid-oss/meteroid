use crate::StoreResult;
use crate::domain::{Invoice, InvoiceLinesPatch, SubscriptionDetails};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::invoice_lines::credit_units::{CoverTarget, CreditUnitCover};
use crate::services::invoice_lines::invoice_lines::{BilledSpan, ComputedInvoiceContent};
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::PgConn;
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch};
use error_stack::{Report, ResultExt, bail};
use scoped_futures::ScopedFutureExt;

impl Services {
    /// [`Self::refresh_invoice_data`] as the outermost transaction: the metering the refresh
    /// prices (the draft's usage, the earlier drafts priced again) is read before the draft and
    /// its customer are locked, so nothing that needs them waits on metering.
    pub(in crate::services) async fn refresh_draft_reading_usage_first(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<Invoice> {
        self.transaction_reading_usage_first(conn, |services, conn| {
            async move {
                services
                    .refresh_invoice_data(conn, id, tenant_id, &None, true)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    pub(in crate::services) async fn refresh_invoice_data(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        subscription_details_cache: &Option<SubscriptionDetails>,
        refresh_invoice_lines: bool,
    ) -> StoreResult<Invoice> {
        let invoice_details = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    let invoice_lock =
                        InvoiceRow::select_for_update_by_id(conn, tenant_id, id).await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &invoice_lock.lock).await?;

                    let invoice: Invoice = invoice_lock.invoice.try_into()?;

                    // An operator who edited the billing content owns these lines: recomputing
                    // would regenerate the usage lines over their quantities, drop lines they
                    // added by hand (no `sub_component_id` to rebuild from), and derive `total`
                    // from coupons alone while the `discount` column keeps their value — the
                    // patch carries no discount. Mirrors the routing in `finalize_invoice` and
                    // `consolidate`. Everything else still refreshes (credits, coupons, tax,
                    // annexes) and the row is stamped, so the sweep does not retry it hourly.
                    let refresh_invoice_lines =
                        refresh_invoice_lines && invoice.manually_edited_at.is_none();

                    let patch = self
                        .build_invoice_lines_patch(
                            conn,
                            &invoice,
                            subscription_details_cache,
                            refresh_invoice_lines,
                            CreditUnitCover::Estimate {
                                invoice_id: Some(invoice.id),
                            },
                        )
                        .await?;

                    // Same rule as the settle path: an empty recompute may only stand for an
                    // invoice that was already empty. Lines that existed and recomputed away mean
                    // the recompute disagrees with what was billed (e.g. a metering outage);
                    // writing the wipe — or merging it into a consolidated parent — would forfeit
                    // that revenue silently. Refuse before `update_lines`: a failed scheduled
                    // event still commits what it wrote beforehand.
                    if refresh_invoice_lines
                        && patch.line_items.is_empty()
                        && !invoice.line_items.is_empty()
                        && invoice.invoice_type.bills_subscription_usage()
                    {
                        return Err(Report::new(StoreError::InvoiceComputationError).attach(
                            format!(
                                "Refusing to refresh invoice {id}: it has stored lines but recomputed to nothing"
                            ),
                        ));
                    }

                    let row_patch: InvoiceRowLinesPatch = patch.try_into()?;

                    row_patch
                        .update_lines(id, tenant_id, conn)
                        .await
                        .map(|_| ())
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok::<_, StoreErrorReport>(invoice)
                }
                .scope_boxed()
            })
            .await?;

        Ok(invoice_details)
    }

    /// To be called alongside the other refresh, like the coupons (and mrr?). `cover` decides how
    /// recomputed credit-priced lines learn what their unit wallets pay.
    pub(in crate::services) async fn build_invoice_lines_patch(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        subscription_details_cache: &Option<SubscriptionDetails>,
        refresh_invoice_lines: bool,
        cover: CreditUnitCover,
    ) -> StoreResult<InvoiceLinesPatch> {
        if !invoice.can_edit() {
            bail!(StoreError::InvalidArgument(
                "Cannot refresh invoice that is not in draft or pending status".into(),
            ));
        }

        // A draft's estimate draws what the drafts finalizing before it leave.
        let earlier = match cover {
            CreditUnitCover::Estimate { .. } => {
                self.books_before_draft(conn, invoice, common_domain::clock::now())
                    .await?
            }
            CreditUnitCover::Draw { .. } | CreditUnitCover::DrawAsShown { .. } => None,
        };
        let content = self
            .invoice_content_on(
                conn,
                invoice,
                subscription_details_cache,
                refresh_invoice_lines,
                cover,
                earlier.as_ref(),
            )
            .await?;

        // Annexes are a projection of the raw event stream, so they are re-resolved against the
        // (possibly recomputed) lines and frozen when this patch is applied at finalization.
        //
        // Two drafts keep their stored snapshot instead, because the operator owns their billing
        // content: a correction reissuing a fully credited invoice (its lines are a detached clone
        // of an already issued document), and any draft whose lines or discount were hand-edited.
        // Re-deriving there would publish an appendix that contradicts the edited lines — and the
        // preview the operator approved.
        let annexes = if invoice.is_correction() || invoice.manually_edited_at.is_some() {
            invoice.annexes.clone()
        } else {
            self.resolve_invoice_annexes(
                conn,
                invoice.tenant_id,
                invoice.plan_version_id,
                invoice.customer_id,
                invoice.subscription_id,
                &content.invoice_lines,
            )
            .await?
        };

        Ok(InvoiceLinesPatch {
            line_items: content.invoice_lines,
            amount_due: content.amount_due,
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            total: content.total,
            tax_amount: content.tax_amount,
            applied_credits: content.applied_credits,
            promotional_credits: content.promotional_credits,
            applied_coupons: content.applied_coupons,
            tax_breakdown: content.tax_breakdown,
            annexes,
        })
    }

    /// The content `build_invoice_lines_patch` writes, its credit estimated on `book` when given.
    pub(in crate::services) async fn invoice_content_on(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        subscription_details_cache: &Option<SubscriptionDetails>,
        refresh_invoice_lines: bool,
        cover: CreditUnitCover,
        book: Option<&CreditBooks>,
    ) -> StoreResult<ComputedInvoiceContent> {
        Ok(if refresh_invoice_lines {
            let res;

            let subscription_details = match subscription_details_cache {
                Some(details) => details,
                None => match invoice.subscription_id {
                    Some(subscription_id) => {
                        res = self
                            .store
                            .get_subscription_details_with_conn(
                                conn,
                                invoice.tenant_id,
                                subscription_id,
                            )
                            .await?;
                        &res
                    }
                    None => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot refresh invoice lines without subscription details".into(),
                        )
                        .into());
                    }
                },
            };

            let own_period;
            let subscription_details =
                match minimum_of_its_period(conn, invoice, subscription_details).await? {
                    Some(details) => {
                        own_period = details;
                        &own_period
                    }
                    None => subscription_details,
                };

            // Actual payments already applied (excludes credits, which compute_invoice recalculates)
            let already_paid_amount = invoice.total - invoice.amount_due - invoice.applied_credits;

            self.compute_invoice_on(
                conn,
                &invoice.billing_period_start,
                subscription_details,
                if already_paid_amount > 0 {
                    Some(already_paid_amount as u64)
                } else {
                    None
                },
                Some(invoice),
                invoice.invoice_type.clone(),
                cover,
                book,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?
        } else if let CreditUnitCover::Draw { at, .. } = cover
            && invoice.subscription_id.is_none()
            && invoice
                .line_items
                .iter()
                .any(|line| line.credit_unit.is_some())
        {
            // A consolidated invoice draws its units over all its drafts at once, priced again.
            if invoice.invoice_type != crate::domain::InvoiceType::Recurring {
                bail!(StoreError::InvalidArgument(
                    "usage priced in a credit unit is billed on a subscription's invoice".into(),
                ));
            }
            self.consolidated_content_drawing_units(conn, invoice, at)
                .await?
        } else {
            // Stored credit-priced lines stand as shown: their units are drawn exactly so.
            if let CreditUnitCover::Draw { invoice_id, at } = cover
                && let Some(subscription_id) = invoice.subscription_id
                && invoice
                    .line_items
                    .iter()
                    .any(|line| line.credit_unit.is_some())
            {
                self.draw_credit_units_as_shown(
                    conn,
                    invoice,
                    invoice_id,
                    subscription_id,
                    subscription_details_cache,
                    at,
                )
                .await?;
            }
            // The stored lines stand; only the credit they would draw is re-estimated.
            let already_paid =
                (invoice.total - invoice.amount_due - invoice.applied_credits).max(0);
            let at = match cover {
                CreditUnitCover::Estimate { .. } => common_domain::clock::now(),
                CreditUnitCover::Draw { at, .. } | CreditUnitCover::DrawAsShown { at, .. } => at.at,
            };
            self.estimate_stored_credit(conn, invoice, already_paid, at, book)
                .await?
        })
    }

    /// Draws the units a subscription invoice's stored credit-priced lines show covered.
    async fn draw_credit_units_as_shown(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        invoice_id: InvoiceId,
        subscription_id: common_domain::ids::SubscriptionId,
        subscription_details_cache: &Option<SubscriptionDetails>,
        at: crate::repositories::credit::customer_credit::MovementTime,
    ) -> StoreResult<()> {
        let loaded;
        let details = match subscription_details_cache {
            Some(details) => details,
            None => {
                loaded = self
                    .store
                    .get_subscription_details_with_conn(conn, invoice.tenant_id, subscription_id)
                    .await?;
                &loaded
            }
        };
        let currency_precision =
            crate::constants::Currencies::resolve_currency_precision(&invoice.currency)
                .ok_or_else(|| {
                    Report::new(StoreError::ValueNotFound(format!(
                        "Currency {} not found",
                        invoice.currency
                    )))
                })?;
        let mut lines = invoice.line_items.clone();
        self.cover_credit_unit_lines(
            conn,
            CreditUnitCover::DrawAsShown { invoice_id, at },
            &mut [CoverTarget {
                details,
                invoice_date: invoice.billing_period_start,
                cycle_index: invoice
                    .cycle_index
                    .or(details.subscription.cycle_index)
                    .unwrap_or(0),
                span: BilledSpan::of(&invoice.invoice_type),
                currency_precision,
                lines: &mut lines,
            }],
        )
        .await
    }
}

/// A period invoice keeps the minimum of the plan version it bills, read from that version once a
/// scheduled change replaced it. An override survives plan changes and has no history: like any
/// subscription term edited since, it applies to every unissued period.
pub(in crate::services) async fn minimum_of_its_period(
    conn: &mut PgConn,
    invoice: &Invoice,
    details: &SubscriptionDetails,
) -> crate::StoreResult<Option<SubscriptionDetails>> {
    let Some(plan_version_id) = invoice
        .plan_version_id
        .filter(|plan| *plan != details.subscription.plan_version_id)
    else {
        return Ok(None);
    };
    if invoice.invoice_type != crate::domain::InvoiceType::Recurring
        || details
            .minimum
            .as_ref()
            .is_some_and(|minimum| minimum.is_override)
    {
        return Ok(None);
    }
    let minimum = crate::repositories::plans::load_plan_minimum(conn, plan_version_id)
        .await?
        .map(|minimum| crate::domain::SubscriptionMinimum {
            subscription_id: details.subscription.id,
            amount_subunit: minimum.amount_subunit,
            scope: minimum.scope,
            is_override: false,
        });
    Ok(Some(SubscriptionDetails {
        minimum,
        ..details.clone()
    }))
}

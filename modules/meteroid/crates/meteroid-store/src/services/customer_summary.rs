//! The customer summary (worth, receivables, term) and a subscription's usage this cycle.

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{CustomerId, SubscriptionId, TenantId};
use diesel_models::query::account_signals::{
    self as queries, FailedPaymentRow, OpenInvoiceRow, PendingRetryRow, SignalSubscriptionRow,
};
use diesel_models::query::alert_settings::builtin_alert_settings;
use error_stack::Report;

use crate::StoreResult;
use crate::domain::customer_signals::LIMIT_BURNDOWN_CODE;
use crate::domain::customer_summary::{
    CustomerReceivables, CustomerSummary, CustomerTerm, FeatureUsage, LimitBurndownSettings,
    MoneyTotal, RenewalMode, SubscriptionUsage, SummaryParts, UsageCycle, currency_totals,
    days_left, ends_on, project_linear, usage_pct,
};
use crate::domain::entitlements::{OverageBehavior, ResetPeriod};
use crate::errors::StoreError;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::entitlements::{SubscriptionCycle, USAGE_FETCH_CONCURRENCY};
use crate::services::fx::{customer_billing_mrr, reporting_fx};
use crate::services::{Services, ServicesEdge};
use crate::utils::datetime::{elapsed_between, elapsed_fraction};

impl ServicesEdge {
    /// A missing customer is `ValueNotFound`; a part the caller may not see is left out.
    pub async fn get_customer_summary(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        parts: SummaryParts,
        now: NaiveDateTime,
    ) -> StoreResult<CustomerSummary> {
        self.services
            .get_customer_summary(tenant_id, customer_id, parts, now)
            .await
    }

    /// Limited metered entitlements of the subscription against its current windows.
    pub async fn get_subscription_usage(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: NaiveDateTime,
    ) -> StoreResult<SubscriptionUsage> {
        self.services
            .get_subscription_usage(tenant_id, subscription_id, now)
            .await
    }
}

impl Services {
    async fn get_customer_summary(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        parts: SummaryParts,
        now: NaiveDateTime,
    ) -> StoreResult<CustomerSummary> {
        self.store
            .find_customer_by_id(customer_id, tenant_id)
            .await?;
        let fx = reporting_fx(&self.store, tenant_id, now.date()).await?;
        let mrr_cents = customer_billing_mrr(&self.store, tenant_id, &[customer_id], &fx)
            .await?
            .get(&customer_id)
            .copied()
            .unwrap_or(0);

        let receivables = if parts.invoices {
            Some(self.receivables(tenant_id, customer_id, now).await?)
        } else {
            None
        };
        let term = if parts.subscriptions {
            self.term(tenant_id, customer_id, now.date()).await?
        } else {
            None
        };

        Ok(CustomerSummary {
            mrr_cents,
            reporting_currency: fx.target().to_string(),
            receivables,
            term,
        })
    }

    async fn receivables(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        now: NaiveDateTime,
    ) -> StoreResult<CustomerReceivables> {
        let mut conn = self.store.get_conn().await?;
        let open = queries::list_open_invoices(&mut conn, tenant_id, customer_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let ids: Vec<_> = open.iter().map(|i| i.id).collect();
        let failures = queries::list_last_failed_payments(&mut conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let retries = queries::list_pending_payment_retries(&mut conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(summarize_receivables(&open, &failures, &retries, now))
    }

    async fn term(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        today: NaiveDate,
    ) -> StoreResult<Option<CustomerTerm>> {
        let mut conn = self.store.get_conn().await?;
        let subscriptions =
            queries::list_billing_active_subscriptions(&mut conn, tenant_id, &[customer_id])
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let Some(mut term) = earliest_term(&subscriptions, today) else {
            return Ok(None);
        };
        if term.mode == RenewalMode::Ending {
            term.cancellation_event_id =
                queries::pending_cancellation_event(&mut conn, tenant_id, term.subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
        }
        Ok(Some(term))
    }

    async fn get_subscription_usage(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: NaiveDateTime,
    ) -> StoreResult<SubscriptionUsage> {
        let sub = self
            .store
            .get_subscription(tenant_id, subscription_id)
            .await?;
        let cycle = SubscriptionCycle {
            subscription_id,
            customer_id: sub.customer_id,
            period_start: sub.current_period_start,
            period_end: sub.current_period_end,
            activated_at: sub.activated_at,
        };
        let settings = self.limit_settings(tenant_id).await;
        let limited = self
            .limited_usage_for_subscription(tenant_id, &cycle, now, USAGE_FETCH_CONCURRENCY)
            .await?;
        let usage = limited
            .into_iter()
            .map(|u| {
                let elapsed = u
                    .period_end
                    .map(|end| elapsed_between(u.period_start, end, now))
                    .unwrap_or(0.0);
                let refused = u.refused();
                let warning_pct = u.warning_pct();
                FeatureUsage {
                    feature_id: u.feature.id,
                    feature_name: u.feature.name,
                    pct: usage_pct(u.consumed, u.limit),
                    projected: project_linear(
                        u.consumed,
                        elapsed,
                        f64::from(settings.min_projection_elapsed_pct) / 100.0,
                    ),
                    consumed: u.consumed,
                    limit: Some(u.limit),
                    warning_threshold_pct: settings.threshold_pct(warning_pct),
                    reset: reset_name(&u.reset_period).to_string(),
                    period_start: Some(u.period_start.date()),
                    resets_at: u.period_end,
                    overage_behavior: match u.overage_behavior {
                        OverageBehavior::Allow => "allow",
                        OverageBehavior::Block { .. } => "block",
                    }
                    .to_string(),
                    refused,
                    grace_pct: match u.overage_behavior {
                        OverageBehavior::Block { grace_period_pct } => grace_period_pct,
                        OverageBehavior::Allow => None,
                    },
                    group_by: u.group_by,
                }
            })
            .collect();

        Ok(SubscriptionUsage {
            cycle: Some(UsageCycle {
                period_start: sub.current_period_start,
                period_end: sub.current_period_end,
                elapsed_pct: sub
                    .current_period_end
                    .map(|end| elapsed_fraction(sub.current_period_start, end, now) * 100.0)
                    .unwrap_or(0.0),
                days_left: sub.current_period_end.map(|end| days_left(end, now.date())),
            }),
            usage,
        })
    }

    /// The tenant's limit_burndown thresholds (warning marker, projection start); defaults while
    /// the built-in is not seeded.
    async fn limit_settings(&self, tenant_id: TenantId) -> LimitBurndownSettings {
        let settings = match self.store.get_conn().await {
            Ok(mut conn) => builtin_alert_settings(&mut conn, tenant_id, LIMIT_BURNDOWN_CODE)
                .await
                .ok()
                .flatten(),
            Err(_) => None,
        };
        LimitBurndownSettings::parse(settings.as_ref()).unwrap_or_default()
    }
}

/// The earliest upcoming scheduled end or renewal of a billing-active subscription.
fn earliest_term(subs: &[SignalSubscriptionRow], today: NaiveDate) -> Option<CustomerTerm> {
    subs.iter()
        .filter_map(|s| {
            let (mode, date) = match ends_on(s) {
                Some(end) if end >= today => (RenewalMode::Ending, end),
                Some(_) => return None,
                None => (RenewalMode::Renewing, s.current_period_end?),
            };
            Some(CustomerTerm {
                subscription_id: s.id,
                plan_name: s.plan_name.clone(),
                date,
                mode,
                cancellation_event_id: None,
            })
        })
        .min_by_key(|t| t.date)
}

fn summarize_receivables(
    open: &[OpenInvoiceRow],
    failures: &[FailedPaymentRow],
    retries: &[PendingRetryRow],
    now: NaiveDateTime,
) -> CustomerReceivables {
    let totals = |invoices: &mut dyn Iterator<Item = &OpenInvoiceRow>| {
        currency_totals(invoices.map(|i| (i.currency.as_str(), i.amount_due)))
            .into_iter()
            .map(|t| MoneyTotal {
                currency: t.currency,
                amount_cents: t.amount_due_cents,
                invoice_count: t.count,
            })
            .collect::<Vec<_>>()
    };
    let last_failure = failures.iter().max_by_key(|f| f.failed_at);

    CustomerReceivables {
        open: totals(&mut open.iter()),
        overdue: totals(&mut open.iter().filter(|i| {
            i.due_at
                .is_some_and(|d| crate::services::subscription_receivables::is_past_due(d, now))
        })),
        last_failure_at: last_failure.map(|f| f.failed_at),
        last_failure_reason: last_failure.and_then(|f| f.error_type.clone()),
        last_failure_invoice_id: last_failure.map(|f| f.invoice_id),
        next_retry_at: retries.iter().map(|r| r.next_retry_at).min(),
    }
}

fn reset_name(reset: &ResetPeriod) -> &'static str {
    match reset {
        ResetPeriod::BillingCycle => "billing_cycle",
        ResetPeriod::Calendar { .. } => "calendar",
        ResetPeriod::FixedWindow { .. } => "fixed_window",
        ResetPeriod::SlidingWindow { .. } => "sliding_window",
        ResetPeriod::Never => "never",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::InvoiceId;
    use uuid::Uuid;

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn invoice(n: u128, currency: &str, amount_due: i64, due: Option<u32>) -> OpenInvoiceRow {
        OpenInvoiceRow {
            id: InvoiceId::from(Uuid::from_u128(n)),
            invoice_number: format!("INV-{n}"),
            currency: currency.to_string(),
            amount_due,
            due_at: due.map(at),
            payment_status: "UNPAID".to_string(),
        }
    }

    #[test]
    fn an_invoice_due_today_is_not_overdue_yet() {
        let open = vec![invoice(1, "EUR", 1_000, Some(10))];
        let r = summarize_receivables(&open, &[], &[], at(10) + chrono::Duration::hours(9));
        assert!(r.overdue.is_empty());
        let r = summarize_receivables(&open, &[], &[], at(11));
        assert_eq!(r.overdue.len(), 1);
    }

    #[test]
    fn receivables_split_open_and_overdue_per_currency() {
        let open = vec![
            invoice(1, "EUR", 1_000, Some(1)),
            invoice(2, "USD", 500, Some(5)),
            invoice(3, "EUR", 300, Some(20)),
            invoice(4, "EUR", 200, None),
        ];
        let failures = vec![
            FailedPaymentRow {
                invoice_id: open[0].id,
                failed_at: at(2),
                error_type: Some("card_declined".into()),
            },
            FailedPaymentRow {
                invoice_id: open[1].id,
                failed_at: at(6),
                error_type: Some("insufficient_funds".into()),
            },
        ];
        let retries = vec![
            PendingRetryRow {
                invoice_id: open[0].id,
                next_retry_at: at(18),
            },
            PendingRetryRow {
                invoice_id: open[1].id,
                next_retry_at: at(17),
            },
        ];

        let receivables = summarize_receivables(&open, &failures, &retries, at(15));
        let money = |m: &[MoneyTotal]| {
            m.iter()
                .map(|m| (m.currency.clone(), m.amount_cents, m.invoice_count))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            money(&receivables.open),
            vec![("EUR".into(), 1_500, 3), ("USD".into(), 500, 1)]
        );
        assert_eq!(
            money(&receivables.overdue),
            vec![("EUR".into(), 1_000, 1), ("USD".into(), 500, 1)]
        );
        assert_eq!(receivables.last_failure_invoice_id, Some(open[1].id));
        assert_eq!(
            receivables.last_failure_reason.as_deref(),
            Some("insufficient_funds")
        );
        assert_eq!(receivables.next_retry_at, Some(at(17)));
    }
}

//! What the intelligence module's signals read of these services, re-exported by
//! `crate::intelligence_support`: internal to that module.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BillableMetricId, PlanVersionId, SubscriptionId, TenantId};

use crate::domain::{BillableMetric, ComponentParameterization, Invoice, SubscriptionDetails};
use crate::services::clients::usage::UsageClient;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::services::invoice_lines::minimum::PeriodTrueUp;
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use crate::{Store, StoreResult};

pub use super::entitlements::{LimitedUsage, SubscriptionCycle, USAGE_FETCH_CONCURRENCY};
pub use super::invoices::upcoming::upcoming_invoice_details;
pub use super::orchestration::payment_transaction_failed::DUNNING_RETRY_SCHEDULE_DAYS;
pub use super::subscriptions::plan_change::PreparedPlanChange;
pub use super::subscriptions::utils::{
    MrrLine, calculate_mrr, floor_subscription_mrr, monthly_share,
};
pub use crate::domain::credit::stop_reason_of;

/// The services a signal reads billing through: metering, plan-change previews, invoice compute.
#[derive(Clone)]
pub struct SignalServices(Services);

impl SignalServices {
    pub fn of(services: &ServicesEdge) -> Self {
        Self(services.services.clone())
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.0.store
    }

    pub fn usage_client(&self) -> &Arc<dyn UsageClient> {
        &self.0.usage_client
    }

    /// These services reading metering through `usage_client`.
    pub fn reading_usage(&self, usage_client: Arc<dyn UsageClient>) -> Self {
        Self(self.0.reading_usage(usage_client))
    }

    pub async fn limited_usage_for_subscription(
        &self,
        tenant_id: TenantId,
        sub: &SubscriptionCycle,
        now: NaiveDateTime,
        concurrency: usize,
    ) -> StoreResult<Vec<LimitedUsage>> {
        self.0
            .limited_usage_for_subscription(tenant_id, sub, now, concurrency)
            .await
    }

    pub async fn load_metrics(
        &self,
        conn: &mut PgConn,
        metric_ids: &[BillableMetricId],
        tenant_id: &TenantId,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<HashMap<BillableMetricId, BillableMetric>> {
        self.0
            .load_metrics(conn, metric_ids, tenant_id, subscription_id)
            .await
    }

    pub async fn prepare_plan_change_readonly(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        self.0
            .prepare_plan_change_readonly(
                conn,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
                change_date,
            )
            .await
    }

    /// The stored subscription's recurring invoice of `period_start` billed on the rows of
    /// `subscription_details` instead of its own, from its facts: the whole period with its
    /// usage, reconciled against no issued invoice. Nothing is written.
    pub async fn compute_period_invoice_on_rows(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        period_start: NaiveDate,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.0
            .preview_period_invoice_on_rows(conn, subscription_details, period_start)
            .await
    }

    /// The stored subscription's upcoming invoice, from its facts: a change made to
    /// `subscription_details` in memory is not read.
    pub async fn compute_upcoming_invoice(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.0
            .compute_upcoming_invoice(conn, subscription_details)
            .await
    }

    /// The upcoming invoice once `prepared`, a plan change to `target` not written, is in force,
    /// from the facts with its rows laid over.
    pub async fn compute_upcoming_invoice_after_plan_change(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        target: common_domain::ids::PlanVersionId,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.0
            .upcoming_invoice_after_plan_change(conn, prepared, target)
            .await
    }

    /// The upcoming invoice with one more `add_on` from the current period, from the facts with
    /// its row laid over.
    pub async fn compute_upcoming_invoice_with_add_on(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        add_on: common_domain::ids::AddOnId,
    ) -> StoreResult<ComputedInvoiceContent> {
        self.0
            .upcoming_invoice_with_add_on(conn, subscription_details, add_on)
            .await
    }
}

pub async fn load_period_true_up(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    invoice: Option<&Invoice>,
) -> StoreResult<Option<PeriodTrueUp>> {
    super::invoice_lines::minimum::load_period_true_up(
        conn,
        details,
        invoice_date,
        cycle_index,
        invoice,
    )
    .await
}

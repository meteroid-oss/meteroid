//! Demo-data writes of Sandbox tenants, kept off the production repository traits.

use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::enums::{
    InvoicePaymentStatus, InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum,
    TenantEnvironmentEnum,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRowNew;
use diesel_models::tenants::TenantRow;
use error_stack::{Report, bail};
use scoped_futures::ScopedFutureExt;

use crate::domain::Invoice;
use crate::domain::sandbox_seeds::SandboxSeed;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use diesel_models::customer_credit::CreditRecomputeMarkRow;
use diesel_models::sandbox_seed::SandboxSeedRow;

pub const INTERRUPTED_SEED_REASON: &str =
    "Seeding was interrupted. Create a new sandbox to try again.";

#[async_trait::async_trait]
pub trait SandboxInterface {
    /// Records a declined card payment on a finalized, unpaid invoice of a Sandbox tenant and
    /// marks the invoice ERRORED, without touching any payment provider.
    async fn stage_sandbox_payment_failure(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice>;

    /// Dates a seeded Sandbox's signatures before what they started, as a real book would have
    /// them: a subscription signed a week before it started, a pending cancellation given twelve
    /// days' notice (never later than now). Seeding writes them all today otherwise, and a target's
    /// past days would read as if billing never knew anything ahead.
    async fn backdate_sandbox_signatures(&self, tenant_id: TenantId) -> StoreResult<()>;

    async fn start_sandbox_seed(&self, tenant_id: TenantId) -> StoreResult<()>;

    /// Completes the running seed, or fails it with a reason shown to the user.
    async fn finish_sandbox_seed(
        &self,
        tenant_id: TenantId,
        failure_reason: Option<String>,
    ) -> StoreResult<()>;

    /// Fails the seeds a dead process left running; their tenants' customers are marked for a
    /// recompute, as when a seed finishes.
    async fn reap_interrupted_sandbox_seeds(&self) -> StoreResult<Vec<TenantId>>;

    /// `None` for a tenant seeded before seeds were tracked.
    async fn get_sandbox_seed(&self, tenant_id: TenantId) -> StoreResult<Option<SandboxSeed>>;
}

#[async_trait::async_trait]
impl SandboxInterface for Store {
    async fn start_sandbox_seed(&self, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        SandboxSeedRow::start(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn finish_sandbox_seed(
        &self,
        tenant_id: TenantId,
        failure_reason: Option<String>,
    ) -> StoreResult<()> {
        // The recompute leader skipped the tenant while it seeded: it judges every balance now.
        self.transaction(|conn| {
            async move {
                let finished = SandboxSeedRow::finish(conn, tenant_id, failure_reason)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if finished {
                    CreditRecomputeMarkRow::mark_tenant_customers(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn reap_interrupted_sandbox_seeds(&self) -> StoreResult<Vec<TenantId>> {
        self.transaction(|conn| {
            async move {
                let reaped = SandboxSeedRow::fail_interrupted(conn, INTERRUPTED_SEED_REASON)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                for tenant_id in &reaped {
                    CreditRecomputeMarkRow::mark_tenant_customers(conn, *tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }
                Ok(reaped)
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_sandbox_seed(&self, tenant_id: TenantId) -> StoreResult<Option<SandboxSeed>> {
        let mut conn = self.get_conn().await?;
        SandboxSeedRow::find(&mut conn, tenant_id)
            .await
            .map(|row| row.map(Into::into))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn backdate_sandbox_signatures(&self, tenant_id: TenantId) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let tenant = TenantRow::find_by_id(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if tenant.environment != TenantEnvironmentEnum::Sandbox {
                    bail!(StoreError::InvalidArgument(
                        "only a sandbox's signatures are backdated".to_string()
                    ));
                }
                diesel_models::query::sandbox_seed::backdate_sandbox_signatures(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn stage_sandbox_payment_failure(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice> {
        self.transaction(|conn| {
            async move {
                let tenant = TenantRow::find_by_id(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if tenant.environment != TenantEnvironmentEnum::Sandbox {
                    bail!(StoreError::InvalidArgument(
                        "payment failures can only be staged on sandbox tenants".to_string()
                    ));
                }

                let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if invoice.status != InvoiceStatusEnum::Finalized
                    || invoice.payment_status != InvoicePaymentStatus::Unpaid
                    || invoice.amount_due <= 0
                {
                    bail!(StoreError::InvalidArgument(
                        "only a finalized, unpaid invoice with an amount due can fail".to_string()
                    ));
                }

                PaymentTransactionRowNew {
                    id: PaymentTransactionId::new(),
                    tenant_id,
                    invoice_id: Some(invoice_id),
                    provider_transaction_id: None,
                    amount: invoice.amount_due,
                    currency: invoice.currency.clone(),
                    payment_method_id: None,
                    status: PaymentStatusEnum::Failed,
                    payment_type: PaymentTypeEnum::Payment,
                    error_type: Some("card_declined".to_string()),
                    processed_at: Some(common_domain::clock::now().naive_utc()),
                    checkout_session_id: None,
                    pending_plan_version_id: None,
                    next_action: None,
                    initiated_by_customer_id: None,
                    pending_provider_intent_id: None,
                    pending_connection_id: None,
                    parent_transaction_id: None,
                    credit_note_id: None,
                    refund_mode: None,
                    reversal_reason: None,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                InvoiceRow::apply_payment_status(
                    conn,
                    invoice_id,
                    tenant_id,
                    InvoicePaymentStatus::Errored,
                    None,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(TryInto::try_into)
            }
            .scope_boxed()
        })
        .await
    }
}

//! Invoices and credit notes as API v2 lists them (ADR-0013 §11): by keyset in SQL, the
//! collections being unbounded.
use crate::StoreResult;
use crate::domain::enums::{CreditNoteStatus, InvoicePaymentStatus, InvoiceStatusEnum};
use crate::domain::{CreditNote, InvoiceWithCustomer};
use crate::errors::StoreError;
use crate::store::Store;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;

/// What a page of invoices is narrowed to.
#[derive(Debug, Clone, Default)]
pub struct InvoicePageFilter {
    pub customer_id: Option<CustomerId>,
    /// Its own invoices and the consolidated ones its drafts were merged into.
    pub subscription_id: Option<SubscriptionId>,
    pub statuses: Option<Vec<InvoiceStatusEnum>>,
    pub payment_statuses: Option<Vec<InvoicePaymentStatus>>,
    pub currency: Option<String>,
    /// Only those finalized after it.
    pub issued_after: Option<NaiveDateTime>,
    /// Only those created or changed after it: `updated_at`, which every edit of the row sets.
    pub updated_after: Option<NaiveDateTime>,
    /// Only those created after it.
    pub created_after: Option<DateTime<Utc>>,
    /// Only those created before it.
    pub created_before: Option<DateTime<Utc>>,
}

/// What a page of credit notes is narrowed to.
#[derive(Debug, Clone, Default)]
pub struct CreditNotePageFilter {
    pub customer_id: Option<CustomerId>,
    pub invoice_id: Option<InvoiceId>,
    pub subscription_id: Option<SubscriptionId>,
    pub statuses: Option<Vec<CreditNoteStatus>>,
    /// Only those created after it.
    pub created_after: Option<DateTime<Utc>>,
    /// Only those created before it.
    pub created_before: Option<DateTime<Utc>>,
}

#[async_trait::async_trait]
pub trait DocumentPagesInterface {
    /// The tenant's invoices of `filter` in id order, those after `after`, at most `limit`, each
    /// with its live customer.
    async fn list_invoices_after(
        &self,
        tenant_id: TenantId,
        filter: InvoicePageFilter,
        after: Option<InvoiceId>,
        limit: usize,
    ) -> StoreResult<Vec<InvoiceWithCustomer>>;

    /// The tenant's credit notes of `filter` in id order, those after `after`, at most `limit`.
    async fn list_credit_notes_after(
        &self,
        tenant_id: TenantId,
        filter: CreditNotePageFilter,
        after: Option<CreditNoteId>,
        limit: usize,
    ) -> StoreResult<Vec<CreditNote>>;
}

#[async_trait::async_trait]
impl DocumentPagesInterface for Store {
    async fn list_invoices_after(
        &self,
        tenant_id: TenantId,
        filter: InvoicePageFilter,
        after: Option<InvoiceId>,
        limit: usize,
    ) -> StoreResult<Vec<InvoiceWithCustomer>> {
        let mut conn = self.get_conn().await?;
        InvoiceRow::list_page(
            &mut conn,
            tenant_id,
            filter.customer_id,
            filter.subscription_id,
            filter
                .statuses
                .map(|statuses| statuses.into_iter().map(Into::into).collect()),
            filter
                .payment_statuses
                .map(|statuses| statuses.into_iter().map(Into::into).collect()),
            filter.currency,
            filter.issued_after,
            filter.updated_after,
            filter.created_after,
            filter.created_before,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(InvoiceWithCustomer::try_from)
        .collect()
    }

    async fn list_credit_notes_after(
        &self,
        tenant_id: TenantId,
        filter: CreditNotePageFilter,
        after: Option<CreditNoteId>,
        limit: usize,
    ) -> StoreResult<Vec<CreditNote>> {
        let mut conn = self.get_conn().await?;
        CreditNoteRow::list_page(
            &mut conn,
            tenant_id,
            filter.customer_id,
            filter.invoice_id,
            filter.subscription_id,
            filter
                .statuses
                .map(|statuses| statuses.into_iter().map(Into::into).collect()),
            filter.created_after,
            filter.created_before,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(CreditNote::try_from)
        .collect()
    }
}

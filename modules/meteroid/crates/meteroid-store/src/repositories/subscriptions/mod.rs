use crate::domain::{
    CreateSubscription, CreatedSubscription, CursorPaginatedVec, CursorPaginationRequest,
    PaginatedVec, PaginationRequest, Subscription, SubscriptionComponent, SubscriptionComponentNew,
    SubscriptionDetails, SubscriptionInvoiceCandidate,
};
use crate::{domain, StoreResult};
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, PlanId, SubscriptionId, TenantId};

pub mod internal;
mod slots;
pub use slots::SubscriptionSlotsInterface;
mod context;
mod payment_method;
mod subscriptions;
mod utils;

pub use utils::subscription_to_draft;

pub enum CancellationEffectiveAt {
    EndOfBillingPeriod,
    Date(NaiveDate),
}

#[async_trait::async_trait]
pub trait SubscriptionInterface {
    async fn insert_subscription(
        &self,
        subscription: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription>;

    async fn insert_subscription_batch(
        &self,
        batch: Vec<CreateSubscription>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CreatedSubscription>>;

    async fn get_subscription_details(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails>;

    async fn get_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription>;

    async fn insert_subscription_components(
        &self,
        tenant_id: TenantId,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>>;

    async fn cancel_subscription(
        &self,
        subscription_id: SubscriptionId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        context: domain::TenantContext,
    ) -> StoreResult<Subscription>;

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>>;

    async fn list_subscription_invoice_candidates(
        &self,
        date: NaiveDate,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<SubscriptionInvoiceCandidate>>;
}

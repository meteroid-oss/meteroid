//! What a replay of a sandbox's past billing does at past moments, and only in a sandbox: a
//! production tenant's history is never rewritten after the fact.
use crate::domain::coupons::{Coupon, CouponNew};
use crate::domain::entity_activity::Actor;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    CouponCreditRedemption, CreateSubscription, CreatedSubscription, DetailedInvoice,
};
use crate::errors::StoreError;
use crate::repositories::coupons::CouponInterface;
use crate::services::credit::wallet_purchase::ReplayedTopUp;
use crate::services::{InvoiceDelivery, ServicesEdge};
use crate::{Store, StoreResult};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    CustomerId, CustomerPaymentMethodId, InvoiceId, SubscriptionId, TenantId, WalletId,
};
use diesel_models::enums::TenantEnvironmentEnum;
use diesel_models::tenants::TenantRow;
use error_stack::Report;

#[async_trait::async_trait]
pub trait SandboxReplayInterface {
    /// The subscription started at `start`: activated, its first period issued and its coupons
    /// redeemed then.
    async fn insert_subscription_started_at(
        &self,
        actor: Actor,
        subscription: CreateSubscription,
        tenant_id: TenantId,
        start: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<(CreatedSubscription, Option<DetailedInvoice>)>;

    /// The card charged at `at` for what is due on the invoice, the gateway's answer recorded then.
    async fn charge_invoice_at(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<PaymentTransaction>;

    /// A promotion code redeemed at `at`: it must have existed and been valid then.
    async fn redeem_coupon_credit_code_at(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        at: DateTime<Utc>,
    ) -> StoreResult<CouponCreditRedemption>;

    /// `redeem_coupon_credit_code_at` naming the subscription a subscription-scoped code pays.
    async fn redeem_coupon_credit_code_for_at(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        subscription_id: Option<SubscriptionId>,
        at: DateTime<Utc>,
    ) -> StoreResult<CouponCreditRedemption>;

    /// The wallet's automatic top-up charged at `at`, recorded as the executor would have.
    async fn replay_automatic_top_up_at(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
        amount_minor: i64,
        at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<ReplayedTopUp>;

    /// A coupon in the catalog since `at`.
    async fn create_coupon_at(
        &self,
        actor: Actor,
        coupon: CouponNew,
        at: DateTime<Utc>,
    ) -> StoreResult<Coupon>;
}

async fn ensure_sandbox(store: &Store, tenant_id: TenantId) -> StoreResult<()> {
    let mut conn = store.get_conn().await?;
    let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if tenant.environment != TenantEnvironmentEnum::Sandbox {
        return Err(Report::new(StoreError::InvalidArgument(
            "only a sandbox's past billing is replayed".to_owned(),
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl SandboxReplayInterface for ServicesEdge {
    async fn insert_subscription_started_at(
        &self,
        actor: Actor,
        subscription: CreateSubscription,
        tenant_id: TenantId,
        start: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<(CreatedSubscription, Option<DetailedInvoice>)> {
        ensure_sandbox(&self.store, tenant_id).await?;
        self.start_replayed_subscription(actor, subscription, tenant_id, start, delivery)
            .await
    }

    async fn charge_invoice_at(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<PaymentTransaction> {
        ensure_sandbox(&self.store, tenant_id).await?;
        self.services
            .charge_invoice_at(tenant_id, invoice_id, payment_method_id, at, delivery)
            .await
    }

    async fn redeem_coupon_credit_code_at(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        at: DateTime<Utc>,
    ) -> StoreResult<CouponCreditRedemption> {
        ensure_sandbox(&self.store, tenant_id).await?;
        self.redeem_coupon_credit_code_on(tenant_id, customer_id, code, None, at, true)
            .await
    }

    async fn redeem_coupon_credit_code_for_at(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        code: &str,
        subscription_id: Option<SubscriptionId>,
        at: DateTime<Utc>,
    ) -> StoreResult<CouponCreditRedemption> {
        ensure_sandbox(&self.store, tenant_id).await?;
        self.redeem_coupon_credit_code_on(tenant_id, customer_id, code, subscription_id, at, true)
            .await
    }

    async fn replay_automatic_top_up_at(
        &self,
        tenant_id: TenantId,
        wallet_id: WalletId,
        amount_minor: i64,
        at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<ReplayedTopUp> {
        ensure_sandbox(&self.store, tenant_id).await?;
        self.services
            .replay_automatic_top_up(tenant_id, wallet_id, amount_minor, at, delivery)
            .await
    }

    async fn create_coupon_at(
        &self,
        actor: Actor,
        coupon: CouponNew,
        at: DateTime<Utc>,
    ) -> StoreResult<Coupon> {
        let tenant_id = coupon.tenant_id;
        ensure_sandbox(&self.store, tenant_id).await?;
        if at > common_domain::clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a coupon cannot be created at a future moment".to_owned(),
            )));
        }
        let created = self.store.create_coupon(actor, coupon).await?;
        let mut conn = self.store.get_conn().await?;
        diesel_models::coupons::CouponRow::set_created_at(
            &mut conn,
            tenant_id,
            created.id,
            at.naive_utc(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(Coupon {
            created_at: at.naive_utc(),
            ..created
        })
    }
}

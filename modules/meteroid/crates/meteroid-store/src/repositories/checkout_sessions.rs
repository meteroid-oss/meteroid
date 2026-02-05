use crate::domain::checkout_sessions::CheckoutSessionStatus;
use crate::domain::{CheckoutSession, CreateCheckoutSession};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use chrono::{Duration, Utc};
use common_domain::ids::{CheckoutSessionId, CustomerId, SubscriptionId, TenantId};
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use diesel_models::enums::CheckoutSessionStatusEnum;
use error_stack::Report;

#[async_trait::async_trait]
pub trait CheckoutSessionsInterface {
    async fn create_checkout_session(
        &self,
        params: CreateCheckoutSession,
    ) -> StoreResult<CheckoutSession>;

    async fn get_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession>;

    async fn get_checkout_session_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<CheckoutSession>;

    async fn list_checkout_sessions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatus>,
    ) -> StoreResult<Vec<CheckoutSession>>;

    async fn cancel_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession>;

    async fn expire_sessions(&self) -> StoreResult<usize>;

    async fn cleanup_old_sessions(&self, older_than_days: u32) -> StoreResult<usize>;
}

#[async_trait::async_trait]
impl CheckoutSessionsInterface for Store {
    async fn create_checkout_session(
        &self,
        params: CreateCheckoutSession,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row_new: CheckoutSessionRowNew = params.try_into_row()?;

        let row = row_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn get_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn get_checkout_session_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::get_by_subscription(&mut conn, tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "No checkout session found for this subscription".to_string(),
                ))
            })?;

        Ok(row.into())
    }

    async fn list_checkout_sessions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatus>,
    ) -> StoreResult<Vec<CheckoutSession>> {
        let mut conn = self.get_conn().await?;

        let db_status = status.map(status_domain_to_db);

        let rows = CheckoutSessionRow::list(&mut conn, tenant_id, customer_id, db_status)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn cancel_checkout_session(
        &self,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> StoreResult<CheckoutSession> {
        let mut conn = self.get_conn().await?;

        let row = CheckoutSessionRow::mark_cancelled(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Checkout session not found or cannot be cancelled".to_string(),
                ))
            })?;

        Ok(row.into())
    }

    async fn expire_sessions(&self) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        let now = Utc::now();

        CheckoutSessionRow::mark_expired_batch(&mut conn, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn cleanup_old_sessions(&self, older_than_days: u32) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        let older_than = Utc::now() - Duration::days(older_than_days as i64);

        CheckoutSessionRow::delete_old(&mut conn, older_than)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

fn status_domain_to_db(status: CheckoutSessionStatus) -> CheckoutSessionStatusEnum {
    match status {
        CheckoutSessionStatus::Created => CheckoutSessionStatusEnum::Created,
        CheckoutSessionStatus::AwaitingPayment => CheckoutSessionStatusEnum::AwaitingPayment,
        CheckoutSessionStatus::Completed => CheckoutSessionStatusEnum::Completed,
        CheckoutSessionStatus::Expired => CheckoutSessionStatusEnum::Expired,
        CheckoutSessionStatus::Cancelled => CheckoutSessionStatusEnum::Cancelled,
    }
}

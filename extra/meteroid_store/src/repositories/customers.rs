use crate::store::Store;
use crate::{domain, StoreResult};

#[async_trait::async_trait]
pub trait CustomersInterface {
    async fn insert_customer_batch(
        &self,
        batch: Vec<domain::CustomerNew>,
    ) -> StoreResult<Vec<domain::Customer>>;
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn insert_customer_batch(
        &self,
        batch: Vec<domain::CustomerNew>,
    ) -> StoreResult<Vec<domain::Customer>> {
        let mut conn = self.get_conn().await?;

        let insertable_batch: Vec<diesel_models::customers::CustomerNew> =
            batch.into_iter().map(|c| c.into()).collect();

        diesel_models::customers::Customer::insert_customer_batch(&mut conn, insertable_batch)
            .await
            .map_err(Into::into)
            .map(|v| v.into_iter().map(Into::into).collect())
    }
}

use crate::StoreResult;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait CustomerWalletsInterface {
    // up and down for negative cents
    async fn top_up_customer_balance(&self, customer_id: Uuid, cents: i32) -> StoreResult<()>;
}

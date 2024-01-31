use std::future::Future;
use tonic::Status;
use uuid::Uuid;

pub mod api_key_strategy;
pub mod jwt_strategy;

pub trait AuthStrategy {
    type Future: Future<Output = Result<String, Status>> + Send;

    fn authenticate(&self) -> Self::Future;
}

pub enum AuthenticatedState {
    ApiKey { id: Uuid, tenant_id: Uuid },
    User { id: Uuid },
}

pub enum AuthorizedState {
    Tenant { actor_id: Uuid, tenant_id: Uuid },
    User { user_id: Uuid },
}

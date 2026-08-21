use crate::errors::LeaderError;

/// Provider-agnostic single-leader election (Redis impl backs `is_held` with TTL renewal; Postgres with connection liveness).
#[async_trait::async_trait]
pub trait LeaderElection: Send + Sync {
    /// Try to become leader without blocking. Some(guard) if acquired; None if held elsewhere.
    async fn try_acquire(&self) -> Result<Option<Box<dyn LeaderGuard>>, LeaderError>;
}

#[async_trait::async_trait]
pub trait LeaderGuard: Send {
    /// Still leader? Renews the lease / probes liveness; false => lost leadership.
    async fn is_held(&mut self) -> bool;
    async fn release(self: Box<Self>);
}

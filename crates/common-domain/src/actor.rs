use crate::ids::{ApiTokenId, BaseId, CustomerId, UserId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Actor {
    System,
    User { id: UserId },
    ApiToken { id: ApiTokenId },
    Customer { id: CustomerId },
    QuoteRecipient { email: String },
}

/// Matches the Postgres `ActorTypeEnum` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    System,
    User,
    ApiToken,
    Customer,
    QuoteRecipient,
}

impl Actor {
    pub fn actor_type(&self) -> ActorType {
        match self {
            Actor::System => ActorType::System,
            Actor::User { .. } => ActorType::User,
            Actor::ApiToken { .. } => ActorType::ApiToken,
            Actor::Customer { .. } => ActorType::Customer,
            Actor::QuoteRecipient { .. } => ActorType::QuoteRecipient,
        }
    }

    /// Free-form identity for actors that aren't UUID-keyed. Currently only
    /// `QuoteRecipient` (its email); paired with `as_uuid()` returning None.
    pub fn actor_alias(&self) -> Option<String> {
        match self {
            Actor::QuoteRecipient { email } => Some(email.clone()),
            Actor::System
            | Actor::User { .. }
            | Actor::ApiToken { .. }
            | Actor::Customer { .. } => None,
        }
    }

    pub fn is_customer(&self) -> bool {
        matches!(self, Actor::Customer { .. })
    }

    /// Returns None for System and QuoteRecipient.
    pub fn as_uuid(&self) -> Option<Uuid> {
        match self {
            Actor::System | Actor::QuoteRecipient { .. } => None,
            Actor::User { id } => Some(id.as_uuid()),
            Actor::ApiToken { id } => Some(id.as_uuid()),
            Actor::Customer { id } => Some(id.as_uuid()),
        }
    }
}

use crate::ids::{ApiTokenId, BaseId, CustomerId, OAuthAppId, UserId};
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
    OAuthApp { id: OAuthAppId },
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
    OAuthApp,
}

impl Actor {
    pub fn actor_type(&self) -> ActorType {
        match self {
            Actor::System => ActorType::System,
            Actor::User { .. } => ActorType::User,
            Actor::ApiToken { .. } => ActorType::ApiToken,
            Actor::Customer { .. } => ActorType::Customer,
            Actor::QuoteRecipient { .. } => ActorType::QuoteRecipient,
            Actor::OAuthApp { .. } => ActorType::OAuthApp,
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
            | Actor::Customer { .. }
            | Actor::OAuthApp { .. } => None,
        }
    }

    pub fn is_customer(&self) -> bool {
        matches!(self, Actor::Customer { .. })
    }

    /// Rebuild an `Actor` from its persisted columns (see the audit / scheduled-event
    /// tables). A UUID-keyed type with a missing uuid, or a `QuoteRecipient` with no
    /// alias, degrades to `System` rather than fabricating an empty id.
    pub fn from_parts(actor_type: ActorType, uuid: Option<Uuid>, alias: Option<String>) -> Actor {
        match actor_type {
            ActorType::System => Actor::System,
            ActorType::User => uuid
                .map(|id| Actor::User { id: id.into() })
                .unwrap_or(Actor::System),
            ActorType::ApiToken => uuid
                .map(|id| Actor::ApiToken { id: id.into() })
                .unwrap_or(Actor::System),
            ActorType::Customer => uuid
                .map(|id| Actor::Customer { id: id.into() })
                .unwrap_or(Actor::System),
            ActorType::OAuthApp => uuid
                .map(|id| Actor::OAuthApp { id: id.into() })
                .unwrap_or(Actor::System),
            ActorType::QuoteRecipient => alias
                .map(|email| Actor::QuoteRecipient { email })
                .unwrap_or(Actor::System),
        }
    }

    /// Returns None for System and QuoteRecipient.
    pub fn as_uuid(&self) -> Option<Uuid> {
        match self {
            Actor::System | Actor::QuoteRecipient { .. } => None,
            Actor::User { id } => Some(id.as_uuid()),
            Actor::ApiToken { id } => Some(id.as_uuid()),
            Actor::Customer { id } => Some(id.as_uuid()),
            Actor::OAuthApp { id } => Some(id.as_uuid()),
        }
    }
}

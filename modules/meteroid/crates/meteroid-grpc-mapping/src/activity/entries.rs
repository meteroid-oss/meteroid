//! Activity entries as the gRPC API shows them, shared by the entity timeline and the audit log.

use super::error::ActivityApiError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common_domain::ids::{CustomerId, SubscriptionId};
use common_domain::ids::{OrganizationId, TenantId, UserId};
use meteroid_app::conversions::ProtoConv;
use meteroid_app::services::support::SUPPORT_ACTOR_NAME;
use meteroid_grpc::meteroid::api::activity::v1::{ActivityEntry, ActorType as ProtoActorType};
use meteroid_store::Store;
use meteroid_store::domain::entity_activity::{
    ActorType, EntityActivity, EntityType, actor_id_as_proto,
};
use meteroid_store::repositories::entity_activity::{
    ACTIVITY_DEFAULT_LIMIT, ACTIVITY_MAX_LIMIT, ActivityCursor, EntityActivityInterface,
    EntityActivityInterfaceResolveEntities,
};
use meteroid_store::repositories::users::UserInterface;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use tonic::Status;
use uuid::Uuid;

pub fn actor_type_to_proto(a: ActorType) -> ProtoActorType {
    match a {
        ActorType::System => ProtoActorType::System,
        ActorType::User => ProtoActorType::User,
        ActorType::ApiToken => ProtoActorType::ApiToken,
        ActorType::Customer => ProtoActorType::Customer,
        ActorType::QuoteRecipient => ProtoActorType::QuoteRecipient,
        ActorType::OAuthApp => ProtoActorType::OauthApp,
    }
}

pub fn proto_actor_type_to_domain(a: ProtoActorType) -> Option<ActorType> {
    match a {
        ProtoActorType::Unspecified => None,
        ProtoActorType::System => Some(ActorType::System),
        ProtoActorType::User => Some(ActorType::User),
        ProtoActorType::ApiToken => Some(ActorType::ApiToken),
        ProtoActorType::Customer => Some(ActorType::Customer),
        ProtoActorType::QuoteRecipient => Some(ActorType::QuoteRecipient),
        ProtoActorType::OauthApp => Some(ActorType::OAuthApp),
    }
}

pub fn parse_entity_type(raw: &str) -> Result<EntityType, Status> {
    EntityType::from_str(raw)
        .map_err(|_| Status::invalid_argument(format!("unknown entity_type: {raw}")))
}

type ActorNames = HashMap<(ActorType, Uuid), String>;
type SupportActors = HashSet<UserId>;
type EntityNames = HashMap<(EntityType, Uuid), String>;

pub fn collect_actor_uuids(entries: &[EntityActivity]) -> Vec<(ActorType, Uuid)> {
    entries
        .iter()
        .filter_map(|e| e.actor_uuid.map(|u| (e.actor_type, u)))
        .collect()
}

pub fn collect_entity_refs(entries: &[EntityActivity]) -> Vec<(EntityType, Uuid)> {
    let mut out = Vec::with_capacity(entries.len() * 3);
    for e in entries {
        if let Ok(et) = EntityType::from_str(&e.entity_type) {
            out.push((et, e.entity_id));
        }
        if let Some(c) = e.agg_customer_id {
            out.push((EntityType::Customer, c));
        }
        if let Some(s) = e.agg_subscription_id {
            out.push((EntityType::Subscription, s));
        }
    }
    out
}

pub fn to_proto_entry(
    a: EntityActivity,
    actor_names: &ActorNames,
    support_actors: &SupportActors,
    entity_names: &EntityNames,
) -> ActivityEntry {
    let parsed_entity_type = EntityType::from_str(&a.entity_type).ok();
    let entity_id = match parsed_entity_type {
        Some(et) => et.id_as_proto(a.entity_id),
        None => a.entity_id.to_string(),
    };
    let entity_name =
        parsed_entity_type.and_then(|et| entity_names.get(&(et, a.entity_id)).cloned());
    let entity_deleted =
        parsed_entity_type.is_some_and(|et| et.resolves_display_name() && entity_name.is_none());

    let is_support = a.actor_type == ActorType::User
        && a.actor_uuid
            .is_some_and(|u| support_actors.contains(&UserId::from(u)));
    let actor_id_typed = if is_support {
        None
    } else {
        actor_id_as_proto(a.actor_type, a.actor_uuid, a.actor_alias.as_deref())
    };
    let actor_name = match a.actor_type {
        ActorType::System => Some("System".to_string()),
        ActorType::QuoteRecipient => a.actor_alias.clone(),
        _ if is_support => Some(SUPPORT_ACTOR_NAME.to_string()),
        _ => a
            .actor_uuid
            .and_then(|u| actor_names.get(&(a.actor_type, u)).cloned()),
    };

    let agg_customer_name = a
        .agg_customer_id
        .and_then(|u| entity_names.get(&(EntityType::Customer, u)).cloned());
    let agg_subscription_name = a
        .agg_subscription_id
        .and_then(|u| entity_names.get(&(EntityType::Subscription, u)).cloned());

    ActivityEntry {
        id: a.id.as_proto(),
        entity_type: a.entity_type,
        entity_id,
        activity_type: a.activity_type,
        actor_type: actor_type_to_proto(a.actor_type) as i32,
        actor_id: actor_id_typed,
        actor_name,
        metadata_json: a.metadata.map(|m| m.to_string()),
        occurred_at: a.occurred_at.as_proto(),
        agg_customer_id: a.agg_customer_id.map(|u| CustomerId::from(u).as_proto()),
        agg_subscription_id: a
            .agg_subscription_id
            .map(|u| SubscriptionId::from(u).as_proto()),
        entity_name,
        entity_deleted,
        agg_customer_name,
        agg_subscription_name,
    }
}

// base64url(occurred_at_micros_i64_be || uuid_bytes). Opaque to clients.
pub fn encode_cursor(c: ActivityCursor) -> String {
    let micros = c.occurred_at.and_utc().timestamp_micros();
    let mut buf = [0u8; 8 + 16];
    buf[..8].copy_from_slice(&micros.to_be_bytes());
    buf[8..].copy_from_slice(c.id.as_bytes());
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn decode_cursor(s: &str) -> Result<ActivityCursor, Status> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| Status::invalid_argument("Invalid cursor"))?;
    if bytes.len() != 24 {
        return Err(Status::invalid_argument("Invalid cursor"));
    }
    let micros = i64::from_be_bytes(bytes[..8].try_into().unwrap());
    let occurred_at = chrono::DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| Status::invalid_argument("Invalid cursor"))?
        .naive_utc();
    let id =
        Uuid::from_slice(&bytes[8..]).map_err(|_| Status::invalid_argument("Invalid cursor"))?;
    Ok(ActivityCursor { occurred_at, id })
}

pub fn resolve_limit(req_limit: Option<u32>) -> u32 {
    req_limit
        .unwrap_or(ACTIVITY_DEFAULT_LIMIT)
        .min(ACTIVITY_MAX_LIMIT)
}

pub async fn resolve_support_actors(
    store: &Store,
    org_id: OrganizationId,
    entries: &[EntityActivity],
) -> Result<SupportActors, Status> {
    let user_ids: Vec<UserId> = entries
        .iter()
        .filter(|e| e.actor_type == ActorType::User)
        .filter_map(|e| e.actor_uuid.map(UserId::from))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    store
        .support_staff_outside_org(org_id, &user_ids)
        .await
        .map_err(|e| ActivityApiError::from(e).into())
}

/// A page of activity as proto entries, with actor, support-staff and entity names resolved.
pub async fn entries_to_proto(
    store: &Store,
    tenant_id: TenantId,
    org_id: OrganizationId,
    items: Vec<EntityActivity>,
) -> Result<Vec<ActivityEntry>, Status> {
    let actor_names = store
        .resolve_actor_names(tenant_id, &collect_actor_uuids(&items))
        .await
        .map_err(Into::<ActivityApiError>::into)?;
    let support_actors = resolve_support_actors(store, org_id, &items).await?;
    let entity_names = store
        .resolve_entity_names(tenant_id, &collect_entity_refs(&items))
        .await
        .map_err(Into::<ActivityApiError>::into)?;
    Ok(items
        .into_iter()
        .map(|a| to_proto_entry(a, &actor_names, &support_actors, &entity_names))
        .collect())
}

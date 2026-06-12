use super::ActivityServiceComponents;
use super::error::ActivityApiError;
use crate::api::sharable::generate_entity_share_key;
use crate::api::shared::conversions::ProtoConv;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common_domain::ids::{BaseId, EntityActivityId};
use common_domain::ids::{CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::activity::v1::activity_service_server::ActivityService;
use meteroid_grpc::meteroid::api::activity::v1::{
    ActivityEntry, ActorType as ProtoActorType, GetSentEmailRequest, GetSentEmailResponse,
    ListActivityRequest, ListActivityResponse, ListEntityActivityRequest,
    ListEntityActivityResponse, SentEmail as ProtoSentEmail,
    SentEmailAttachment as ProtoSentEmailAttachment,
};
use meteroid_store::domain::entity_activity::{
    ActorType, EntityActivity, EntityType, actor_id_as_proto,
};
use meteroid_store::repositories::entity_activity::{
    ACTIVITY_DEFAULT_LIMIT, ACTIVITY_MAX_LIMIT, ActivityCursor, EntityActivityFilter,
    EntityActivityInterface, EntityActivityInterfaceEmail, EntityActivityInterfaceResolveEntities,
};
use std::collections::HashMap;
use std::str::FromStr;
use tonic::{Request, Response, Status};
use uuid::Uuid;

fn actor_type_to_proto(a: ActorType) -> ProtoActorType {
    match a {
        ActorType::System => ProtoActorType::System,
        ActorType::User => ProtoActorType::User,
        ActorType::ApiToken => ProtoActorType::ApiToken,
        ActorType::Customer => ProtoActorType::Customer,
        ActorType::QuoteRecipient => ProtoActorType::QuoteRecipient,
    }
}

fn proto_actor_type_to_domain(a: ProtoActorType) -> Option<ActorType> {
    match a {
        ProtoActorType::Unspecified => None,
        ProtoActorType::System => Some(ActorType::System),
        ProtoActorType::User => Some(ActorType::User),
        ProtoActorType::ApiToken => Some(ActorType::ApiToken),
        ProtoActorType::Customer => Some(ActorType::Customer),
        ProtoActorType::QuoteRecipient => Some(ActorType::QuoteRecipient),
    }
}

fn parse_entity_type(raw: &str) -> Result<EntityType, Status> {
    EntityType::from_str(raw)
        .map_err(|_| Status::invalid_argument(format!("unknown entity_type: {raw}")))
}

type ActorNames = HashMap<(ActorType, Uuid), String>;
type EntityNames = HashMap<(EntityType, Uuid), String>;

fn collect_actor_uuids(entries: &[EntityActivity]) -> Vec<(ActorType, Uuid)> {
    entries
        .iter()
        .filter_map(|e| e.actor_uuid.map(|u| (e.actor_type, u)))
        .collect()
}

fn collect_entity_refs(entries: &[EntityActivity]) -> Vec<(EntityType, Uuid)> {
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

fn to_proto_entry(
    a: EntityActivity,
    actor_names: &ActorNames,
    entity_names: &EntityNames,
) -> ActivityEntry {
    let parsed_entity_type = EntityType::from_str(&a.entity_type).ok();
    let entity_id = match parsed_entity_type {
        Some(et) => et.id_as_proto(a.entity_id),
        None => a.entity_id.to_string(),
    };
    let entity_name =
        parsed_entity_type.and_then(|et| entity_names.get(&(et, a.entity_id)).cloned());

    let actor_id_typed = actor_id_as_proto(a.actor_type, a.actor_uuid, a.actor_alias.as_deref());
    let actor_name = match a.actor_type {
        ActorType::System => Some("System".to_string()),
        ActorType::QuoteRecipient => a.actor_alias.clone(),
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
        agg_customer_name,
        agg_subscription_name,
    }
}

// base64url(occurred_at_micros_i64_be || uuid_bytes). Opaque to clients.
fn encode_cursor(c: ActivityCursor) -> String {
    let micros = c.occurred_at.and_utc().timestamp_micros();
    let mut buf = [0u8; 8 + 16];
    buf[..8].copy_from_slice(&micros.to_be_bytes());
    buf[8..].copy_from_slice(c.id.as_bytes());
    URL_SAFE_NO_PAD.encode(buf)
}

fn decode_cursor(s: &str) -> Result<ActivityCursor, Status> {
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

fn resolve_limit(req_limit: Option<u32>) -> u32 {
    req_limit
        .unwrap_or(ACTIVITY_DEFAULT_LIMIT)
        .min(ACTIVITY_MAX_LIMIT)
}

#[tonic::async_trait]
impl ActivityService for ActivityServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_entity_activity(
        &self,
        request: Request<ListEntityActivityRequest>,
    ) -> Result<Response<ListEntityActivityResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        if req.entity_type.is_empty() || req.entity_id.is_empty() {
            return Err(Status::invalid_argument(
                "entity_type and entity_id are required",
            ));
        }

        let entity_type = parse_entity_type(&req.entity_type)?;
        let entity_id = entity_type
            .parse_id_proto(&req.entity_id)
            .map_err(|e| Status::invalid_argument(format!("invalid entity_id: {e}")))?;

        let before = req.cursor.as_deref().map(decode_cursor).transpose()?;
        let limit = resolve_limit(req.limit);

        let page = self
            .store
            .list_entity_activities(tenant_id, &req.entity_type, entity_id, before, limit)
            .await
            .map_err(Into::<ActivityApiError>::into)?;

        let actor_names = self
            .store
            .resolve_actor_names(tenant_id, &collect_actor_uuids(&page.items))
            .await
            .map_err(Into::<ActivityApiError>::into)?;
        let entity_names = self
            .store
            .resolve_entity_names(tenant_id, &collect_entity_refs(&page.items))
            .await
            .map_err(Into::<ActivityApiError>::into)?;
        Ok(Response::new(ListEntityActivityResponse {
            entries: page
                .items
                .into_iter()
                .map(|a| to_proto_entry(a, &actor_names, &entity_names))
                .collect(),
            next_cursor: page.next_cursor.map(encode_cursor),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_activity(
        &self,
        request: Request<ListActivityRequest>,
    ) -> Result<Response<ListActivityResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let entity_id = match (req.entity_type.as_deref(), req.entity_id.as_deref()) {
            (Some(_), None) | (None, Some(_)) => {
                return Err(Status::invalid_argument(
                    "entity_type and entity_id must be set together",
                ));
            }
            (Some(et), Some(eid)) => Some(
                parse_entity_type(et)?
                    .parse_id_proto(eid)
                    .map_err(|e| Status::invalid_argument(format!("invalid entity_id: {e}")))?,
            ),
            _ => None,
        };

        let actor_type = ProtoActorType::try_from(req.actor_type.unwrap_or(0))
            .ok()
            .and_then(proto_actor_type_to_domain);

        if req.actor_id.is_some() && actor_type.is_none() {
            return Err(Status::invalid_argument(
                "actor_id filter requires actor_type to also be set",
            ));
        }

        let occurred_after = req
            .occurred_after
            .map(chrono::NaiveDateTime::from_proto)
            .transpose()?;
        let occurred_before = req
            .occurred_before
            .map(chrono::NaiveDateTime::from_proto)
            .transpose()?;

        // The frontend sends typed ids; UUID-keyed actors filter on actor_uuid,
        // QuoteRecipient on its alias.
        let invalid_actor = || Status::invalid_argument("invalid actor_id");
        let (actor_uuid, actor_alias) = match (actor_type, req.actor_id) {
            (Some(ActorType::User), Some(raw)) => (
                Some(
                    common_domain::ids::UserId::from_str(&raw)
                        .map(|u| u.as_uuid())
                        .map_err(|_| invalid_actor())?,
                ),
                None,
            ),
            (Some(ActorType::ApiToken), Some(raw)) => (
                Some(
                    common_domain::ids::ApiTokenId::from_str(&raw)
                        .map(|t| t.as_uuid())
                        .map_err(|_| invalid_actor())?,
                ),
                None,
            ),
            (Some(ActorType::Customer), Some(raw)) => (
                Some(
                    CustomerId::from_str(&raw)
                        .map(|c| c.as_uuid())
                        .map_err(|_| invalid_actor())?,
                ),
                None,
            ),
            (Some(ActorType::QuoteRecipient), Some(raw)) => (None, Some(raw)),
            _ => (None, None),
        };

        let filter = EntityActivityFilter {
            entity_types: req.entity_types,
            activity_types: req.activity_types,
            actor_type,
            actor_uuid,
            actor_alias,
            entity_id,
            entity_type: req.entity_type,
            occurred_after,
            occurred_before,
            rollup_customer_id: None,
            rollup_subscription_id: None,
        };

        let before = req.cursor.as_deref().map(decode_cursor).transpose()?;
        let limit = resolve_limit(req.limit);

        let page = self
            .store
            .list_activities(tenant_id, filter, before, limit)
            .await
            .map_err(Into::<ActivityApiError>::into)?;

        let actor_names = self
            .store
            .resolve_actor_names(tenant_id, &collect_actor_uuids(&page.items))
            .await
            .map_err(Into::<ActivityApiError>::into)?;
        let entity_names = self
            .store
            .resolve_entity_names(tenant_id, &collect_entity_refs(&page.items))
            .await
            .map_err(Into::<ActivityApiError>::into)?;

        Ok(Response::new(ListActivityResponse {
            entries: page
                .items
                .into_iter()
                .map(|a| to_proto_entry(a, &actor_names, &entity_names))
                .collect(),
            next_cursor: page.next_cursor.map(encode_cursor),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_sent_email(
        &self,
        request: Request<GetSentEmailRequest>,
    ) -> Result<Response<GetSentEmailResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let activity_id = EntityActivityId::from_proto(&req.activity_id)
            .map_err(|e| Status::invalid_argument(format!("invalid activity_id: {e}")))?;

        let email = self
            .store
            .get_sent_email(tenant_id, activity_id)
            .await
            .map_err(Into::<ActivityApiError>::into)?;

        let attachments = parse_attachments(email.attachments.as_ref());
        // Only mint a token when something is actually downloadable (old rows
        // recorded filenames without an object id).
        let attachments_share_key = if attachments.iter().any(|a| !a.id.is_empty()) {
            let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
            Some(
                generate_entity_share_key(email.id.as_uuid(), tenant_id, &self.jwt_secret, exp)
                    .map_err(Into::<ActivityApiError>::into)?,
            )
        } else {
            None
        };

        Ok(Response::new(GetSentEmailResponse {
            email: Some(ProtoSentEmail {
                activity_id: email.id.as_base62(),
                subject: email.subject,
                from_addr: email.from_addr,
                reply_to: email.reply_to,
                recipients: email.recipients,
                body_html: email.body_html,
                attachments,
                sent_at: email.sent_at.as_proto(),
                attachments_share_key,
            }),
        }))
    }
}

/// Stored attachment JSON shape; `id`/`kind` are absent on rows written before
/// attachment object ids were tracked.
#[derive(serde::Deserialize)]
struct StoredAttachment {
    filename: String,
    #[serde(default)]
    id: Option<String>,
}

fn parse_attachments(value: Option<&serde_json::Value>) -> Vec<ProtoSentEmailAttachment> {
    let Some(value) = value else {
        return vec![];
    };
    serde_json::from_value::<Vec<StoredAttachment>>(value.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|a| ProtoSentEmailAttachment {
            id: a.id.unwrap_or_default(),
            filename: a.filename,
        })
        .collect()
}

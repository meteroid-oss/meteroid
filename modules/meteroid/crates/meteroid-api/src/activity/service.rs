use super::ActivityServiceComponents;
use super::error::ActivityApiError;
use common_domain::ids::{BaseId, EntityActivityId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::ProtoConv;
use meteroid_app::sharable::generate_entity_share_key;
use meteroid_grpc::meteroid::api::activity::v1::activity_service_server::ActivityService;
use meteroid_grpc::meteroid::api::activity::v1::{
    GetSentEmailRequest, GetSentEmailResponse, ListEntityActivityRequest,
    ListEntityActivityResponse, SentEmail as ProtoSentEmail,
    SentEmailAttachment as ProtoSentEmailAttachment,
};
use meteroid_grpc_mapping::activity::entries::*;
use meteroid_store::repositories::entity_activity::{
    EntityActivityInterface, EntityActivityInterfaceEmail,
};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl ActivityService for ActivityServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_entity_activity(
        &self,
        request: Request<ListEntityActivityRequest>,
    ) -> Result<Response<ListEntityActivityResponse>, Status> {
        let tenant_id = request.tenant()?;
        let org_id = request.organization()?;
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

        Ok(Response::new(ListEntityActivityResponse {
            entries: entries_to_proto(&self.store, tenant_id, org_id, page.items).await?,
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

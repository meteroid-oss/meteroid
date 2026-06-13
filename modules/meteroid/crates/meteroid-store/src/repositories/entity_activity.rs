use crate::StoreResult;
use crate::domain::entity_activity::{Activity, Actor, AuditInput, EntityActivity, build_row_new};
use crate::domain::outbox_event::OutboxEvent;
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};

use common_domain::actor::ActorType;
use common_domain::ids::{
    AliasOr, ApiTokenId, BaseId, CustomerId, EntityActivityId, StoredDocumentId, SubscriptionId,
    TenantId,
};
use diesel_models::api_tokens::ApiTokenRow;
use diesel_models::customers::CustomerRow;
use diesel_models::entity_activity::EntityActivityRow;
pub use diesel_models::query::entity_activity::{ActivityCursor, EntityActivityFilter};
use diesel_models::sent_email::{SentEmailRow, SentEmailRowNew};
use diesel_models::users::UserRow;
use scoped_futures::ScopedFutureExt;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Which object-store bucket an attachment lives in. Maps to a `Prefix` when
/// the bytes are served back; serialized form is stable (it is persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentEmailAttachmentKind {
    InvoicePdf,
    ReceiptPdf,
    CreditNotePdf,
}

/// One stored attachment recorded against a sent email: enough to fetch the
/// exact object that was sent (not the entity's current PDF).
#[derive(Debug, Clone)]
pub struct SentEmailAttachment {
    pub filename: String,
    pub id: StoredDocumentId,
    pub kind: SentEmailAttachmentKind,
}

/// Persisted as one `sent_email` row plus one matching `entity.email_sent` activity
/// sharing the same UUID.
#[derive(Debug, Clone)]
pub struct SentEmailNew {
    pub tenant_id: TenantId,
    pub entity_type: crate::domain::entity_activity::EntityType,
    pub entity_id: Uuid,
    pub agg_customer_id: Option<CustomerId>,
    pub agg_subscription_id: Option<SubscriptionId>,
    pub kind: String,
    pub subject: String,
    pub from_addr: String,
    pub reply_to: Option<String>,
    pub recipients: Vec<String>,
    pub body_html: String,
    pub attachments: Vec<SentEmailAttachment>,
}

#[derive(Debug, Clone)]
pub struct SentEmail {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub sent_at: chrono::NaiveDateTime,
    pub subject: String,
    pub from_addr: String,
    pub reply_to: Option<String>,
    pub recipients: Vec<String>,
    pub body_html: String,
    pub attachments: Option<serde_json::Value>,
}

impl From<SentEmailRow> for SentEmail {
    fn from(r: SentEmailRow) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            sent_at: r.sent_at,
            subject: r.subject,
            from_addr: r.from_addr,
            reply_to: r.reply_to,
            // diesel forces Option<> on TEXT[] elements; column is NOT NULL.
            recipients: r.recipients.into_iter().flatten().collect(),
            body_html: r.body_html,
            attachments: r.attachments,
        }
    }
}

pub const ACTIVITY_DEFAULT_LIMIT: u32 = 50;
pub const ACTIVITY_MAX_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
pub struct ActivityPage {
    pub items: Vec<EntityActivity>,
    pub next_cursor: Option<ActivityCursor>,
}

#[async_trait::async_trait]
pub trait EntityActivityInterface {
    /// Use `record_tx` from inside an existing transaction.
    async fn record(&self, tenant_id: TenantId, actor: Actor, input: AuditInput)
    -> StoreResult<()>;

    async fn record_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        input: AuditInput,
    ) -> StoreResult<()>;

    /// For `customer`/`subscription`, also returns rows pointing at the entity
    /// via `agg_customer_id` / `agg_subscription_id`.
    async fn list_entity_activities(
        &self,
        tenant_id: TenantId,
        entity_type: &str,
        entity_id: Uuid,
        before: Option<ActivityCursor>,
        limit: u32,
    ) -> StoreResult<ActivityPage>;

    async fn list_activities(
        &self,
        tenant_id: TenantId,
        filter: EntityActivityFilter,
        before: Option<ActivityCursor>,
        limit: u32,
    ) -> StoreResult<ActivityPage>;

    /// Batched per actor_type (users / api_tokens / customers). Keyed on the
    /// actor UUID — `System`/`QuoteRecipient` have no UUID and aren't resolved here.
    async fn resolve_actor_names(
        &self,
        tenant_id: TenantId,
        actors: &[(ActorType, Uuid)],
    ) -> StoreResult<HashMap<(ActorType, Uuid), String>>;
}

#[async_trait::async_trait]
impl EntityActivityInterface for Store {
    async fn record(
        &self,
        tenant_id: TenantId,
        actor: Actor,
        input: AuditInput,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        self.internal
            .record_audit_tx(&mut conn, tenant_id, &actor, input)
            .await
    }

    async fn record_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        input: AuditInput,
    ) -> StoreResult<()> {
        self.internal
            .record_audit_tx(conn, tenant_id, actor, input)
            .await
    }

    async fn list_entity_activities(
        &self,
        tenant_id: TenantId,
        entity_type: &str,
        entity_id: Uuid,
        before: Option<ActivityCursor>,
        limit: u32,
    ) -> StoreResult<ActivityPage> {
        let capped_limit = limit.clamp(1, ACTIVITY_MAX_LIMIT);
        let mut conn = self.get_conn().await?;

        let rows = match entity_type {
            "customer" | "subscription" => {
                let mut filter = EntityActivityFilter::default();
                if entity_type == "customer" {
                    filter.rollup_customer_id = Some(entity_id);
                } else {
                    filter.rollup_subscription_id = Some(entity_id);
                }
                EntityActivityRow::list_filtered(
                    &mut conn,
                    tenant_id,
                    &filter,
                    before,
                    fetch_n(capped_limit),
                )
                .await
            }
            _ => {
                EntityActivityRow::list_by_entity(
                    &mut conn,
                    tenant_id,
                    entity_type,
                    entity_id,
                    before,
                    fetch_n(capped_limit),
                )
                .await
            }
        }
        .map_err(Into::<error_stack::Report<StoreError>>::into)?;

        Ok(into_page(rows, capped_limit))
    }

    async fn list_activities(
        &self,
        tenant_id: TenantId,
        filter: EntityActivityFilter,
        before: Option<ActivityCursor>,
        limit: u32,
    ) -> StoreResult<ActivityPage> {
        let capped_limit = limit.clamp(1, ACTIVITY_MAX_LIMIT);
        let mut conn = self.get_conn().await?;
        let rows = EntityActivityRow::list_filtered(
            &mut conn,
            tenant_id,
            &filter,
            before,
            fetch_n(capped_limit),
        )
        .await
        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
        Ok(into_page(rows, capped_limit))
    }

    async fn resolve_actor_names(
        &self,
        tenant_id: TenantId,
        actors: &[(ActorType, Uuid)],
    ) -> StoreResult<HashMap<(ActorType, Uuid), String>> {
        let mut user_ids: HashSet<Uuid> = HashSet::new();
        let mut token_ids: HashSet<Uuid> = HashSet::new();
        let mut customer_ids: HashSet<CustomerId> = HashSet::new();
        for (actor_type, uuid) in actors {
            match actor_type {
                ActorType::User => {
                    user_ids.insert(*uuid);
                }
                ActorType::ApiToken => {
                    token_ids.insert(*uuid);
                }
                ActorType::Customer => {
                    customer_ids.insert(CustomerId::from(*uuid));
                }
                ActorType::System | ActorType::QuoteRecipient => {}
            }
        }

        let mut out: HashMap<(ActorType, Uuid), String> = HashMap::new();
        let mut conn = self.get_conn().await?;

        if !user_ids.is_empty() {
            let ids: Vec<Uuid> = user_ids.into_iter().collect();
            let rows = UserRow::find_by_ids(&mut conn, &ids)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            for u in &rows {
                out.insert((ActorType::User, *u.id), user_display(u));
            }
        }
        if !token_ids.is_empty() {
            let ids: Vec<ApiTokenId> = token_ids.into_iter().map(Into::into).collect();
            let rows = ApiTokenRow::find_by_ids(&mut conn, tenant_id, &ids)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            for t in &rows {
                out.insert((ActorType::ApiToken, *t.id), t.name.clone());
            }
        }
        if !customer_ids.is_empty() {
            let aliased: Vec<AliasOr<CustomerId>> =
                customer_ids.into_iter().map(AliasOr::Id).collect();
            let rows = CustomerRow::find_by_ids_or_aliases(&mut conn, tenant_id, aliased)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            for c in &rows {
                out.insert((ActorType::Customer, c.id.as_uuid()), c.name.clone());
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl EntityActivityInterfaceEmail for Store {
    async fn record_email_sent(
        &self,
        actor: Actor,
        sent: SentEmailNew,
    ) -> StoreResult<EntityActivityId> {
        let id = EntityActivityId::new();
        let tenant_id = sent.tenant_id;
        self.transaction(|conn| {
            let actor = &actor;
            let sent = &sent;
            async move {
                let activity_metadata = serde_json::json!({
                    "kind": sent.kind,
                    "subject": sent.subject,
                    "recipient_count": sent.recipients.len(),
                    "attachment_count": sent.attachments.len(),
                });
                let activity_row = diesel_models::entity_activity::EntityActivityRowNew {
                    id,
                    tenant_id,
                    entity_type: sent.entity_type.to_string(),
                    entity_id: sent.entity_id,
                    activity_type: crate::domain::entity_activity::ActivityType::EmailSent
                        .to_string(),
                    actor_type: actor.actor_type().into(),
                    actor_uuid: actor.as_uuid(),
                    actor_alias: actor.actor_alias(),
                    metadata: Some(activity_metadata),
                    agg_customer_id: sent.agg_customer_id.map(|c| c.as_uuid()),
                    agg_subscription_id: sent.agg_subscription_id.map(|s| s.as_uuid()),
                };
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;

                let email_row = SentEmailRowNew {
                    id,
                    tenant_id,
                    subject: sent.subject.clone(),
                    from_addr: sent.from_addr.clone(),
                    reply_to: sent.reply_to.clone(),
                    recipients: sent.recipients.iter().cloned().map(Some).collect(),
                    body_html: sent.body_html.clone(),
                    attachments: if sent.attachments.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!(
                            sent.attachments
                                .iter()
                                .map(|a| serde_json::json!({
                                    "filename": a.filename,
                                    "id": a.id.as_base62(),
                                    "kind": a.kind,
                                }))
                                .collect::<Vec<_>>()
                        ))
                    },
                };
                email_row
                    .insert(conn)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;

                Ok(id)
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_sent_email(
        &self,
        tenant_id: TenantId,
        id: EntityActivityId,
    ) -> StoreResult<SentEmail> {
        let mut conn = self.get_conn().await?;
        SentEmailRow::find_by_id(&mut conn, tenant_id, id)
            .await
            .map(SentEmail::from)
            .map_err(Into::<error_stack::Report<StoreError>>::into)
    }
}

#[async_trait::async_trait]
pub trait EntityActivityInterfaceEmail {
    async fn record_email_sent(
        &self,
        actor: Actor,
        sent: SentEmailNew,
    ) -> StoreResult<EntityActivityId>;

    async fn get_sent_email(
        &self,
        tenant_id: TenantId,
        id: EntityActivityId,
    ) -> StoreResult<SentEmail>;
}

#[async_trait::async_trait]
impl EntityActivityInterfaceResolveEntities for Store {
    async fn resolve_entity_names(
        &self,
        tenant_id: TenantId,
        refs: &[(crate::domain::entity_activity::EntityType, Uuid)],
    ) -> StoreResult<HashMap<(crate::domain::entity_activity::EntityType, Uuid), String>> {
        use crate::domain::entity_activity::EntityType;
        use diesel_models::query::entity_display as ed;

        let mut by_type: HashMap<EntityType, Vec<Uuid>> = HashMap::new();
        for (t, id) in refs {
            by_type.entry(*t).or_default().push(*id);
        }
        for v in by_type.values_mut() {
            v.sort();
            v.dedup();
        }

        let mut out: HashMap<(EntityType, Uuid), String> = HashMap::new();
        let mut conn = self.get_conn().await?;

        for (et, ids) in by_type {
            let rows: Vec<(Uuid, String)> = match et {
                EntityType::Customer => ed::customer_names(&mut conn, tenant_id, &ids).await,
                EntityType::Invoice => ed::invoice_numbers(&mut conn, tenant_id, &ids).await,
                EntityType::Quote => ed::quote_numbers(&mut conn, tenant_id, &ids).await,
                EntityType::Plan => ed::plan_names(&mut conn, tenant_id, &ids).await,
                EntityType::Product => ed::product_names(&mut conn, tenant_id, &ids).await,
                EntityType::AddOn => ed::add_on_names(&mut conn, tenant_id, &ids).await,
                EntityType::Coupon => ed::coupon_codes(&mut conn, tenant_id, &ids).await,
                EntityType::BillableMetric => {
                    ed::billable_metric_names(&mut conn, tenant_id, &ids).await
                }
                EntityType::CreditNote => ed::credit_note_numbers(&mut conn, tenant_id, &ids).await,
                EntityType::Subscription => {
                    ed::subscription_plan_names(&mut conn, tenant_id, &ids).await
                }
                _ => continue,
            }
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            for (id, name) in rows {
                out.insert((et, id), name);
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
pub trait EntityActivityInterfaceResolveEntities {
    async fn resolve_entity_names(
        &self,
        tenant_id: TenantId,
        refs: &[(crate::domain::entity_activity::EntityType, Uuid)],
    ) -> StoreResult<HashMap<(crate::domain::entity_activity::EntityType, Uuid), String>>;
}

fn user_display(u: &UserRow) -> String {
    let candidate = format!(
        "{} {}",
        u.first_name.as_deref().unwrap_or(""),
        u.last_name.as_deref().unwrap_or("")
    );
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        u.email.clone()
    } else {
        trimmed.to_string()
    }
}

// limit+1 to detect "has more" without a COUNT.
fn fetch_n(limit: u32) -> i64 {
    i64::from(limit) + 1
}

fn into_page(mut rows: Vec<EntityActivityRow>, limit: u32) -> ActivityPage {
    let next_cursor = if rows.len() > limit as usize {
        rows.truncate(limit as usize);
        rows.last().map(|r| ActivityCursor {
            occurred_at: r.occurred_at,
            id: r.id.as_uuid(),
        })
    } else {
        None
    };
    ActivityPage {
        items: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }
}

impl StoreInternal {
    pub async fn record_audit_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        input: AuditInput,
    ) -> StoreResult<()> {
        match input {
            AuditInput::Outbox(event) => {
                let activity: Option<Activity> = (&event).into();
                if let Some(activity) = activity {
                    build_row_new(tenant_id, actor, activity)
                        .insert(conn)
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                }
                self.insert_outbox_events_tx(conn, vec![event]).await?;
            }
            AuditInput::Activity(activity) => {
                build_row_new(tenant_id, actor, activity)
                    .insert(conn)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }
        }
        Ok(())
    }

    pub async fn record_outbox_batch_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        actor: &Actor,
        events: Vec<OutboxEvent>,
    ) -> StoreResult<()> {
        let rows: Vec<_> = events
            .iter()
            .filter_map(|event| {
                let activity: Option<Activity> = event.into();
                activity.map(|a| build_row_new(tenant_id, actor, a))
            })
            .collect();

        diesel_models::entity_activity::EntityActivityRowNew::insert_batch(&rows, conn)
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

        self.insert_outbox_events_tx(conn, events).await
    }
}

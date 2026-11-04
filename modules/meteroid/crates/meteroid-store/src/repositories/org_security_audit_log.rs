use crate::StoreResult;
use crate::domain::org_security_audit_log::{
    Actor, RecordSecurityEvent, SecurityEventType, SecurityRequestContext,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{BaseId, OrganizationId};
use diesel_models::org_security_audit_log::OrgSecurityAuditLogRowNew;
use error_stack::Report;

#[async_trait::async_trait]
pub trait SecurityAuditInterface {
    /// Record a single security event. Failures are returned to the caller —
    /// audit-log writes must succeed; an orphaned action without an audit
    /// trail defeats the point. Callers that don't want to abort the parent
    /// operation should call `record_best_effort` instead.
    async fn record_security_event(&self, event: RecordSecurityEvent) -> StoreResult<()>;

    /// Same as `record_security_event` but takes an existing connection so
    /// the event is written in the same transaction as the action it audits.
    async fn record_security_event_tx(
        &self,
        conn: &mut PgConn,
        event: RecordSecurityEvent,
    ) -> StoreResult<()>;

    /// Fire-and-log variant for callers that prefer "the action succeeded,
    /// the audit row didn't make it" over an aborted user-facing operation.
    /// Used for login/oauth where we don't want to fail a sign-in if the
    /// audit table is temporarily unreachable.
    async fn record_security_event_best_effort(&self, event: RecordSecurityEvent);
}

#[async_trait::async_trait]
impl SecurityAuditInterface for Store {
    async fn record_security_event(&self, event: RecordSecurityEvent) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        self.record_security_event_tx(&mut conn, event).await
    }

    async fn record_security_event_tx(
        &self,
        conn: &mut PgConn,
        event: RecordSecurityEvent,
    ) -> StoreResult<()> {
        build_row(event)
            .insert(conn)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn record_security_event_best_effort(&self, event: RecordSecurityEvent) {
        if let Err(err) = self.record_security_event(event).await {
            tracing::warn!(
                error = ?err,
                "failed to persist security audit event; the corresponding action still succeeded"
            );
        }
    }
}

fn build_row(ev: RecordSecurityEvent) -> OrgSecurityAuditLogRowNew {
    let RecordSecurityEvent {
        organization_id,
        event_type,
        actor,
        actor_email,
        target_user_id,
        target_email,
        metadata,
        context,
    } = ev;

    let SecurityRequestContext {
        ip_address,
        user_agent,
        ua_browser,
        ua_os,
        ua_device,
    } = context;

    OrgSecurityAuditLogRowNew {
        id: common_domain::ids::OrgSecurityAuditLogId::new(),
        organization_id,
        event_type: event_type.to_string(),
        actor_type: actor.actor_type().into(),
        actor_id: actor.as_uuid(),
        actor_email,
        target_user_id,
        target_email,
        metadata,
        ip_address,
        user_agent,
        ua_browser,
        ua_os,
        ua_device,
    }
}

/// Convenience for callers: build the `RecordSecurityEvent` payload without
/// having to spell out every Option/None.
pub fn event(
    organization_id: OrganizationId,
    event_type: SecurityEventType,
    actor: Actor,
) -> RecordSecurityEvent {
    RecordSecurityEvent {
        organization_id,
        event_type,
        actor,
        actor_email: None,
        target_user_id: None,
        target_email: None,
        metadata: None,
        context: SecurityRequestContext::default(),
    }
}

/// Record an auth-level event against every organization the user is a
/// member of. Used for sign-in / MFA-challenge / password-reset events,
/// which are user-scoped but matter to every org the user has access to —
/// each org admin needs the event in their compliance trail.
///
/// Best-effort: failures to enumerate or insert are logged but do not
/// propagate up. Auth flows must not fail because of audit-log issues.
pub async fn record_auth_event_for_user_orgs(
    store: &Store,
    user_id: common_domain::ids::UserId,
    user_email: &str,
    event_type: SecurityEventType,
    metadata: Option<serde_json::Value>,
    context: SecurityRequestContext,
) {
    let mut conn = match store.get_conn().await {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(
                ?err,
                "audit: could not get DB conn; dropping {event_type:?}"
            );
            return;
        }
    };

    let org_rows = match diesel_models::organizations::OrganizationRow::list_by_user_id(
        &mut conn,
        user_id.as_uuid(),
    )
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                ?err,
                "audit: could not list user's orgs; dropping {event_type:?}"
            );
            return;
        }
    };

    let actor = Actor::User { id: user_id };
    for org in org_rows {
        let row = build_row(RecordSecurityEvent {
            organization_id: org.id,
            event_type,
            actor: actor.clone(),
            actor_email: Some(user_email.to_string()),
            target_user_id: Some(user_id.as_uuid()),
            target_email: Some(user_email.to_string()),
            metadata: metadata.clone(),
            context: context.clone(),
        });
        if let Err(err) = row.insert(&mut conn).await {
            tracing::warn!(?err, org_id = %org.id, "audit: insert failed for {event_type:?}");
        }
    }
}

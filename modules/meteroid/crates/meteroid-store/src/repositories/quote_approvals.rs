use common_domain::ids::{BaseId, QuoteApprovalId, QuoteId, TenantId, UserId};
use diesel_models::quote_approvals::{QuoteApprovalRow, QuoteApprovalRowNew, QuoteApprovalRuleRow};
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::QuoteApprovalStatusEnum;
use diesel_models::quotes::{QuoteRow, QuoteRowUpdate};
use error_stack::Report;

use crate::domain::quote_approvals::{QuoteApproval, QuoteApprovalRequirement, QuoteApprovalRule};
use crate::domain::quotes::Quote;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

/// The reads publishing depends on. Rules are only created by the cpq module: without it the
/// list is empty and nothing needs approval.
#[async_trait::async_trait]
pub trait QuoteApprovalsInterface {
    async fn list_quote_approval_rules(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<QuoteApprovalRule>>;

    async fn latest_quote_approval(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteApproval>>;

    /// Evaluates the rules against the quote and stores the resulting approval status.
    async fn quote_approval_requirement(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<(Quote, QuoteApprovalRequirement)>;

    /// Asks for the approval of the quote as it stands. Refused when no rule matches it.
    async fn request_quote_approval(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        requested_by: UserId,
        note: Option<String>,
    ) -> StoreResult<QuoteApproval>;

    async fn list_pending_quote_approvals(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<QuoteApproval>>;

    async fn get_quote_approval(
        &self,
        tenant_id: TenantId,
        id: QuoteApprovalId,
    ) -> StoreResult<Option<QuoteApproval>>;

    /// Approves or rejects a pending request; refused when it was decided meanwhile.
    async fn decide_quote_approval(
        &self,
        tenant_id: TenantId,
        id: QuoteApprovalId,
        decided_by: UserId,
        approve: bool,
        note: Option<String>,
    ) -> StoreResult<QuoteApproval>;
}

#[async_trait::async_trait]
impl QuoteApprovalsInterface for Store {
    async fn list_quote_approval_rules(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<QuoteApprovalRule>> {
        let mut conn = self.get_conn().await?;
        QuoteApprovalRuleRow::list_active(&mut conn, tenant_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn latest_quote_approval(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteApproval>> {
        let mut conn = self.get_conn().await?;
        QuoteApprovalRow::latest_for_quote(&mut conn, tenant_id, quote_id)
            .await
            .map(|r| r.map(Into::into))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn quote_approval_requirement(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<(Quote, QuoteApprovalRequirement)> {
        let mut conn = self.get_conn().await?;
        quote_approval_requirement_tx(&mut conn, tenant_id, quote_id).await
    }

    async fn request_quote_approval(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        requested_by: UserId,
        note: Option<String>,
    ) -> StoreResult<QuoteApproval> {
        use crate::repositories::quote_signing::QuoteSigningInterface;

        self.refresh_quote_document(tenant_id, quote_id).await?;
        let (quote, requirement) = self.quote_approval_requirement(tenant_id, quote_id).await?;
        if requirement.matched_rules.is_empty() {
            return Err(StoreError::InvalidArgument(
                "No approval rule applies to this quote".to_string(),
            )
            .into());
        }
        let document_hash = quote.document_hash.clone().unwrap_or_default();
        if requirement.is_satisfied(Some(&document_hash))
            || requirement.latest.as_ref().is_some_and(|a| {
                a.status == QuoteApprovalStatusEnum::Pending && a.document_hash == document_hash
            })
        {
            return Err(StoreError::InvalidArgument(
                "An approval of this version is already requested or granted".to_string(),
            )
            .into());
        }

        let approval = self
            .transaction(|conn| {
                async move {
                    let row = QuoteApprovalRowNew {
                        id: QuoteApprovalId::new(),
                        tenant_id,
                        quote_id,
                        revision: quote.revision,
                        document_hash,
                        rule_ids: requirement
                            .matched_rules
                            .iter()
                            .map(|r| Some(r.id.as_uuid()))
                            .collect(),
                        status: QuoteApprovalStatusEnum::Pending.into(),
                        requested_by: Some(requested_by.as_uuid()),
                        request_note: note,
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    self.internal
                        .record_audit_tx(
                            conn,
                            tenant_id,
                            &Actor::User { id: requested_by },
                            AuditInput::Activity(Activity::new(
                                ActivityType::QuoteApprovalRequested,
                                EntityType::Quote,
                                quote_id.as_uuid(),
                            )),
                        )
                        .await?;
                    Ok(QuoteApproval::from(row))
                }
                .scope_boxed()
            })
            .await?;
        self.quote_approval_requirement(tenant_id, quote_id).await?;
        Ok(approval)
    }

    async fn list_pending_quote_approvals(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<QuoteApproval>> {
        let mut conn = self.get_conn().await?;
        QuoteApprovalRow::list_pending(&mut conn, tenant_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn get_quote_approval(
        &self,
        tenant_id: TenantId,
        id: QuoteApprovalId,
    ) -> StoreResult<Option<QuoteApproval>> {
        let mut conn = self.get_conn().await?;
        QuoteApprovalRow::find(&mut conn, tenant_id, id)
            .await
            .map(|r| r.map(Into::into))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn decide_quote_approval(
        &self,
        tenant_id: TenantId,
        id: QuoteApprovalId,
        decided_by: UserId,
        approve: bool,
        note: Option<String>,
    ) -> StoreResult<QuoteApproval> {
        let approval = self
            .transaction(|conn| {
                async move {
                    let status = if approve {
                        QuoteApprovalStatusEnum::Approved
                    } else {
                        QuoteApprovalStatusEnum::Rejected
                    };
                    let row = QuoteApprovalRow::decide(
                        conn,
                        tenant_id,
                        id,
                        status.into(),
                        decided_by.as_uuid(),
                        note.clone(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(
                            "This approval request was already decided".to_string(),
                        )
                    })?;
                    let activity = if approve {
                        ActivityType::QuoteApproved
                    } else {
                        ActivityType::QuoteApprovalRejected
                    };
                    let mut entry =
                        Activity::new(activity, EntityType::Quote, row.quote_id.as_uuid());
                    if let Some(note) = note {
                        entry = entry.with_metadata(serde_json::json!({ "note": note }));
                    }
                    self.internal
                        .record_audit_tx(
                            conn,
                            tenant_id,
                            &Actor::User { id: decided_by },
                            AuditInput::Activity(entry),
                        )
                        .await?;
                    Ok(QuoteApproval::from(row))
                }
                .scope_boxed()
            })
            .await?;
        self.quote_approval_requirement(tenant_id, approval.quote_id)
            .await?;
        Ok(approval)
    }
}

/// Evaluates the rules against the quote and stores the resulting approval status.
pub(crate) async fn quote_approval_requirement_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<(Quote, QuoteApprovalRequirement)> {
    let row = QuoteRow::find_by_id(conn, tenant_id, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let quote: Quote = row.try_into()?;
    let rules: Vec<QuoteApprovalRule> = QuoteApprovalRuleRow::list_active(conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(Into::into)
        .collect();
    let matched_rules: Vec<QuoteApprovalRule> =
        rules.into_iter().filter(|r| r.matches(&quote)).collect();
    let latest = QuoteApprovalRow::latest_for_quote(conn, tenant_id, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .map(Into::into);
    let requirement = QuoteApprovalRequirement {
        matched_rules,
        latest,
    };

    let status = requirement.status(quote.document_hash.as_deref());
    if status != quote.approval_status {
        QuoteRow::update_by_id(
            conn,
            tenant_id,
            quote_id,
            QuoteRowUpdate {
                approval_status: Some(status.into()),
                ..Default::default()
            },
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok((
        Quote {
            approval_status: status,
            ..quote
        },
        requirement,
    ))
}

/// A published quote edited into one needing an approval it lacks goes back to draft, so
/// nobody signs it before the approval.
pub(crate) async fn hold_quote_for_approval_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<bool> {
    let (quote, requirement) = quote_approval_requirement_tx(conn, tenant_id, quote_id).await?;
    if quote.status != crate::domain::enums::QuoteStatusEnum::Pending
        || requirement.is_satisfied(quote.document_hash.as_deref())
    {
        return Ok(false);
    }
    QuoteRow::update_by_id(
        conn,
        tenant_id,
        quote_id,
        QuoteRowUpdate {
            status: Some(diesel_models::enums::QuoteStatusEnum::Draft),
            ..Default::default()
        },
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(true)
}

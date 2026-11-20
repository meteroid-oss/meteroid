use crate::domain::quote_drafts::{QuoteDraft, QuoteDraftSave, QuoteDraftSaveOutcome};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{BaseId, QuoteDraftId, QuoteId, TenantId, UserId};
use diesel_models::quote_drafts::{QuoteDraftRow, QuoteDraftRowNew};
use diesel_models::quotes::QuoteRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Larger payloads are refused: an editor state is a few kilobytes.
pub const MAX_QUOTE_DRAFT_BYTES: usize = 512 * 1024;
const LIST_LIMIT: i64 = 50;

#[async_trait::async_trait]
pub trait QuoteDraftsInterface {
    async fn save_quote_draft(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        save: QuoteDraftSave,
    ) -> StoreResult<QuoteDraftSaveOutcome>;

    async fn get_quote_draft(
        &self,
        tenant_id: TenantId,
        id: QuoteDraftId,
    ) -> StoreResult<Option<QuoteDraft>>;

    async fn get_quote_draft_by_quote(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteDraft>>;

    async fn list_unsaved_quote_drafts(&self, tenant_id: TenantId) -> StoreResult<Vec<QuoteDraft>>;

    async fn delete_quote_draft(&self, tenant_id: TenantId, id: QuoteDraftId) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl QuoteDraftsInterface for Store {
    async fn save_quote_draft(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        save: QuoteDraftSave,
    ) -> StoreResult<QuoteDraftSaveOutcome> {
        if serde_json::to_vec(&save.payload).map_or(0, |b| b.len()) > MAX_QUOTE_DRAFT_BYTES {
            return Err(StoreError::InvalidArgument("Quote draft is too large".to_string()).into());
        }

        self.transaction(|conn| {
            async move {
                if let Some(quote_id) = save.quote_id {
                    // Tenant check: the draft of another tenant's quote must not be creatable.
                    QuoteRow::find_by_id(conn, tenant_id, quote_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                let existing = match (save.id, save.quote_id) {
                    (Some(id), _) => QuoteDraftRow::find_by_id(conn, tenant_id, id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?,
                    (None, Some(quote_id)) => {
                        QuoteDraftRow::find_by_quote_id(conn, tenant_id, quote_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                    }
                    (None, None) => None,
                };

                let Some(existing) = existing else {
                    if save.id.is_some() {
                        return Err(StoreError::ValueNotFound("quote draft".to_string()).into());
                    }
                    let row = QuoteDraftRowNew {
                        id: QuoteDraftId::new(),
                        tenant_id,
                        quote_id: save.quote_id,
                        created_by: user_id.as_uuid(),
                        updated_by: Some(user_id.as_uuid()),
                        label: save.label,
                        payload: save.payload,
                        payload_version: save.payload_version,
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    return Ok(QuoteDraftSaveOutcome::Saved(row.into()));
                };

                if existing.quote_id != save.quote_id {
                    return Err(StoreError::InvalidArgument(
                        "The draft belongs to another quote".to_string(),
                    )
                    .into());
                }

                let Some(expected) = save.expected_version else {
                    return Ok(QuoteDraftSaveOutcome::Conflict(existing.into()));
                };

                let updated = QuoteDraftRow::update_if_version(
                    conn,
                    tenant_id,
                    existing.id,
                    expected,
                    save.payload,
                    save.payload_version,
                    save.label,
                    user_id.as_uuid(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(match updated {
                    Some(row) => QuoteDraftSaveOutcome::Saved(row.into()),
                    None => QuoteDraftSaveOutcome::Conflict(
                        QuoteDraftRow::find_by_id(conn, tenant_id, existing.id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .unwrap_or(existing)
                            .into(),
                    ),
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_quote_draft(
        &self,
        tenant_id: TenantId,
        id: QuoteDraftId,
    ) -> StoreResult<Option<QuoteDraft>> {
        let mut conn = self.get_conn().await?;
        QuoteDraftRow::find_by_id(&mut conn, tenant_id, id)
            .await
            .map(|r| r.map(Into::into))
            .map_err(Into::into)
    }

    async fn get_quote_draft_by_quote(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteDraft>> {
        let mut conn = self.get_conn().await?;
        QuoteDraftRow::find_by_quote_id(&mut conn, tenant_id, quote_id)
            .await
            .map(|r| r.map(Into::into))
            .map_err(Into::into)
    }

    async fn list_unsaved_quote_drafts(&self, tenant_id: TenantId) -> StoreResult<Vec<QuoteDraft>> {
        let mut conn = self.get_conn().await?;
        QuoteDraftRow::list_unsaved(&mut conn, tenant_id, LIST_LIMIT)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    async fn delete_quote_draft(&self, tenant_id: TenantId, id: QuoteDraftId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        QuoteDraftRow::delete(&mut conn, tenant_id, id)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

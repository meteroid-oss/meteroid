use common_domain::ids::{BaseId, QuoteCommentId, QuoteId, TenantId, UserId};
use diesel_models::customers::CustomerRow;
use diesel_models::enums::QuoteStatusEnum;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::quote_comments::{QuoteCommentRow, QuoteCommentRowNew};
use diesel_models::quotes::QuoteRow;
use diesel_models::users::UserRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::pgmq::{PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::domain::quote_comments::{
    MAX_ANCHOR_LENGTH, MAX_COMMENT_LENGTH, QuoteComment, QuoteCommentAuthor, QuoteCommentNew,
};
use crate::domain::quote_signing::{QuoteEvidenceKind, QuoteEvidenceNew, RequestOrigin};
use crate::domain::quotes::RecipientDetails;
use crate::errors::StoreError;
use crate::repositories::pgmq::PgmqInterface;
use crate::store::PgConn;
use crate::{Store, StoreResult};

/// The discussion on a quote. The cpq module serves it to the team; the portal to recipients.
#[async_trait::async_trait]
pub trait QuoteCommentsInterface {
    async fn list_quote_comments(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        shared_only: bool,
    ) -> StoreResult<Vec<QuoteComment>>;

    /// Posts a comment and emails the other side of shared ones. A change request marks the
    /// quote and joins its evidence.
    async fn add_quote_comment(
        &self,
        tenant_id: TenantId,
        comment: QuoteCommentNew,
        origin: RequestOrigin,
    ) -> StoreResult<QuoteComment>;

    async fn resolve_quote_change_request(
        &self,
        tenant_id: TenantId,
        id: QuoteCommentId,
        resolved_by: UserId,
    ) -> StoreResult<QuoteComment>;
}

fn db_err(e: impl Into<Report<StoreError>>) -> Report<StoreError> {
    e.into()
}

fn invalid(msg: &str) -> Report<StoreError> {
    StoreError::InvalidArgument(msg.to_string()).into()
}

/// The member's name; without one, the seller's: the customer never sees a member's email.
async fn author_name_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote: &QuoteRow,
    user: &UserRow,
) -> StoreResult<String> {
    let name = [user.first_name.as_deref(), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !name.is_empty() {
        return Ok(name);
    }
    let customer = CustomerRow::find_by_id(conn, &quote.customer_id, &tenant_id)
        .await
        .map_err(db_err)?;
    Ok(InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
        conn,
        customer.invoicing_entity_id,
        tenant_id,
    )
    .await
    .map_err(db_err)?
    .legal_name)
}

/// Clears the quote's mark once none of its change requests is open.
pub(crate) async fn refresh_changes_requested_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<()> {
    let open = QuoteCommentRow::count_open_change_requests(conn, tenant_id, quote_id)
        .await
        .map_err(db_err)?;
    if open == 0 {
        QuoteCommentRow::mark_quote_changes_requested(conn, tenant_id, quote_id, None)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

impl Store {
    /// Who hears of a customer message: the quote's owner and the members in the discussion.
    async fn quote_team_emails_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote: &QuoteRow,
    ) -> StoreResult<Vec<String>> {
        let mut ids: Vec<UserId> = quote.owner_id.into_iter().collect();
        for comment in QuoteCommentRow::list_by_quote_id(conn, tenant_id, quote.id, false)
            .await
            .map_err(db_err)?
        {
            if let Some(id) = comment.author_user_id
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        Ok(UserRow::find_members_by_ids(conn, &ids, tenant_id)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|u| u.email)
            .collect())
    }

    async fn quote_message_emails_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote: &QuoteRow,
        comment: &QuoteCommentRow,
    ) -> StoreResult<Vec<PgmqMessageNew>> {
        let customer = CustomerRow::find_by_id(conn, &quote.customer_id, &tenant_id)
            .await
            .map_err(db_err)?;
        let entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(db_err)?;
        let message = |recipient_email: String, portal_url: Option<String>| {
            SendEmailRequest::QuoteMessage {
                tenant_id,
                quote_id: quote.id,
                invoicing_entity_id: customer.invoicing_entity_id,
                quote_number: quote.quote_number.clone(),
                company_name: entity.legal_name.clone(),
                customer_name: customer.name.clone(),
                author_name: comment.author_name.clone(),
                body: comment.body.clone(),
                change_request: comment.is_change_request,
                recipient_email,
                portal_url,
            }
            .try_into()
        };

        let mut messages = vec![];
        if comment.author_user_id.is_none() {
            for email in Self::quote_team_emails_tx(conn, tenant_id, quote).await? {
                messages.push(message(email, None)?);
            }
        } else {
            let recipients: Vec<RecipientDetails> =
                serde_json::from_value(quote.recipients.clone()).map_err(|e| {
                    StoreError::SerdeError("Failed to deserialize recipients".to_string(), e)
                })?;
            for recipient in recipients {
                let token = self
                    .quote_portal_token_tx(conn, tenant_id, quote, recipient.email.clone(), false)
                    .await?;
                let url = format!("{}/portal/quote?token={}", &self.settings.public_url, token);
                messages.push(message(recipient.email, Some(url))?);
            }
        }
        Ok(messages)
    }
}

#[async_trait::async_trait]
impl QuoteCommentsInterface for Store {
    async fn list_quote_comments(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        shared_only: bool,
    ) -> StoreResult<Vec<QuoteComment>> {
        let mut conn = self.get_conn().await?;
        QuoteCommentRow::list_by_quote_id(&mut conn, tenant_id, quote_id, shared_only)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_err)
    }

    async fn add_quote_comment(
        &self,
        tenant_id: TenantId,
        comment: QuoteCommentNew,
        origin: RequestOrigin,
    ) -> StoreResult<QuoteComment> {
        let body = comment.body.trim().to_string();
        if body.is_empty() {
            return Err(invalid("A message cannot be empty"));
        }
        if body.chars().count() > MAX_COMMENT_LENGTH {
            return Err(invalid("A message is at most 10,000 characters"));
        }
        let anchor = comment
            .anchor
            .as_deref()
            .map(|a| {
                let a = a.trim();
                if a.is_empty() {
                    Err(invalid("The quote line a message is about cannot be empty"))
                } else if a.chars().count() > MAX_ANCHOR_LENGTH {
                    Err(invalid(
                        "The quote line a message is about is at most 200 characters",
                    ))
                } else {
                    Ok(a.to_string())
                }
            })
            .transpose()?;

        self.transaction(|conn| {
            async move {
                let quote = QuoteRow::find_by_id_for_update(conn, tenant_id, comment.quote_id)
                    .await
                    .map_err(db_err)?;

                let (author_user_id, author_email, author_name, shared) = match &comment.author {
                    QuoteCommentAuthor::Member(user_id) => {
                        if comment.is_change_request {
                            return Err(invalid("Only the customer requests changes"));
                        }
                        if comment.shared && quote.status == QuoteStatusEnum::Draft {
                            return Err(invalid(
                                "Publish the quote before writing to the customer",
                            ));
                        }
                        let user = UserRow::find_by_id(conn, *user_id).await.map_err(db_err)?;
                        let name = author_name_tx(conn, tenant_id, &quote, &user).await?;
                        (Some(*user_id), None, name, comment.shared)
                    }
                    QuoteCommentAuthor::Recipient { email } => {
                        if quote.status != QuoteStatusEnum::Pending {
                            return Err(invalid("This quote is no longer open for discussion"));
                        }
                        if comment.is_change_request && !quote.allow_change_requests {
                            return Err(StoreError::FailedPrecondition(
                                "This quote does not take change requests".to_string(),
                            )
                            .into());
                        }
                        if !comment.is_change_request && !quote.allow_questions {
                            return Err(StoreError::FailedPrecondition(
                                "This quote does not take questions".to_string(),
                            )
                            .into());
                        }
                        let recipients: Vec<RecipientDetails> =
                            serde_json::from_value(quote.recipients.clone()).map_err(|e| {
                                StoreError::SerdeError(
                                    "Failed to deserialize recipients".to_string(),
                                    e,
                                )
                            })?;
                        let recipient = recipients
                            .into_iter()
                            .find(|r| r.email.trim().eq_ignore_ascii_case(email.trim()))
                            .ok_or_else(|| invalid("Only the quote's recipients can write here"))?;
                        (None, Some(recipient.email), recipient.name, true)
                    }
                };

                let row = QuoteCommentRowNew {
                    id: QuoteCommentId::new(),
                    tenant_id,
                    quote_id: quote.id,
                    revision: quote.revision,
                    author_user_id,
                    author_email: author_email.clone(),
                    author_name,
                    body,
                    shared,
                    is_change_request: comment.is_change_request,
                    anchor,
                }
                .insert(conn)
                .await
                .map_err(db_err)?;

                if row.is_change_request {
                    QuoteCommentRow::mark_quote_changes_requested(
                        conn,
                        tenant_id,
                        quote.id,
                        Some(row.created_at),
                    )
                    .await
                    .map_err(db_err)?;
                    Self::append_quote_evidence_tx(
                        conn,
                        tenant_id,
                        &quote,
                        QuoteEvidenceNew {
                            kind: QuoteEvidenceKind::ChangesRequested,
                            actor_email: author_email,
                            origin,
                            details: serde_json::json!({
                                "comment_id": row.id.to_string(),
                                "message": row.body,
                            }),
                        },
                    )
                    .await?;
                }

                if row.shared {
                    let messages = self
                        .quote_message_emails_tx(conn, tenant_id, &quote, &row)
                        .await?;
                    if !messages.is_empty() {
                        self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, messages)
                            .await?;
                    }
                }
                Ok(row.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn resolve_quote_change_request(
        &self,
        tenant_id: TenantId,
        id: QuoteCommentId,
        resolved_by: UserId,
    ) -> StoreResult<QuoteComment> {
        self.transaction(|conn| {
            async move {
                let not_found =
                    || Report::new(StoreError::ValueNotFound("Comment not found".to_string()));
                let quote_id = QuoteCommentRow::find(conn, tenant_id, id)
                    .await
                    .map_err(db_err)?
                    .ok_or_else(not_found)?
                    .quote_id;
                QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?;
                let row = QuoteCommentRow::find(conn, tenant_id, id)
                    .await
                    .map_err(db_err)?
                    .ok_or_else(not_found)?;
                if !row.is_change_request {
                    return Err(invalid("Only a change request can be resolved"));
                }
                if row.resolved_at.is_none() {
                    QuoteCommentRow::resolve_change_requests(
                        conn,
                        tenant_id,
                        row.quote_id,
                        Some(id),
                        Some(resolved_by),
                        common_clock::now().naive_utc(),
                    )
                    .await
                    .map_err(db_err)?;
                    refresh_changes_requested_tx(conn, tenant_id, row.quote_id).await?;
                }
                QuoteCommentRow::find(conn, tenant_id, id)
                    .await
                    .map_err(db_err)?
                    .map(Into::into)
                    .ok_or_else(not_found)
            }
            .scope_boxed()
        })
        .await
    }
}

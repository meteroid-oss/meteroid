use chrono::{Duration, NaiveDateTime, Timelike};
use common_domain::ids::{
    BaseId, QuoteEvidenceEventId, QuoteId, QuoteRevisionId, QuoteSignatureOtpId, StoredDocumentId,
    TenantId,
};
use diesel_models::enums::{QuoteSignatureLevelEnum, QuoteStatusEnum};
use diesel_models::quote_comments::QuoteCommentRow;
use diesel_models::quote_recipient_engagement::QuoteRecipientEngagementRow;
use diesel_models::quote_revisions::{QuoteRevisionRow, QuoteRevisionRowNew};
use diesel_models::quote_signing::{
    QuoteEvidenceEventRow, QuoteEvidenceEventRowNew, QuoteSignatureCertificateRow,
    QuoteSignatureOtpRow, QuoteSignatureOtpRowNew,
};
use diesel_models::quote_views::QuoteViewRow;
use diesel_models::quotes::{QuoteRow, QuoteRowUpdate, QuoteSignatureRow, QuoteSignatureRowNew};
use error_stack::Report;
use rand::RngExt;
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::QuoteSignerRoleEnum;
use crate::domain::pgmq::{PgmqQueue, SendEmailRequest};
use crate::domain::quote_document::{QuoteDocument, hex_sha256, max_discount_percent};
use crate::domain::quote_signing::{
    QuoteEvidenceEvent, QuoteEvidenceKind, QuoteEvidenceNew, QuoteRecipientEngagement,
    QuoteRevision, QuoteSignatureCertificate, QuoteViewSession, RequestOrigin,
    SignatureCodeChannel, chain_hash,
};
use crate::domain::quotes::{Quote, QuoteSignature};
use crate::errors::StoreError;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::repositories::quote_comments::refresh_changes_requested_tx;
use crate::store::PgConn;
use crate::{Store, StoreResult};

const CODE_TTL_MINUTES: i64 = 10;
const CODE_MAX_ATTEMPTS: i32 = 5;
const CODES_PER_WINDOW: i64 = 5;
const CODE_WINDOW_MINUTES: i64 = 30;
/// How long a verified code allows signing.
const VERIFIED_CODE_VALIDITY_MINUTES: i64 = 15;
/// Portal reloads (refetch on focus, after a comment) within this gap of the last open are one view.
const VIEW_GAP_MINUTES: i64 = 30;
const VIEW_SESSIONS_PER_PERSON: i64 = 50;
pub const AUTOMATIC_COUNTERSIGNATURE: &str = "automatic_countersignature";

#[derive(Debug, Clone)]
pub struct SignQuoteInput {
    pub quote_id: QuoteId,
    pub role: QuoteSignerRoleEnum,
    pub signed_by_name: String,
    pub signed_by_email: String,
    pub signed_by_title: Option<String>,
    pub signature_data: Option<String>,
    pub signature_method: String,
    /// The `document_hash` of the version the signer was shown.
    pub document_hash: Option<String>,
    /// The consent statement, exactly as displayed next to the signature.
    pub consent_text: Option<String>,
    /// A signed-in member of the seller's organization, already authenticated: no code needed.
    pub authenticated: bool,
    pub origin: RequestOrigin,
    pub actor: Actor,
}

#[derive(Debug, Clone)]
pub struct SignQuoteOutcome {
    pub signature: QuoteSignature,
    pub accepted: bool,
    /// The customer side is complete and countersigners still have to sign.
    pub awaiting_countersignature: bool,
}

#[async_trait::async_trait]
pub trait QuoteSigningInterface {
    /// Recomputes the quote's document hash and largest discount from its current content.
    async fn refresh_quote_document(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<QuoteDocument>;

    /// Freezes the current revision of a published quote, superseding the earlier ones.
    async fn record_quote_revision(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        created_by: Option<String>,
    ) -> StoreResult<()>;

    async fn list_quote_revisions(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteRevision>>;

    async fn find_quote_revision(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
    ) -> StoreResult<Option<QuoteRevision>>;

    /// Records the revision's PDF unless one already is; returns the one kept.
    async fn set_quote_revision_pdf(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
        pdf_document_id: StoredDocumentId,
    ) -> StoreResult<StoredDocumentId>;

    async fn record_quote_evidence(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        event: QuoteEvidenceNew,
    ) -> StoreResult<()>;

    /// A recipient opened the quote: in the evidence once per recipient and revision, in the
    /// engagement and the view sessions on every open.
    async fn record_quote_view(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        origin: RequestOrigin,
    ) -> StoreResult<()>;

    async fn list_quote_recipient_engagement(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteRecipientEngagement>>;

    /// Each person's latest view sessions, newest first.
    async fn list_quote_view_sessions(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteViewSession>>;

    async fn list_quote_evidence(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteEvidenceEvent>>;

    /// How a signature code reaches each of `emails`, in order.
    async fn signature_code_channels(
        &self,
        tenant_id: TenantId,
        emails: &[String],
    ) -> StoreResult<Vec<SignatureCodeChannel>>;

    /// Emails a one-time code to a signer of a quote that requires one. A test-mode workspace
    /// emails nothing and returns the code for the portal to display.
    async fn send_quote_signature_code(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        origin: RequestOrigin,
    ) -> StoreResult<Option<String>>;

    /// Checks a code; `false` when wrong or expired. Attempts are kept even when refused.
    async fn verify_quote_signature_code(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        code: String,
        origin: RequestOrigin,
    ) -> StoreResult<bool>;

    /// Whether the signer has a confirmed code ready for signing.
    async fn quote_signature_code_verified(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
    ) -> StoreResult<bool>;

    async fn sign_quote(
        &self,
        tenant_id: TenantId,
        input: SignQuoteInput,
    ) -> StoreResult<SignQuoteOutcome>;

    /// Emails the countersigners who still have to sign their link.
    async fn notify_quote_countersigners(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<()>;

    async fn get_quote_signature_certificate(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteSignatureCertificate>>;

    /// Keeps the first certificate issued: returns it when the quote already has one.
    async fn save_quote_signature_certificate(
        &self,
        tenant_id: TenantId,
        certificate: QuoteSignatureCertificate,
    ) -> StoreResult<QuoteSignatureCertificate>;

    /// The HubSpot deals of the quotes these subscriptions were converted from.
    async fn list_quote_deal_links(
        &self,
        subscription_ids: Vec<common_domain::ids::SubscriptionId>,
    ) -> StoreResult<Vec<(common_domain::ids::SubscriptionId, String)>>;

    /// Queues the push of the quote's state to its HubSpot deal, when it is linked to one.
    async fn sync_quote_to_hubspot(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<()>;
}

fn db_err(e: impl Into<Report<StoreError>>) -> Report<StoreError> {
    e.into()
}

/// Postgres keeps microseconds: the chained timestamp must read back identical.
fn now_micros() -> NaiveDateTime {
    let now = common_clock::now().naive_utc();
    now.with_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .unwrap_or(now)
}

/// Exposed for tests, which store a code of their own instead of reading the email.
pub fn code_hash(id: QuoteSignatureOtpId, email: &str, code: &str) -> String {
    hex_sha256(format!("{id}|{}|{code}", email.to_lowercase()).as_bytes())
}

fn same_email(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Whether `email` may sign `quote` now, and in which role.
fn ensure_signer(quote: &Quote, email: &str, role: QuoteSignerRoleEnum) -> StoreResult<()> {
    let listed = match role {
        QuoteSignerRoleEnum::Customer => {
            quote.recipients.iter().any(|r| same_email(&r.email, email))
        }
        QuoteSignerRoleEnum::Countersigner => quote
            .countersigners
            .iter()
            .any(|c| !c.automatic && same_email(&c.email, email)),
    };
    if listed {
        Ok(())
    } else {
        Err(
            StoreError::InvalidArgument("This email is not a signer of the quote".to_string())
                .into(),
        )
    }
}

impl Store {
    pub(crate) async fn append_quote_evidence_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote: &QuoteRow,
        event: QuoteEvidenceNew,
    ) -> StoreResult<QuoteEvidenceEvent> {
        let last = QuoteEvidenceEventRow::last_by_quote_id(conn, tenant_id, quote.id)
            .await
            .map_err(db_err)?;
        let seq = last.as_ref().map_or(1, |l| l.seq + 1);
        let prev_hash = last.map(|l| l.hash);
        let occurred_at = now_micros();
        let kind = event.kind.as_str();
        let hash = chain_hash(
            prev_hash.as_deref(),
            seq,
            kind,
            event.actor_email.as_deref(),
            event.origin.ip_address.as_deref(),
            event.origin.user_agent.as_deref(),
            quote.revision,
            quote.document_hash.as_deref(),
            &event.details,
            occurred_at,
        );
        QuoteEvidenceEventRowNew {
            id: QuoteEvidenceEventId::new(),
            tenant_id,
            quote_id: quote.id,
            seq,
            kind: kind.to_string(),
            actor_email: event.actor_email,
            ip_address: event.origin.ip_address,
            user_agent: event.origin.user_agent,
            revision: quote.revision,
            document_hash: quote.document_hash.clone(),
            details: event.details,
            occurred_at,
            prev_hash,
            hash,
        }
        .insert(conn)
        .await
        .map(Into::into)
        .map_err(db_err)
    }

    async fn verified_code_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
    ) -> StoreResult<QuoteSignatureOtpRow> {
        let refused = || {
            Report::new(StoreError::InvalidArgument(
                "Confirm the code sent to your email before signing".to_string(),
            ))
        };
        let otp =
            QuoteSignatureOtpRow::latest_for(conn, tenant_id, quote_id, &email.to_lowercase())
                .await
                .map_err(db_err)?
                .ok_or_else(refused)?;
        let fresh = otp.verified_at.is_some_and(|at| {
            at > common_clock::now().naive_utc() - Duration::minutes(VERIFIED_CODE_VALIDITY_MINUTES)
        });
        if !fresh || otp.consumed_at.is_some() {
            return Err(refused());
        }
        Ok(otp)
    }
}

#[async_trait::async_trait]
impl QuoteSigningInterface for Store {
    async fn refresh_quote_document(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<QuoteDocument> {
        let mut conn = self.get_conn().await?;
        refresh_quote_document_tx(&mut conn, tenant_id, quote_id).await
    }

    async fn record_quote_revision(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        created_by: Option<String>,
    ) -> StoreResult<()> {
        let document = self.refresh_quote_document(tenant_id, quote_id).await?;
        let hash = document.hash();
        let mut conn = self.get_conn().await?;
        QuoteRevisionRowNew {
            id: QuoteRevisionId::new(),
            tenant_id,
            quote_id,
            revision: document.revision,
            snapshot: serde_json::to_value(&document).map_err(|e| {
                StoreError::SerdeError("Failed to serialize quote document".to_string(), e)
            })?,
            document_hash: hash,
            created_by,
        }
        .insert_if_absent(&mut conn)
        .await
        .map_err(db_err)?;
        QuoteRevisionRow::mark_superseded(&mut conn, tenant_id, quote_id, document.revision)
            .await
            .map_err(db_err)?;
        if document.revision > 1 {
            QuoteCommentRow::resolve_change_requests(
                &mut conn,
                tenant_id,
                quote_id,
                None,
                None,
                common_clock::now().naive_utc(),
            )
            .await
            .map_err(db_err)?;
            refresh_changes_requested_tx(&mut conn, tenant_id, quote_id).await?;
        }
        Ok(())
    }

    async fn list_quote_revisions(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteRevision>> {
        let mut conn = self.get_conn().await?;
        QuoteRevisionRow::list_by_quote_id(&mut conn, tenant_id, quote_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_err)
    }

    async fn find_quote_revision(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
    ) -> StoreResult<Option<QuoteRevision>> {
        let mut conn = self.get_conn().await?;
        QuoteRevisionRow::find(&mut conn, tenant_id, quote_id, revision)
            .await
            .map(|row| row.map(Into::into))
            .map_err(db_err)
    }

    async fn set_quote_revision_pdf(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
        pdf_document_id: StoredDocumentId,
    ) -> StoreResult<StoredDocumentId> {
        let mut conn = self.get_conn().await?;
        if QuoteRevisionRow::set_pdf_document_if_absent(
            &mut conn,
            tenant_id,
            quote_id,
            revision,
            pdf_document_id,
        )
        .await
        .map_err(db_err)?
        {
            return Ok(pdf_document_id);
        }
        QuoteRevisionRow::find(&mut conn, tenant_id, quote_id, revision)
            .await
            .map_err(db_err)?
            .and_then(|row| row.pdf_document_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "Revision {revision} of quote {quote_id}"
                )))
            })
    }

    async fn record_quote_evidence(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        event: QuoteEvidenceNew,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let quote = QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?;
                Self::append_quote_evidence_tx(conn, tenant_id, &quote, event).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn record_quote_view(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        origin: RequestOrigin,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let quote = QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?;
                let now = common_clock::now().naive_utc();
                if quote.first_viewed_at.is_none() {
                    QuoteRow::update_by_id(
                        conn,
                        tenant_id,
                        quote_id,
                        QuoteRowUpdate {
                            first_viewed_at: Some(Some(now)),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(db_err)?;
                }
                let new_view_before = now - Duration::minutes(VIEW_GAP_MINUTES);
                QuoteRecipientEngagementRow::record_view(
                    conn,
                    tenant_id,
                    quote_id,
                    email.trim(),
                    quote.revision,
                    now,
                    new_view_before,
                )
                .await
                .map_err(db_err)?;
                QuoteViewRow::record(
                    conn,
                    tenant_id,
                    quote_id,
                    email.trim(),
                    quote.revision,
                    now,
                    new_view_before,
                    origin.ip_address.as_deref(),
                    origin.user_agent.as_deref(),
                )
                .await
                .map_err(db_err)?;
                let seen = QuoteEvidenceEventRow::list_by_quote_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?
                    .iter()
                    .any(|e| {
                        e.kind == QuoteEvidenceKind::Viewed.as_str()
                            && e.revision == quote.revision
                            && e.actor_email
                                .as_deref()
                                .is_some_and(|a| same_email(a, &email))
                    });
                if !seen {
                    Self::append_quote_evidence_tx(
                        conn,
                        tenant_id,
                        &quote,
                        QuoteEvidenceNew {
                            kind: QuoteEvidenceKind::Viewed,
                            actor_email: Some(email),
                            origin,
                            details: serde_json::json!({}),
                        },
                    )
                    .await?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_quote_recipient_engagement(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteRecipientEngagement>> {
        let mut conn = self.get_conn().await?;
        QuoteRecipientEngagementRow::list_by_quote_id(&mut conn, tenant_id, quote_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_err)
    }

    async fn list_quote_view_sessions(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteViewSession>> {
        let mut conn = self.get_conn().await?;
        QuoteViewRow::list_latest_by_quote_id(
            &mut conn,
            tenant_id,
            quote_id,
            VIEW_SESSIONS_PER_PERSON,
        )
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(db_err)
    }

    async fn list_quote_evidence(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<QuoteEvidenceEvent>> {
        let mut conn = self.get_conn().await?;
        QuoteEvidenceEventRow::list_by_quote_id(&mut conn, tenant_id, quote_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_err)
    }

    async fn signature_code_channels(
        &self,
        tenant_id: TenantId,
        emails: &[String],
    ) -> StoreResult<Vec<SignatureCodeChannel>> {
        if crate::services::customer_emails::tenant_test_mode(self, tenant_id).await? {
            return Ok(vec![SignatureCodeChannel::TestModeDisplay; emails.len()]);
        }
        let delivery = self.email_delivery(tenant_id).await?;
        Ok(emails
            .iter()
            .map(|email| {
                if delivery.reaches(std::slice::from_ref(email)).is_empty() {
                    SignatureCodeChannel::Unavailable
                } else {
                    SignatureCodeChannel::Email
                }
            })
            .collect())
    }

    async fn send_quote_signature_code(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        origin: RequestOrigin,
    ) -> StoreResult<Option<String>> {
        let channel = self
            .signature_code_channels(tenant_id, std::slice::from_ref(&email))
            .await?
            .into_iter()
            .next()
            .unwrap_or(SignatureCodeChannel::Unavailable);
        self.transaction(|conn| {
            async move {
                let row = QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?;
                if row.status != QuoteStatusEnum::Pending {
                    return Err(StoreError::InvalidArgument(
                        "Only pending quotes can be signed".to_string(),
                    )
                    .into());
                }
                if row.signature_level != QuoteSignatureLevelEnum::Verified {
                    return Err(StoreError::InvalidArgument(
                        "This quote does not use signature codes".to_string(),
                    )
                    .into());
                }
                let quote: Quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?
                    .try_into()?;
                let signs_as_customer = quote.recipients.iter().any(|r| same_email(&r.email, &email));
                let role = if signs_as_customer {
                    QuoteSignerRoleEnum::Customer
                } else {
                    QuoteSignerRoleEnum::Countersigner
                };
                ensure_signer(&quote, &email, role)?;
                if channel == SignatureCodeChannel::Unavailable {
                    return Err(StoreError::FailedPrecondition(
                        "The seller can't email signature codes right now".to_string(),
                    )
                    .into());
                }

                let email = email.to_lowercase();
                let recent = QuoteSignatureOtpRow::count_since(
                    conn,
                    tenant_id,
                    quote_id,
                    &email,
                    common_clock::now().naive_utc() - Duration::minutes(CODE_WINDOW_MINUTES),
                )
                .await
                .map_err(db_err)?;
                if recent >= CODES_PER_WINDOW {
                    return Err(StoreError::InvalidArgument(
                        "Too many codes requested, try again later".to_string(),
                    )
                    .into());
                }

                let code = format!("{:06}", rand::rng().random_range(0..1_000_000));
                let id = QuoteSignatureOtpId::new();
                QuoteSignatureOtpRowNew {
                    id,
                    tenant_id,
                    quote_id,
                    email: email.clone(),
                    code_hash: code_hash(id, &email, &code),
                    expires_at: common_clock::now().naive_utc() + Duration::minutes(CODE_TTL_MINUTES),
                }
                .insert(conn)
                .await
                .map_err(db_err)?;

                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &row,
                    QuoteEvidenceNew {
                        kind: QuoteEvidenceKind::CodeSent,
                        actor_email: Some(email.clone()),
                        origin,
                        details: serde_json::json!({ "channel": channel.as_str() }),
                    },
                )
                .await?;
                if channel == SignatureCodeChannel::TestModeDisplay {
                    return Ok(Some(code));
                }

                let customer =
                    diesel_models::customers::CustomerRow::find_by_id(conn, &row.customer_id, &tenant_id)
                        .await
                        .map_err(db_err)?;
                let entity = diesel_models::invoicing_entities::InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                    conn,
                    customer.invoicing_entity_id,
                    tenant_id,
                )
                .await
                .map_err(db_err)?;

                let message = SendEmailRequest::QuoteSignatureCode {
                    tenant_id,
                    quote_id,
                    invoicing_entity_id: entity.id,
                    quote_number: row.quote_number.clone(),
                    company_name: entity.legal_name.clone(),
                    recipient_email: email,
                    code,
                    expires_in_minutes: CODE_TTL_MINUTES as u32,
                }
                .try_into()?;
                self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![message])
                    .await?;
                Ok(None)
            }
            .scope_boxed()
        })
        .await
    }

    async fn verify_quote_signature_code(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: String,
        code: String,
        origin: RequestOrigin,
    ) -> StoreResult<bool> {
        self.transaction(|conn| {
            async move {
                let row = QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?;
                let email = email.to_lowercase();
                let Some(otp) = QuoteSignatureOtpRow::latest_for(conn, tenant_id, quote_id, &email)
                    .await
                    .map_err(db_err)?
                else {
                    Self::append_quote_evidence_tx(
                        conn,
                        tenant_id,
                        &row,
                        QuoteEvidenceNew {
                            kind: QuoteEvidenceKind::CodeRejected,
                            actor_email: Some(email),
                            origin,
                            details: serde_json::json!({ "reason": "no_code_sent" }),
                        },
                    )
                    .await?;
                    return Ok(false);
                };

                let usable = otp.consumed_at.is_none()
                    && otp.expires_at > common_clock::now().naive_utc()
                    && otp.attempts < CODE_MAX_ATTEMPTS;
                let matches = usable && otp.code_hash == code_hash(otp.id, &email, code.trim());

                QuoteSignatureOtpRow::record_attempt(conn, otp.id, matches)
                    .await
                    .map_err(db_err)?;
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &row,
                    QuoteEvidenceNew {
                        kind: if matches {
                            QuoteEvidenceKind::CodeVerified
                        } else {
                            QuoteEvidenceKind::CodeRejected
                        },
                        actor_email: Some(email),
                        origin,
                        details: serde_json::json!({ "attempt": otp.attempts + 1 }),
                    },
                )
                .await?;
                Ok(matches)
            }
            .scope_boxed()
        })
        .await
    }

    async fn quote_signature_code_verified(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        match Self::verified_code_tx(&mut conn, tenant_id, quote_id, email).await {
            Ok(_) => Ok(true),
            Err(e) if matches!(e.current_context(), StoreError::InvalidArgument(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn sign_quote(
        &self,
        tenant_id: TenantId,
        input: SignQuoteInput,
    ) -> StoreResult<SignQuoteOutcome> {
        self.transaction(|conn| {
            async move {
                let row = QuoteRow::find_by_id_for_update(conn, tenant_id, input.quote_id)
                    .await
                    .map_err(db_err)?;
                if row.status != QuoteStatusEnum::Pending {
                    return Err(StoreError::InvalidArgument(
                        "Only pending quotes can be signed".to_string(),
                    )
                    .into());
                }
                if row.expires_at.is_some_and(|at| at < common_clock::now().naive_utc()) {
                    return Err(StoreError::InvalidArgument("The quote has expired".to_string()).into());
                }
                let document_hash = row.document_hash.clone();
                let signature_level = row.signature_level;
                let quote: Quote = QuoteRow::find_by_id(conn, tenant_id, input.quote_id)
                    .await
                    .map_err(db_err)?
                    .try_into()?;

                ensure_signer(&quote, &input.signed_by_email, input.role)?;

                let existing = QuoteSignatureRow::list_by_quote_id(conn, input.quote_id)
                    .await
                    .map_err(db_err)?;
                let signed = |email: &str, role: diesel_models::enums::QuoteSignerRoleEnum| {
                    existing
                        .iter()
                        .any(|s| s.signer_role == role && same_email(&s.signed_by_email, email))
                };
                if signed(&input.signed_by_email, input.role.into()) {
                    return Err(StoreError::InvalidArgument(
                        "This signer has already signed the quote".to_string(),
                    )
                    .into());
                }
                let customers_signed_before = quote
                    .recipients
                    .iter()
                    .all(|r| signed(&r.email, diesel_models::enums::QuoteSignerRoleEnum::Customer));
                if input.role == QuoteSignerRoleEnum::Countersigner && !customers_signed_before {
                    return Err(StoreError::InvalidArgument(
                        "The customer has to sign before the countersignature".to_string(),
                    )
                    .into());
                }

                if let Some(expected) = &document_hash
                    && input.document_hash.as_ref() != Some(expected)
                {
                    return Err(StoreError::InvalidArgument(
                        "The quote changed since it was opened: reload it to sign the current version"
                            .to_string(),
                    )
                    .into());
                }

                let otp_verified_at = if signature_level == QuoteSignatureLevelEnum::Verified
                    && !input.authenticated
                {
                    let otp = Self::verified_code_tx(
                        conn,
                        tenant_id,
                        input.quote_id,
                        &input.signed_by_email,
                    )
                    .await?;
                    QuoteSignatureOtpRow::consume(conn, otp.id).await.map_err(db_err)?;
                    otp.verified_at
                } else {
                    None
                };

                let signature: QuoteSignature = QuoteSignatureRowNew {
                    id: common_domain::ids::QuoteSignatureId::new(),
                    quote_id: input.quote_id,
                    signed_by_name: input.signed_by_name.clone(),
                    signed_by_email: input.signed_by_email.clone(),
                    signed_by_title: input.signed_by_title.clone(),
                    signature_data: input.signature_data.clone(),
                    signature_method: input.signature_method.clone(),
                    ip_address: input.origin.ip_address.clone(),
                    user_agent: input.origin.user_agent.clone(),
                    verification_token: None,
                    signer_role: input.role.into(),
                    revision: Some(row.revision),
                    document_hash: document_hash.clone(),
                    consent_text: input.consent_text.clone(),
                    otp_verified_at,
                }
                .insert(conn)
                .await
                .map_err(db_err)?
                .into();

                let (activity, evidence) = match input.role {
                    QuoteSignerRoleEnum::Customer => {
                        (ActivityType::QuoteSignatureAdded, QuoteEvidenceKind::Signed)
                    }
                    QuoteSignerRoleEnum::Countersigner => {
                        (ActivityType::QuoteCountersigned, QuoteEvidenceKind::Countersigned)
                    }
                };
                self.internal
                    .record_audit_tx(
                        conn,
                        tenant_id,
                        &input.actor,
                        AuditInput::Activity(
                            Activity::new(activity, EntityType::Quote, input.quote_id.as_uuid())
                                .with_metadata(serde_json::json!({
                                    "signed_by_name": input.signed_by_name,
                                    "signed_by_email": input.signed_by_email,
                                    "signed_from_ip": input.origin.ip_address,
                                    "signed_with_ua": input.origin.user_agent,
                                })),
                        ),
                    )
                    .await?;
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &row,
                    QuoteEvidenceNew {
                        kind: evidence,
                        actor_email: Some(input.signed_by_email.clone()),
                        origin: input.origin.clone(),
                        details: serde_json::json!({
                            "name": input.signed_by_name,
                            "title": input.signed_by_title,
                            "method": input.signature_method,
                            "consent_text": input.consent_text,
                            "code_verified": otp_verified_at.is_some(),
                            "authenticated": input.authenticated,
                        }),
                    },
                )
                .await?;

                let customers_done = customers_signed_before
                    || quote.recipients.iter().all(|r| {
                        same_email(&r.email, &input.signed_by_email)
                            || signed(&r.email, diesel_models::enums::QuoteSignerRoleEnum::Customer)
                    });

                if customers_done && input.role == QuoteSignerRoleEnum::Customer {
                    for countersigner in quote.countersigners.iter().filter(|c| c.automatic) {
                        QuoteSignatureRowNew {
                            id: common_domain::ids::QuoteSignatureId::new(),
                            quote_id: input.quote_id,
                            signed_by_name: countersigner.name.clone(),
                            signed_by_email: countersigner.email.clone(),
                            signed_by_title: countersigner.title.clone(),
                            signature_data: None,
                            signature_method: AUTOMATIC_COUNTERSIGNATURE.to_string(),
                            ip_address: None,
                            user_agent: None,
                            verification_token: None,
                            signer_role: diesel_models::enums::QuoteSignerRoleEnum::Countersigner,
                            revision: Some(row.revision),
                            document_hash: document_hash.clone(),
                            consent_text: None,
                            otp_verified_at: None,
                        }
                        .insert(conn)
                        .await
                        .map_err(db_err)?;
                        Self::append_quote_evidence_tx(
                            conn,
                            tenant_id,
                            &row,
                            QuoteEvidenceNew {
                                kind: QuoteEvidenceKind::Countersigned,
                                actor_email: Some(countersigner.email.clone()),
                                origin: RequestOrigin::default(),
                                details: serde_json::json!({
                                    "name": countersigner.name,
                                    "title": countersigner.title,
                                    "method": AUTOMATIC_COUNTERSIGNATURE,
                                }),
                            },
                        )
                        .await?;
                    }
                }

                let signing_now = |email: &str| {
                    input.role == QuoteSignerRoleEnum::Countersigner
                        && same_email(email, &input.signed_by_email)
                };
                let manual_countersigners_left = quote.countersigners.iter().any(|c| {
                    let done = signing_now(&c.email)
                        || signed(&c.email, diesel_models::enums::QuoteSignerRoleEnum::Countersigner);
                    !c.automatic && !done
                });

                let accepted = customers_done && !manual_countersigners_left;
                if accepted {
                    self.accept_quote_tx(conn, &input.actor, input.quote_id, tenant_id)
                        .await?;
                    Self::append_quote_evidence_tx(
                        conn,
                        tenant_id,
                        &row,
                        QuoteEvidenceNew {
                            kind: QuoteEvidenceKind::Accepted,
                            actor_email: None,
                            origin: RequestOrigin::default(),
                            details: serde_json::json!({}),
                        },
                    )
                    .await?;
                }

                Ok(SignQuoteOutcome {
                    signature,
                    accepted,
                    awaiting_countersignature: customers_done && manual_countersigners_left,
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn notify_quote_countersigners(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                let quote: Quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(db_err)?
                    .try_into()?;
                let signatures = QuoteSignatureRow::list_by_quote_id(conn, quote_id)
                    .await
                    .map_err(db_err)?;
                let mut messages = vec![];
                for countersigner in quote.countersigners.iter().filter(|c| {
                    !c.automatic
                        && !signatures
                            .iter()
                            .any(|s| same_email(&s.signed_by_email, &c.email))
                }) {
                    messages.push(
                        self.quote_ready_email_tx(
                            conn,
                            tenant_id,
                            quote_id,
                            &countersigner.email,
                            Some("The customer has signed this quote: it awaits your countersignature.".to_string()),
                        )
                        .await?,
                    );
                }
                if messages.is_empty() {
                    return Ok(());
                }
                self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, messages)
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_quote_signature_certificate(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteSignatureCertificate>> {
        let mut conn = self.get_conn().await?;
        QuoteSignatureCertificateRow::find(&mut conn, tenant_id, quote_id)
            .await
            .map(|r| r.map(Into::into))
            .map_err(db_err)
    }

    async fn save_quote_signature_certificate(
        &self,
        tenant_id: TenantId,
        certificate: QuoteSignatureCertificate,
    ) -> StoreResult<QuoteSignatureCertificate> {
        self.transaction(|conn| {
            async move {
                let quote = QuoteRow::find_by_id_for_update(conn, tenant_id, certificate.quote_id)
                    .await
                    .map_err(db_err)?;
                let inserted = QuoteSignatureCertificateRow {
                    quote_id: certificate.quote_id,
                    tenant_id,
                    revision: certificate.revision,
                    document_hash: certificate.document_hash.clone(),
                    evidence_hash: certificate.evidence_hash.clone(),
                    pdf_document_id: certificate.pdf_document_id,
                    pdf_hash: certificate.pdf_hash.clone(),
                    seal_key_id: certificate.seal_key_id.clone(),
                    seal_signature: certificate.seal_signature.clone(),
                    timestamp_authority: certificate.timestamp_authority.clone(),
                    timestamp_token: certificate.timestamp_token.clone(),
                    created_at: certificate.created_at,
                    document_included: certificate.document_included,
                }
                .insert_if_absent(conn)
                .await
                .map_err(db_err)?;
                let Some(saved) = inserted else {
                    return QuoteSignatureCertificateRow::find(
                        conn,
                        tenant_id,
                        certificate.quote_id,
                    )
                    .await
                    .map_err(db_err)?
                    .map(Into::into)
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "Quote signature certificate".to_string(),
                        ))
                    });
                };
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &quote,
                    QuoteEvidenceNew {
                        kind: QuoteEvidenceKind::CertificateIssued,
                        actor_email: None,
                        origin: RequestOrigin::default(),
                        details: serde_json::json!({
                            "evidence_hash": certificate.evidence_hash,
                            "pdf_hash": certificate.pdf_hash,
                            "sealed": certificate.seal_signature.is_some(),
                            "timestamped": certificate.timestamp_token.is_some(),
                        }),
                    },
                )
                .await?;
                Ok(saved.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_quote_deal_links(
        &self,
        subscription_ids: Vec<common_domain::ids::SubscriptionId>,
    ) -> StoreResult<Vec<(common_domain::ids::SubscriptionId, String)>> {
        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.get_conn().await?;
        QuoteRow::list_deal_links_by_subscription_ids(&mut conn, &subscription_ids)
            .await
            .map_err(db_err)
    }

    async fn sync_quote_to_hubspot(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<()> {
        use crate::domain::pgmq::{HubspotSyncQuote, HubspotSyncRequestEvent};

        let mut conn = self.get_conn().await?;
        let quote = QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(db_err)?;
        if quote.hubspot_deal_id.is_none() {
            return Ok(());
        }
        let message = HubspotSyncRequestEvent::Quote(Box::new(HubspotSyncQuote {
            id: quote_id,
            tenant_id,
        }))
        .try_into()?;
        self.pgmq_send_batch(PgmqQueue::HubspotSync, vec![message])
            .await
    }
}

/// Stores the hash of the quote's current document; inside the edit's transaction, a signer
/// never sees new content with the previous hash.
pub(crate) async fn refresh_quote_document_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<QuoteDocument> {
    // A coupon edit checks the open quotes under its lock: the terms read here are the ones it
    // commits, never ones it is about to replace.
    diesel_models::coupons::CouponRow::lock_shared_by_quote_id(conn, tenant_id, quote_id)
        .await
        .map_err(db_err)?;
    let detailed =
        crate::repositories::quotes::detailed_quote_tx(conn, tenant_id, quote_id).await?;

    let document = QuoteDocument::from_detailed(&detailed);
    QuoteRow::update_by_id(
        conn,
        tenant_id,
        quote_id,
        QuoteRowUpdate {
            document_hash: Some(Some(document.hash())),
            max_discount_percent: Some(max_discount_percent(&detailed, &detailed.coupon_details)),
            ..Default::default()
        },
    )
    .await
    .map_err(db_err)?;
    Ok(document)
}

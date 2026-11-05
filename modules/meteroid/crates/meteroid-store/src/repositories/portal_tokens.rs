use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::pgmq::{PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::domain::portal_tokens::{
    IssuedPortalToken, PortalToken, PortalTokenStatus, ResourceKey, canonical_scopes,
    derive_portal_token, hash_portal_token, invoice_link_expiry, is_portal_token,
    min_link_validity, quote_link_expiry, scope_names,
};
use crate::errors::StoreError;
use crate::jwt_claims::{PortalScope, ResourceAccess};
use crate::repositories::pgmq::PgmqInterface;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use cached::proc_macro::cached;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerId, InvoiceId, PortalTokenId, QuoteId, SavedViewId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::InvoicePaymentStatus;
use diesel_models::invoices::InvoiceRow;
use diesel_models::portal_tokens::{PortalTokenRow, PortalTokenRowNew};
use diesel_models::quotes::QuoteRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Throttles the recovery email a single expired link can trigger.
const RECOVERY_COOLDOWN_MINUTES: i64 = 60;
const LAST_USED_GRANULARITY_MINUTES: i64 = 60;

/// Which live tokens may be handed out again instead of minting a new one.
pub(crate) struct Reuse {
    valid_until: DateTime<Utc>,
    expires_before: Option<DateTime<Utc>>,
}

impl Reuse {
    fn valid_until(valid_until: DateTime<Utc>) -> Option<Self> {
        Some(Self {
            valid_until,
            expires_before: None,
        })
    }
}

pub async fn forget_cached_portal_tokens() {
    use cached::Cached;
    AUTHENTICATE_PORTAL_TOKEN_CACHED.lock().await.cache_clear();
}

/// Revocation reaches other instances once their entry ages out.
#[cached(
    result = true,
    size = 500,
    time = 30,
    key = "String",
    convert = r#"{ hash_portal_token(token) }"#
)]
async fn authenticate_portal_token_cached(
    store: &Store,
    token: &str,
) -> StoreResult<Option<PortalToken>> {
    let portal_token = store.resolve_portal_token(token).await?;
    if let Some(portal_token) = &portal_token
        && let Err(err) = store.touch_portal_token(portal_token).await
    {
        log::warn!("Failed to record portal token use: {err:?}");
    }
    Ok(portal_token)
}

#[async_trait::async_trait]
pub trait PortalTokensInterface {
    async fn invoice_portal_token(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        delegated: bool,
    ) -> StoreResult<String>;

    async fn quote_portal_token(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        recipient_email: String,
        delegated: bool,
    ) -> StoreResult<String>;

    async fn checkout_portal_token(
        &self,
        tenant_id: TenantId,
        session_id: CheckoutSessionId,
        delegated: bool,
    ) -> StoreResult<String>;

    /// Reuses a live token with the same scopes and delegation that has 90-100% of `ttl` left, so frequent
    /// callers don't mint a row each time and never get more lifetime than they asked for.
    async fn customer_portal_token(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        ttl: Duration,
        scopes: Option<Vec<PortalScope>>,
        delegated: bool,
    ) -> StoreResult<IssuedPortalToken>;

    async fn dashboard_portal_token(
        &self,
        tenant_id: TenantId,
        saved_view_id: SavedViewId,
    ) -> StoreResult<String>;

    async fn resolve_portal_token(&self, token: &str) -> StoreResult<Option<PortalToken>>;

    /// `resolve_portal_token` for the auth path: briefly cached, and records the use.
    async fn authenticate_portal_token(&self, token: &str) -> StoreResult<Option<PortalToken>>;

    async fn touch_portal_token(&self, token: &PortalToken) -> StoreResult<()>;

    async fn revoke_portal_tokens(
        &self,
        tenant_id: TenantId,
        resource: &ResourceAccess,
    ) -> StoreResult<()>;

    /// Re-sends the email behind an expired invoice or quote link when `email` is one of its
    /// recipients. Says nothing about the outcome, so it can't be used to probe addresses.
    async fn recover_portal_link(&self, token: &str, email: &str) -> StoreResult<()>;

    async fn purge_stale_portal_tokens(&self) -> StoreResult<usize>;
}

impl Store {
    /// Hands out a live token matching `reuse`, else mints one ending at `expires_at`.
    /// `scopes` only applies to customer tokens.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn issue_portal_token_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        resource: &ResourceAccess,
        scopes: Option<&[PortalScope]>,
        delegated: bool,
        expires_at: Option<DateTime<Utc>>,
        reuse: Option<Reuse>,
    ) -> StoreResult<IssuedPortalToken> {
        let key = ResourceKey::from(resource);
        let secret = &self.settings.jwt_secret;
        let scopes = match resource {
            ResourceAccess::Customer(_) => scope_names(&canonical_scopes(scopes)),
            _ => Vec::new(),
        };

        if let Some(reuse) = reuse {
            let reusable = PortalTokenRow::list_reusable(
                conn,
                tenant_id,
                key.kind,
                key.id,
                key.recipient_email.as_deref(),
                &scopes,
                delegated,
                reuse.valid_until,
                reuse.expires_before,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            let reused = reusable.into_iter().find_map(|row| {
                let token = derive_portal_token(secret, row.id);
                (hash_portal_token(&token) == row.token_hash).then_some(IssuedPortalToken {
                    token,
                    expires_at: row.expires_at,
                })
            });
            if let Some(issued) = reused {
                return Ok(issued);
            }
        }

        let id = PortalTokenId::new();
        let token = derive_portal_token(secret, id);
        let row = PortalTokenRowNew {
            id,
            tenant_id,
            token_hash: hash_portal_token(&token),
            resource_kind: key.kind,
            resource_id: key.id,
            recipient_email: key.recipient_email,
            scopes,
            delegated,
            expires_at,
        }
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(IssuedPortalToken {
            token,
            expires_at: row.expires_at,
        })
    }

    pub(crate) async fn quote_portal_token_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote: &QuoteRow,
        recipient_email: String,
        delegated: bool,
    ) -> StoreResult<String> {
        let now = Utc::now();
        self.issue_portal_token_tx(
            conn,
            tenant_id,
            &ResourceAccess::Quote {
                quote_id: quote.id,
                recipient_email,
            },
            None,
            delegated,
            Some(quote_link_expiry(quote.expires_at, now)),
            Reuse::valid_until(now + min_link_validity()),
        )
        .await
        .map(|issued| issued.token)
    }

    pub(crate) async fn checkout_portal_token_tx(
        &self,
        conn: &mut PgConn,
        session: &CheckoutSessionRow,
        delegated: bool,
    ) -> StoreResult<String> {
        let reuse_until = session
            .expires_at
            .map_or(Utc::now() + min_link_validity(), |exp| {
                exp.min(Utc::now() + min_link_validity())
            });
        self.issue_portal_token_tx(
            conn,
            session.tenant_id,
            &ResourceAccess::CheckoutSession(session.id),
            None,
            delegated,
            session.expires_at,
            Reuse::valid_until(reuse_until),
        )
        .await
        .map(|issued| issued.token)
    }

    async fn recover_invoice_link(
        &self,
        conn: &mut PgConn,
        token: &PortalToken,
        invoice_id: InvoiceId,
        email: &str,
    ) -> StoreResult<()> {
        let invoice = InvoiceRow::find_by_id(conn, token.tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &token.tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let Some(recipient) = customer
            .invoicing_emails
            .iter()
            .flatten()
            .map(|e| e.trim())
            .find(|e| e.eq_ignore_ascii_case(email))
            .map(str::to_string)
        else {
            return Ok(());
        };
        if !self.claim_recovery(conn, token.id).await? {
            return Ok(());
        }

        let kind = if invoice.payment_status == InvoicePaymentStatus::Paid {
            EmailKind::InvoicePaid
        } else {
            EmailKind::InvoiceReady
        };
        let message: PgmqMessageNew = SendEmailRequest::CustomerEmail {
            tenant_id: token.tenant_id,
            kind,
            context: EmailContext::Invoice { invoice_id },
            dedupe_key: format!(
                "{kind}:link_recovery:{}:{}",
                token.id,
                Utc::now().timestamp()
            ),
            resent_from: None,
            manual: true,
            attempts: Default::default(),
            recipient: Some(recipient),
        }
        .try_into()?;
        self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![message])
            .await
    }

    async fn recover_quote_link(
        &self,
        conn: &mut PgConn,
        token: &PortalToken,
        quote_id: QuoteId,
        recipient_email: &str,
        email: &str,
    ) -> StoreResult<()> {
        if !recipient_email.eq_ignore_ascii_case(email.trim()) {
            return Ok(());
        }
        if !self.claim_recovery(conn, token.id).await? {
            return Ok(());
        }
        let message = self
            .quote_ready_email_tx(conn, token.tenant_id, quote_id, recipient_email, None)
            .await?;
        self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, vec![message])
            .await
    }

    async fn claim_recovery(&self, conn: &mut PgConn, id: PortalTokenId) -> StoreResult<bool> {
        PortalTokenRow::claim_recovery(
            conn,
            id,
            Utc::now(),
            Duration::minutes(RECOVERY_COOLDOWN_MINUTES),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }
}

#[async_trait::async_trait]
impl PortalTokensInterface for Store {
    async fn invoice_portal_token(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        delegated: bool,
    ) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;
        let invoice = InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let now = Utc::now();
        self.issue_portal_token_tx(
            &mut conn,
            tenant_id,
            &ResourceAccess::Invoice(invoice_id),
            None,
            delegated,
            Some(invoice_link_expiry(
                invoice.finalized_at,
                invoice.due_at,
                now,
            )),
            Reuse::valid_until(now + min_link_validity()),
        )
        .await
        .map(|issued| issued.token)
    }

    async fn quote_portal_token(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        recipient_email: String,
        delegated: bool,
    ) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;
        let quote = QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        self.quote_portal_token_tx(&mut conn, tenant_id, &quote, recipient_email, delegated)
            .await
    }

    async fn checkout_portal_token(
        &self,
        tenant_id: TenantId,
        session_id: CheckoutSessionId,
        delegated: bool,
    ) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;
        let session = CheckoutSessionRow::get_by_id(&mut conn, tenant_id, session_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        self.checkout_portal_token_tx(&mut conn, &session, delegated)
            .await
    }

    async fn customer_portal_token(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        ttl: Duration,
        scopes: Option<Vec<PortalScope>>,
        delegated: bool,
    ) -> StoreResult<IssuedPortalToken> {
        let expires_at = Utc::now() + ttl;
        let mut conn = self.get_conn().await?;
        self.issue_portal_token_tx(
            &mut conn,
            tenant_id,
            &ResourceAccess::Customer(customer_id),
            scopes.as_deref(),
            delegated,
            Some(expires_at),
            Some(Reuse {
                valid_until: expires_at - ttl / 10,
                expires_before: Some(expires_at),
            }),
        )
        .await
    }

    async fn dashboard_portal_token(
        &self,
        tenant_id: TenantId,
        saved_view_id: SavedViewId,
    ) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;
        self.issue_portal_token_tx(
            &mut conn,
            tenant_id,
            &ResourceAccess::Dashboard(saved_view_id),
            None,
            false,
            None,
            Reuse::valid_until(Utc::now()),
        )
        .await
        .map(|issued| issued.token)
    }

    async fn resolve_portal_token(&self, token: &str) -> StoreResult<Option<PortalToken>> {
        if !is_portal_token(token) {
            return Ok(None);
        }
        let mut conn = self.get_conn().await?;
        PortalTokenRow::find_by_hash(&mut conn, &hash_portal_token(token))
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .map(PortalToken::try_from)
            .transpose()
    }

    async fn authenticate_portal_token(&self, token: &str) -> StoreResult<Option<PortalToken>> {
        authenticate_portal_token_cached(self, token).await
    }

    async fn touch_portal_token(&self, token: &PortalToken) -> StoreResult<()> {
        let now = Utc::now();
        let granularity = Duration::minutes(LAST_USED_GRANULARITY_MINUTES);
        if token
            .last_used_at
            .is_some_and(|used| used > now - granularity)
        {
            return Ok(());
        }
        let mut conn = self.get_conn().await?;
        PortalTokenRow::touch_last_used(&mut conn, token.id, now, granularity)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn revoke_portal_tokens(
        &self,
        tenant_id: TenantId,
        resource: &ResourceAccess,
    ) -> StoreResult<()> {
        let key = ResourceKey::from(resource);
        let mut conn = self.get_conn().await?;
        PortalTokenRow::revoke_for_resource(&mut conn, tenant_id, key.kind, key.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        forget_cached_portal_tokens().await;
        Ok(())
    }

    async fn recover_portal_link(&self, token: &str, email: &str) -> StoreResult<()> {
        let Some(portal_token) = self.resolve_portal_token(token).await? else {
            return Ok(());
        };
        if portal_token.status(Utc::now()) != PortalTokenStatus::Expired {
            return Ok(());
        }
        self.transaction(|conn| {
            async move {
                match &portal_token.resource {
                    ResourceAccess::Invoice(invoice_id) => {
                        self.recover_invoice_link(conn, &portal_token, *invoice_id, email)
                            .await
                    }
                    ResourceAccess::Quote {
                        quote_id,
                        recipient_email,
                    } => {
                        self.recover_quote_link(
                            conn,
                            &portal_token,
                            *quote_id,
                            recipient_email,
                            email,
                        )
                        .await
                    }
                    ResourceAccess::Customer(_)
                    | ResourceAccess::CheckoutSession(_)
                    | ResourceAccess::Dashboard(_) => Ok(()),
                }
            }
            .scope_boxed()
        })
        .await
    }

    async fn purge_stale_portal_tokens(&self) -> StoreResult<usize> {
        let now = Utc::now();
        let mut conn = self.get_conn().await?;
        PortalTokenRow::delete_stale(
            &mut conn,
            now - Duration::days(90),
            now - Duration::days(365),
            now - Duration::days(7),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }
}

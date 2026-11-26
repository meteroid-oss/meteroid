use chrono::{Duration, NaiveDateTime};
use common_domain::ids::{
    BaseId, EventId, TenantId, WebhookAttemptId, WebhookDeliveryId, WebhookEndpointId,
};
use diesel_models::enums::WebhookDeliveryStatusEnum;
use diesel_models::webhooks_out::{
    WebhookOutAttemptRow, WebhookOutAttemptRowNew, WebhookOutDeliveryRow, WebhookOutDeliveryRowNew,
    WebhookOutEndpointRow, WebhookOutEndpointRowNew, WebhookOutEndpointRowPatch,
    WebhookOutMessageRow, WebhookOutMessageRowNew, WebhookOutMessageVersionRow,
    WebhookOutRetiredSecretRow, WebhookOutRetiredSecretRowNew,
};
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;
use secrecy::SecretString;
use std::collections::HashMap;

use crate::crypt;
use crate::domain::enums::{WebhookDeliveryStatus, WebhookEndpointDisabledReason};
use crate::domain::misc::{PaginatedVec, PaginationRequest};
use crate::domain::webhooks_out::{
    ImportedSvixWebhookEndpoint, SvixWebhookEndpointImport, WebhookAttemptOutcome, WebhookDelivery,
    WebhookDeliveryDetails, WebhookDeliveryListItem, WebhookEndpoint, WebhookEndpointHeader,
    WebhookEndpointNew, WebhookEndpointPatch, WebhookEndpointWithSecrets, WebhookHeaderInput,
    WebhookOutMessage, WebhookOutMessageNew, WebhookOutMessageVersion, WebhookOutTarget,
};
use crate::domain::webhooks_out::{WebhookDailyStats, WebhookEndpointStats};
use crate::errors::StoreError;
use crate::services::webhooks::out::headers::{SensitiveHeaderValues, merge_headers};
use crate::services::webhooks::out::schedule::{MAX_ATTEMPTS, PROBE_THRESHOLD, RETRY_DELAYS_SECS};
use crate::services::webhooks::out::secret::generate_secret;
use crate::store::PgConn;
use crate::{Store, StoreResult};

/// A rotated-out secret keeps verifying consumer-side for this long: every secret rotated out
/// within it signs beside the current one.
pub const PREVIOUS_SECRET_GRACE: Duration = Duration::hours(24);

/// Rotated-out secrets an endpoint may have signing at once: each adds a signature to every
/// delivery, so rotations past this within the grace wait for the oldest to expire.
pub const MAX_RETIRED_SECRETS: usize = 10;

/// Non-archived endpoints a tenant may create.
pub const MAX_ENDPOINTS_PER_TENANT: i64 = 50;

#[async_trait::async_trait]
pub trait WebhookOutInterface {
    async fn list_webhook_endpoints(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<WebhookEndpoint>>;

    async fn get_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint>;

    /// Returns the endpoint and the plaintext signing secret, which is never readable
    /// again except through the reveal and delivery paths.
    async fn create_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        endpoint: WebhookEndpointNew,
    ) -> StoreResult<(WebhookEndpoint, SecretString)>;

    /// Skipped when the tenant already has a non-archived endpoint with the same url, so
    /// re-running an import never duplicates. Not capped by `MAX_ENDPOINTS_PER_TENANT`:
    /// it carries over endpoints that already exist.
    async fn import_svix_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        endpoint: ImportedSvixWebhookEndpoint,
    ) -> StoreResult<SvixWebhookEndpointImport>;

    async fn update_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        patch: WebhookEndpointPatch,
    ) -> StoreResult<WebhookEndpoint>;

    /// The endpoint archived, its pending deliveries cancelled.
    async fn archive_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint>;

    async fn get_webhook_endpoint_secret(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<SecretString>;

    /// A new signing secret; the one it replaces is retired, signing beside it for
    /// `PREVIOUS_SECRET_GRACE`, as every other retired within that window still does. Refused
    /// (`FailedPrecondition`) past `MAX_RETIRED_SECRETS` such secrets.
    async fn rotate_webhook_endpoint_secret(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<SecretString>;

    async fn list_active_webhook_endpoints_for_tenants(
        &self,
        tenant_ids: &[TenantId],
    ) -> StoreResult<Vec<WebhookEndpoint>>;

    /// Stores the message, its payload at each version in `versions`, and one PENDING delivery
    /// per target, sending the payload of the target's version. Idempotent on the message id: a
    /// message fanned out again adds what is missing only.
    async fn insert_webhook_message_with_deliveries(
        &self,
        message: WebhookOutMessageNew,
        versions: Vec<WebhookOutMessageVersion>,
        targets: &[WebhookOutTarget],
    ) -> StoreResult<()>;

    async fn reclaim_expired_webhook_deliveries(&self) -> StoreResult<usize>;

    async fn claim_due_webhook_deliveries(
        &self,
        limit: i64,
        lease_secs: i32,
    ) -> StoreResult<Vec<WebhookDelivery>>;

    async fn get_webhook_endpoint_for_delivery(
        &self,
        endpoint_id: WebhookEndpointId,
    ) -> StoreResult<Option<WebhookEndpointWithSecrets>>;

    /// The message as a delivery at `api_version` sends it (v1's payload without one).
    async fn get_webhook_message(
        &self,
        message_id: EventId,
        api_version: Option<&str>,
    ) -> StoreResult<WebhookOutMessage>;

    async fn record_webhook_attempt(&self, outcome: WebhookAttemptOutcome) -> StoreResult<()>;

    /// Hands a claimed delivery back for `next_attempt_at` when we could not even try it:
    /// no attempt is logged, no budget spent, and the endpoint's health is left alone.
    async fn defer_webhook_delivery(
        &self,
        delivery_id: WebhookDeliveryId,
        claimed_attempt_count: i32,
        lease_expires_at: NaiveDateTime,
        next_attempt_at: NaiveDateTime,
        reason: String,
    ) -> StoreResult<()>;

    /// Deletes in bounded chunks until nothing older than `cutoff` is left; returns the total.
    async fn purge_webhook_messages_older_than(
        &self,
        cutoff: NaiveDateTime,
        chunk_size: i64,
    ) -> StoreResult<usize>;

    async fn list_webhook_deliveries(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        status: Option<WebhookDeliveryStatus>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<WebhookDeliveryListItem>>;

    /// A page of the endpoint's deliveries by keyset (API v2): newest first, those older than
    /// `before`, at most `limit`; None when `before` is not one of the endpoint's deliveries.
    /// `attempts` is not counted (`attempt_count` says it).
    async fn list_webhook_deliveries_before(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        status: Option<Vec<WebhookDeliveryStatus>>,
        before: Option<WebhookDeliveryId>,
        limit: usize,
    ) -> StoreResult<Option<Vec<WebhookDeliveryListItem>>>;

    async fn get_webhook_delivery(
        &self,
        tenant_id: TenantId,
        id: WebhookDeliveryId,
    ) -> StoreResult<WebhookDeliveryDetails>;

    async fn resend_webhook_delivery(
        &self,
        tenant_id: TenantId,
        id: WebhookDeliveryId,
    ) -> StoreResult<WebhookDelivery>;

    /// A delivery made by hand (a test event): `version` is the payload it sends when the
    /// endpoint's version has payloads of its own, v1's (`message.payload`) without.
    async fn create_manual_webhook_delivery(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        message: WebhookOutMessageNew,
        version: Option<WebhookOutMessageVersion>,
    ) -> StoreResult<WebhookDelivery>;

    /// Ends a pause now, so the oldest due delivery is sent as the next probe. The
    /// endpoint stays unreachable (`paused_until` set, one delivery at a time) until a
    /// probe gets an answer.
    async fn resume_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint>;

    /// One entry per day since `since` that has deliveries with an outcome, oldest first.
    async fn webhook_endpoint_daily_stats(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<WebhookDailyStats>>;

    /// Deliveries since `since` by outcome, of every endpoint that has some.
    async fn webhook_endpoints_stats(
        &self,
        tenant_id: TenantId,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<WebhookEndpointStats>>;
}

impl Store {
    fn encrypt_webhook_secret(&self, secret: &SecretString) -> StoreResult<String> {
        use secrecy::ExposeSecret;

        crypt::encrypt(&self.settings.crypt_key, secret.expose_secret())
            .change_context(StoreError::CryptError("webhook secret encryption".into()))
    }

    fn decrypt_webhook_secret(&self, encrypted: &str) -> StoreResult<SecretString> {
        crypt::decrypt(&self.settings.crypt_key, encrypted)
            .change_context(StoreError::CryptError("webhook secret decryption".into()))
    }

    fn decrypt_sensitive_headers(
        &self,
        encrypted: Option<&str>,
    ) -> StoreResult<SensitiveHeaderValues> {
        use secrecy::ExposeSecret;

        let Some(encrypted) = encrypted else {
            return Ok(SensitiveHeaderValues::new());
        };
        let json = self.decrypt_webhook_secret(encrypted)?;
        serde_json::from_str(json.expose_secret())
            .map_err(|e| Report::new(StoreError::SerdeError("webhook headers".into(), e)))
    }

    /// The stored form of a header list: the JSON column and the encrypted sensitive
    /// values (`None` when there are none).
    fn store_headers(
        &self,
        input: Vec<WebhookHeaderInput>,
        previous: &SensitiveHeaderValues,
    ) -> StoreResult<(
        Vec<WebhookEndpointHeader>,
        serde_json::Value,
        Option<String>,
    )> {
        let (headers, sensitive) = merge_headers(input, previous);

        let json = serde_json::to_value(&headers)
            .map_err(|e| Report::new(StoreError::SerdeError("webhook headers".into(), e)))?;

        let encrypted = if sensitive.is_empty() {
            None
        } else {
            let plain = serde_json::to_string(&sensitive)
                .map_err(|e| Report::new(StoreError::SerdeError("webhook headers".into(), e)))?;
            Some(self.encrypt_webhook_secret(&SecretString::from(plain))?)
        };

        Ok((headers, json, encrypted))
    }
}

fn needs_setup(headers: &[WebhookEndpointHeader]) -> bool {
    headers.iter().any(|h| h.sensitive && !h.set)
}

const NEEDS_SETUP_MESSAGE: &str =
    "Set a value for every sensitive header before enabling this endpoint";

async fn disable_endpoint_tx(
    conn: &mut PgConn,
    endpoint_id: WebhookEndpointId,
    reason: WebhookEndpointDisabledReason,
    now: NaiveDateTime,
) -> StoreResult<()> {
    WebhookOutEndpointRow::disable(conn, endpoint_id, reason.into(), now)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    WebhookOutDeliveryRow::cancel_pending_for_endpoint(conn, endpoint_id, now)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    Ok(())
}

#[async_trait::async_trait]
impl WebhookOutInterface for Store {
    async fn list_webhook_endpoints(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<WebhookEndpoint>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutEndpointRow::list_by_tenant(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint> {
        let mut conn = self.get_conn().await?;

        let row = WebhookOutEndpointRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn create_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        endpoint: WebhookEndpointNew,
    ) -> StoreResult<(WebhookEndpoint, SecretString)> {
        let secret = generate_secret();

        let (headers, headers_json, sensitive_header_values) =
            self.store_headers(endpoint.headers, &SensitiveHeaderValues::new())?;
        if needs_setup(&headers) {
            return Err(Report::new(StoreError::InvalidArgument(
                "Every sensitive header needs a value".to_string(),
            )));
        }

        let row_new = WebhookOutEndpointRowNew {
            id: WebhookEndpointId::new(),
            tenant_id,
            url: endpoint.url,
            description: endpoint.description,
            event_types: endpoint.event_types,
            secret: self.encrypt_webhook_secret(&secret)?,
            max_in_flight: endpoint.max_in_flight,
            headers: headers_json,
            sensitive_header_values,
            disabled: false,
            disabled_reason: None,
            disabled_at: None,
            rate_limit_per_sec: endpoint.rate_limit_per_sec,
            legacy_svix_headers: false,
            api_version: endpoint.api_version,
        };

        let row = self
            .transaction(|conn| {
                async move {
                    WebhookOutEndpointRow::lock_tenant_endpoints(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let existing = WebhookOutEndpointRow::count_by_tenant(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if existing >= MAX_ENDPOINTS_PER_TENANT {
                        return Err(Report::new(StoreError::FailedPrecondition(format!(
                            "A tenant can have at most {MAX_ENDPOINTS_PER_TENANT} webhook endpoints; delete one before creating another"
                        ))));
                    }

                    row_new
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;

        Ok((row.into(), secret))
    }

    async fn import_svix_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        endpoint: ImportedSvixWebhookEndpoint,
    ) -> StoreResult<SvixWebhookEndpointImport> {
        let now = common_clock::wall().naive_utc();

        let (headers, headers_json, sensitive_header_values) =
            self.store_headers(endpoint.headers, &SensitiveHeaderValues::new())?;
        if !endpoint.disabled && needs_setup(&headers) {
            return Err(Report::new(StoreError::InvalidArgument(
                NEEDS_SETUP_MESSAGE.to_string(),
            )));
        }

        let row_new = WebhookOutEndpointRowNew {
            id: WebhookEndpointId::new(),
            tenant_id,
            url: endpoint.url,
            description: endpoint.description,
            event_types: endpoint.event_types,
            secret: self.encrypt_webhook_secret(&endpoint.secret)?,
            max_in_flight: endpoint.max_in_flight,
            headers: headers_json,
            sensitive_header_values,
            disabled: endpoint.disabled,
            disabled_reason: endpoint
                .disabled
                .then_some(WebhookEndpointDisabledReason::Manual.into()),
            disabled_at: endpoint.disabled.then_some(now),
            rate_limit_per_sec: endpoint.rate_limit_per_sec,
            legacy_svix_headers: true,
            api_version: None,
        };

        self.transaction(|conn| {
            async move {
                WebhookOutEndpointRow::lock_tenant_endpoints(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let existing = WebhookOutEndpointRow::list_by_tenant(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if let Some(row) = existing.iter().find(|row| row.url == row_new.url) {
                    return Ok(SvixWebhookEndpointImport::AlreadyPresent(row.id));
                }

                let row = row_new
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(SvixWebhookEndpointImport::Created(Box::new(row.into())))
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        patch: WebhookEndpointPatch,
    ) -> StoreResult<WebhookEndpoint> {
        let now = common_clock::wall().naive_utc();
        let endpoint_id = patch.id;

        let existing = {
            let mut conn = self.get_conn().await?;
            WebhookOutEndpointRow::get_by_id(&mut conn, tenant_id, endpoint_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };

        let (headers, headers_json, sensitive_header_values) = match patch.headers {
            Some(input) => {
                let previous =
                    self.decrypt_sensitive_headers(existing.sensitive_header_values.as_deref())?;
                let (headers, json, encrypted) = self.store_headers(input, &previous)?;
                (headers, Some(json), Some(encrypted))
            }
            None => (WebhookEndpoint::from(existing.clone()).headers, None, None),
        };

        let enabled_after = !patch.disabled.unwrap_or(existing.disabled);
        if enabled_after && needs_setup(&headers) {
            return Err(Report::new(StoreError::FailedPrecondition(
                NEEDS_SETUP_MESSAGE.to_string(),
            )));
        }

        let reenabling = patch.disabled == Some(false);
        let row_patch = WebhookOutEndpointRowPatch {
            url: patch.url,
            description: patch.description,
            event_types: patch.event_types,
            disabled: patch.disabled,
            // Re-enabling clears why it was disabled and gives the endpoint a clean slate.
            disabled_reason: patch
                .disabled
                .map(|disabled| disabled.then_some(WebhookEndpointDisabledReason::Manual.into())),
            disabled_at: patch.disabled.map(|disabled| disabled.then_some(now)),
            consecutive_failures: reenabling.then_some(0),
            // Otherwise the next failure would pause it again, for up to an hour.
            unreachable_streak: reenabling.then_some(0),
            failing_since: reenabling.then_some(None),
            headers: headers_json,
            sensitive_header_values,
            rate_limit_per_sec: patch.rate_limit_per_sec,
            paused_until: reenabling.then_some(None),
            api_version: patch.api_version,
            updated_at: Some(now),
        };

        let disabling = patch.disabled == Some(true);

        self.transaction(|conn| {
            async move {
                let row = row_patch
                    .patch(conn, tenant_id, endpoint_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if disabling {
                    WebhookOutDeliveryRow::cancel_pending_for_endpoint(conn, endpoint_id, now)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                Ok(row.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn archive_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint> {
        let now = common_clock::wall().naive_utc();

        self.transaction(|conn| {
            async move {
                let archived = WebhookOutEndpointRow::archive(conn, tenant_id, id, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "Webhook endpoint not found".to_string(),
                        ))
                    })?;

                WebhookOutDeliveryRow::cancel_pending_for_endpoint(conn, id, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(archived.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_webhook_endpoint_secret(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<SecretString> {
        let mut conn = self.get_conn().await?;

        let row = WebhookOutEndpointRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.decrypt_webhook_secret(&row.secret)
    }

    async fn rotate_webhook_endpoint_secret(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<SecretString> {
        let secret = generate_secret();
        let encrypted = self.encrypt_webhook_secret(&secret)?;
        let now = common_clock::wall().naive_utc();

        self.transaction(|conn| {
            async move {
                let endpoint = WebhookOutEndpointRow::lock_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                WebhookOutRetiredSecretRow::delete_expired(conn, id, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let live = WebhookOutRetiredSecretRow::list_live(conn, id, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if live.len() >= MAX_RETIRED_SECRETS {
                    return Err(Report::new(StoreError::FailedPrecondition(format!(
                        "This endpoint's secret was rotated {MAX_RETIRED_SECRETS} times in 24 \
                         hours, each still signing: rotate again once the oldest expires"
                    ))));
                }
                WebhookOutRetiredSecretRowNew {
                    id: uuid::Uuid::now_v7(),
                    tenant_id,
                    endpoint_id: id,
                    secret: endpoint.secret,
                    expires_at: now + PREVIOUS_SECRET_GRACE,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                WebhookOutEndpointRow::rotate_secret(conn, tenant_id, id, encrypted, now)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        Ok(secret)
    }

    async fn list_active_webhook_endpoints_for_tenants(
        &self,
        tenant_ids: &[TenantId],
    ) -> StoreResult<Vec<WebhookEndpoint>> {
        if tenant_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut conn = self.get_conn().await?;

        let rows = WebhookOutEndpointRow::list_active_by_tenants(&mut conn, tenant_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn insert_webhook_message_with_deliveries(
        &self,
        message: WebhookOutMessageNew,
        versions: Vec<WebhookOutMessageVersion>,
        targets: &[WebhookOutTarget],
    ) -> StoreResult<()> {
        if targets.is_empty() {
            return Ok(());
        }

        let now = common_clock::wall().naive_utc();
        let message_id = message.id;
        let tenant_id = message.tenant_id;

        let message_row = WebhookOutMessageRowNew {
            id: message.id,
            tenant_id: message.tenant_id,
            event_type: message.event_type,
            payload: message.payload,
        };

        let version_rows: Vec<WebhookOutMessageVersionRow> = versions
            .into_iter()
            .map(|version| WebhookOutMessageVersionRow {
                message_id,
                api_version: version.api_version,
                payload: version.payload,
            })
            .collect();

        let delivery_rows: Vec<WebhookOutDeliveryRowNew> = targets
            .iter()
            .map(|target| WebhookOutDeliveryRowNew {
                id: WebhookDeliveryId::new(),
                tenant_id,
                message_id,
                endpoint_id: target.endpoint_id,
                status: WebhookDeliveryStatusEnum::Pending,
                next_attempt_at: Some(now),
                manual: false,
                api_version: target.api_version.clone(),
            })
            .collect();

        self.transaction(|conn| {
            async move {
                message_row
                    .insert_ignore_conflict(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                WebhookOutMessageVersionRow::insert_batch_ignore_conflict(conn, &version_rows)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                WebhookOutDeliveryRowNew::insert_batch_ignore_conflict(conn, &delivery_rows)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn reclaim_expired_webhook_deliveries(&self) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;

        WebhookOutDeliveryRow::reclaim_expired(
            &mut conn,
            common_clock::wall().naive_utc(),
            MAX_ATTEMPTS,
            &RETRY_DELAYS_SECS,
        )
        .await
        .map_err(Into::into)
    }

    async fn claim_due_webhook_deliveries(
        &self,
        limit: i64,
        lease_secs: i32,
    ) -> StoreResult<Vec<WebhookDelivery>> {
        if limit <= 0 {
            return Ok(vec![]);
        }

        let mut conn = self.get_conn().await?;

        let rows = WebhookOutDeliveryRow::claim_due(
            &mut conn,
            limit,
            lease_secs,
            common_clock::wall().naive_utc(),
            PROBE_THRESHOLD,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_webhook_endpoint_for_delivery(
        &self,
        endpoint_id: WebhookEndpointId,
    ) -> StoreResult<Option<WebhookEndpointWithSecrets>> {
        let mut conn = self.get_conn().await?;

        let Some(row) = WebhookOutEndpointRow::get_by_id_unscoped(&mut conn, endpoint_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };

        let secret = self.decrypt_webhook_secret(&row.secret)?;

        // Every secret rotated out within the grace, the newest first; and the single one a
        // release before `webhook_out_retired_secret` kept on the endpoint, while it lasts.
        let now = common_clock::wall().naive_utc();
        let mut previous_secrets = Vec::new();
        for retired in WebhookOutRetiredSecretRow::list_live(&mut conn, endpoint_id, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            previous_secrets.push(self.decrypt_webhook_secret(&retired.secret)?);
        }
        if let (Some(previous), Some(expires_at)) =
            (&row.previous_secret, row.previous_secret_expires_at)
            && expires_at > now
        {
            previous_secrets.push(self.decrypt_webhook_secret(previous)?);
        }

        let sensitive = self.decrypt_sensitive_headers(row.sensitive_header_values.as_deref())?;
        let endpoint: WebhookEndpoint = row.into();

        let headers = endpoint
            .headers
            .iter()
            .filter_map(|header| {
                let value = if header.sensitive {
                    sensitive.get(&header.name).cloned()
                } else {
                    header.value.clone()
                };
                value.map(|value| (header.name.clone(), SecretString::from(value)))
            })
            .collect();

        Ok(Some(WebhookEndpointWithSecrets {
            endpoint,
            secret,
            previous_secrets,
            headers,
        }))
    }

    async fn get_webhook_message(
        &self,
        message_id: EventId,
        api_version: Option<&str>,
    ) -> StoreResult<WebhookOutMessage> {
        let mut conn = self.get_conn().await?;

        WebhookOutMessageRow::get_at_version(&mut conn, message_id, api_version)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn record_webhook_attempt(&self, outcome: WebhookAttemptOutcome) -> StoreResult<()> {
        let now = common_clock::wall().naive_utc();

        let completed_at = outcome.final_status.is_terminal().then_some(now);

        self.transaction(|conn| {
            async move {
                let attempt_number =
                    WebhookOutAttemptRow::next_attempt_number(conn, outcome.delivery_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                WebhookOutAttemptRowNew {
                    id: WebhookAttemptId::new(),
                    delivery_id: outcome.delivery_id,
                    attempt_number,
                    duration_ms: outcome.duration_ms,
                    response_status: outcome.response_status,
                    response_body: outcome.response_body,
                    error: outcome.error.clone(),
                    succeeded: outcome.succeeded,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let updated = WebhookOutDeliveryRow::record_attempt_result(
                    conn,
                    outcome.delivery_id,
                    outcome.attempt_number,
                    outcome.lease_expires_at,
                    outcome.final_status.into(),
                    outcome.next_attempt_at,
                    outcome.response_status,
                    outcome.error,
                    completed_at,
                    outcome.refund_attempt,
                    now,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if updated == 0 {
                    log::warn!(
                        "Webhook delivery {} moved on (resent or lease reclaimed) while attempt {attempt_number} was in flight; keeping the newer state",
                        outcome.delivery_id
                    );
                    return Ok(());
                }

                if !matches!(outcome.final_status, WebhookDeliveryStatus::Cancelled) {
                    WebhookOutEndpointRow::record_delivery_outcome(
                        conn,
                        outcome.endpoint_id,
                        outcome.succeeded,
                        outcome.endpoint_wide_failure,
                        outcome.pause_endpoint_until,
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }

                if let Some(reason) = outcome.disable_endpoint {
                    disable_endpoint_tx(conn, outcome.endpoint_id, reason, now).await?;
                }

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn defer_webhook_delivery(
        &self,
        delivery_id: WebhookDeliveryId,
        claimed_attempt_count: i32,
        lease_expires_at: NaiveDateTime,
        next_attempt_at: NaiveDateTime,
        reason: String,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        WebhookOutDeliveryRow::record_attempt_result(
            &mut conn,
            delivery_id,
            claimed_attempt_count,
            lease_expires_at,
            WebhookDeliveryStatus::Pending.into(),
            Some(next_attempt_at),
            None,
            Some(reason),
            None,
            true,
            common_clock::wall().naive_utc(),
        )
        .await
        .map(|_| ())
        .map_err(Into::into)
    }

    async fn purge_webhook_messages_older_than(
        &self,
        cutoff: NaiveDateTime,
        chunk_size: i64,
    ) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;
        let chunk_size = chunk_size.max(1);
        let mut total = 0usize;

        loop {
            let deleted =
                WebhookOutMessageRow::purge_chunk_older_than(&mut conn, cutoff, chunk_size)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

            total += deleted;

            if deleted == 0 {
                return Ok(total);
            }
        }
    }

    async fn list_webhook_deliveries(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        status: Option<WebhookDeliveryStatus>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<WebhookDeliveryListItem>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutDeliveryRow::list_by_endpoint(
            &mut conn,
            tenant_id,
            endpoint_id,
            status.map(Into::into),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let ids: Vec<WebhookDeliveryId> = rows.items.iter().map(|row| row.delivery.id).collect();
        let attempts: HashMap<WebhookDeliveryId, i64> =
            WebhookOutAttemptRow::count_by_deliveries(&mut conn, &ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .collect();

        Ok(PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(|row| {
                    let mut item = WebhookDeliveryListItem::from(row);
                    item.attempts = attempts.get(&item.delivery.id).copied().unwrap_or(0);
                    item
                })
                .collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_webhook_deliveries_before(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        status: Option<Vec<WebhookDeliveryStatus>>,
        before: Option<WebhookDeliveryId>,
        limit: usize,
    ) -> StoreResult<Option<Vec<WebhookDeliveryListItem>>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutDeliveryRow::list_page_by_endpoint(
            &mut conn,
            tenant_id,
            endpoint_id,
            status.map(|statuses| statuses.into_iter().map(Into::into).collect()),
            before,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.map(|rows| {
            rows.into_iter()
                .map(WebhookDeliveryListItem::from)
                .collect()
        }))
    }

    async fn get_webhook_delivery(
        &self,
        tenant_id: TenantId,
        id: WebhookDeliveryId,
    ) -> StoreResult<WebhookDeliveryDetails> {
        let mut conn = self.get_conn().await?;

        let delivery = WebhookOutDeliveryRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let message = WebhookOutMessageRow::get_at_version(
            &mut conn,
            delivery.message_id,
            delivery.api_version.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let attempts = WebhookOutAttemptRow::list_by_delivery(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(WebhookDeliveryDetails {
            delivery: delivery.into(),
            message: message.into(),
            attempts: attempts.into_iter().map(Into::into).collect(),
        })
    }

    async fn resend_webhook_delivery(
        &self,
        tenant_id: TenantId,
        id: WebhookDeliveryId,
    ) -> StoreResult<WebhookDelivery> {
        let now = common_clock::wall().naive_utc();

        self.transaction(|conn| {
            async move {
                let delivery = WebhookOutDeliveryRow::get_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let endpoint =
                    WebhookOutEndpointRow::get_by_id(conn, tenant_id, delivery.endpoint_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                if endpoint.disabled {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "This webhook endpoint is disabled".to_string(),
                    )));
                }

                let status: WebhookDeliveryStatus = delivery.status.into();

                let leased = delivery
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at > now);

                if status == WebhookDeliveryStatus::InFlight && leased {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "This delivery is currently being sent, retry once it completes"
                            .to_string(),
                    )));
                }

                let row = WebhookOutDeliveryRow::reschedule_now(
                    conn,
                    tenant_id,
                    id,
                    status.is_terminal(),
                    now,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(row.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_manual_webhook_delivery(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        message: WebhookOutMessageNew,
        version: Option<WebhookOutMessageVersion>,
    ) -> StoreResult<WebhookDelivery> {
        let now = common_clock::wall().naive_utc();
        let message_id = message.id;
        let api_version = version.as_ref().map(|version| version.api_version.clone());
        let version_rows: Vec<WebhookOutMessageVersionRow> = version
            .into_iter()
            .map(|version| WebhookOutMessageVersionRow {
                message_id,
                api_version: version.api_version,
                payload: version.payload,
            })
            .collect();

        let message_row = WebhookOutMessageRowNew {
            id: message.id,
            tenant_id: message.tenant_id,
            event_type: message.event_type,
            payload: message.payload,
        };

        self.transaction(|conn| {
            async move {
                let endpoint = WebhookOutEndpointRow::get_by_id(conn, tenant_id, endpoint_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if endpoint.disabled {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "This webhook endpoint is disabled".to_string(),
                    )));
                }

                message_row
                    .insert_ignore_conflict(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                WebhookOutMessageVersionRow::insert_batch_ignore_conflict(conn, &version_rows)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let row = WebhookOutDeliveryRowNew {
                    id: WebhookDeliveryId::new(),
                    tenant_id,
                    message_id,
                    endpoint_id,
                    status: WebhookDeliveryStatusEnum::Pending,
                    next_attempt_at: Some(now),
                    manual: true,
                    api_version,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                Ok(row.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn resume_webhook_endpoint(
        &self,
        tenant_id: TenantId,
        id: WebhookEndpointId,
    ) -> StoreResult<WebhookEndpoint> {
        let now = common_clock::wall().naive_utc();
        let mut conn = self.get_conn().await?;

        WebhookOutEndpointRow::end_pause(&mut conn, tenant_id, id, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let row = WebhookOutEndpointRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn webhook_endpoint_daily_stats(
        &self,
        tenant_id: TenantId,
        endpoint_id: WebhookEndpointId,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<WebhookDailyStats>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutDeliveryRow::daily_stats(&mut conn, tenant_id, endpoint_id, since)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|row| WebhookDailyStats {
                day: row.day,
                succeeded: row.succeeded,
                failed: row.failed,
                retrying: row.retrying,
                cancelled: row.cancelled,
            })
            .collect())
    }

    async fn webhook_endpoints_stats(
        &self,
        tenant_id: TenantId,
        since: NaiveDateTime,
    ) -> StoreResult<Vec<WebhookEndpointStats>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutDeliveryRow::endpoint_stats(&mut conn, tenant_id, since)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|row| WebhookEndpointStats {
                endpoint_id: row.endpoint_id,
                succeeded: row.succeeded,
                failed: row.failed,
                retrying: row.retrying,
                cancelled: row.cancelled,
            })
            .collect())
    }
}

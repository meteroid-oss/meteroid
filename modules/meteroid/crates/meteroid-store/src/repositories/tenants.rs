use crate::domain::{
    InvoicingEntityNew, OrganizationWithTenants, Tenant, TenantEnvironmentEnum, TenantNew,
    TenantUpdate, TenantWithOrganization,
};
use cached::proc_macro::cached;
use cached::{Cached, TimedSizedCache};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::constants::{Currencies, Currency};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::EmailDeliveryMode;
use crate::errors::StoreError;
use crate::repositories::OrganizationsInterface;
use crate::repositories::org_security_audit_log::SecurityAuditInterface;
use crate::store::{PgConn, PgPool, Store, StoreInternal, pool_conn};
use crate::{StoreResult, domain};
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, OrganizationId, TenantId, UserId};
use diesel_models::organizations::OrganizationRow;
use diesel_models::tenants::{TenantRow, TenantRowNew, TenantRowPatch};
use diesel_models::users::UserRow;

/// Tenants hard-deleted per transaction, to bound each cascading delete's size.
const TENANT_HARD_DELETE_CHUNK: usize = 25;

/// First `base`, `base-1`, `base-2`, … not already present in `taken`. `taken` must
/// include archived tenants: the slug unique index `(organization_id, slug)` covers them.
pub fn next_available_slug(base: &str, taken: &[String]) -> String {
    let mut slug = base.to_string();
    let mut i = 1;
    while taken.contains(&slug) {
        slug = format!("{base}-{i}");
        i += 1;
    }
    slug
}

#[async_trait::async_trait]
pub trait TenantInterface {
    async fn insert_tenant(
        &self,
        tenant: TenantNew,
        organization_id: OrganizationId,
    ) -> StoreResult<Tenant>;
    async fn update_tenant(
        &self,
        actor: Actor,
        tenant: TenantUpdate,
        organization_id: OrganizationId,
        tenant_id: TenantId,
    ) -> StoreResult<Tenant>;
    async fn find_tenant_by_id_and_organization(
        &self,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> StoreResult<Tenant>;
    async fn find_tenant_by_slug_and_organization_slug(
        &self,
        slug: String,
        organization_slug: String,
    ) -> StoreResult<Tenant>;
    async fn list_tenants_by_organization_id(
        &self,
        organization_id: OrganizationId,
    ) -> StoreResult<Vec<Tenant>>;

    /// All non-archived sandbox tenants across organizations (for the
    /// sandbox-maintenance worker).
    async fn list_sandbox_tenants(&self) -> StoreResult<Vec<Tenant>>;

    /// Record that a user just interacted with the tenant (throttled write).
    async fn touch_tenant_activity(&self, tenant_id: TenantId) -> StoreResult<()>;

    /// Number of subscriptions whose billing has not yet caught up to today —
    /// non-zero means a seeded sandbox is still generating historical data.
    async fn count_subscriptions_pending_catchup(&self, tenant_id: TenantId) -> StoreResult<u64>;

    /// Non-Production tenants the lifecycle worker manages (ACTIVE or PAUSED).
    async fn list_lifecycle_managed_tenants(&self) -> StoreResult<Vec<Tenant>>;
    /// Pause an inactive non-prod tenant (scheduled workers skip it). Guarded
    /// against Production in the query layer.
    async fn pause_tenant(&self, tenant_id: TenantId) -> StoreResult<()>;
    /// Resume a paused/archived tenant back to ACTIVE (e.g. the user clicked resume).
    async fn resume_tenant(&self, tenant_id: TenantId) -> StoreResult<()>;
    /// Mark an inactive non-prod tenant as archived / pending deletion. Guarded
    /// against Production in the query layer.
    async fn archive_tenant(&self, tenant_id: TenantId) -> StoreResult<()>;

    /// Mark a non-Production tenant `PendingDeletion` for the cleanup worker.
    /// Archives the organization if it was its last active tenant, and returns
    /// whether it did.
    async fn schedule_tenant_deletion(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        tenant_id: TenantId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<bool>;

    /// Up to `limit` tenant ids currently pending deletion — the cleanup worker's
    /// work list, drained in batches.
    async fn list_tenants_pending_deletion(&self, limit: i64) -> StoreResult<Vec<TenantId>>;

    /// Hard-delete the given tenants (Postgres cascade), after the cleanup worker
    /// has purged their external (ClickHouse + object store) data.
    async fn hard_delete_tenants(&self, tenant_ids: &[TenantId]) -> StoreResult<()>;

    async fn get_reporting_currency_by_tenant_id(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Currency>;

    async fn list_tenant_currencies_with_customer_count(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<(String, u64)>>;

    async fn list_tenant_currencies(&self, tenant_id: TenantId) -> StoreResult<Vec<String>>;

    async fn update_tenant_available_currencies(
        &self,
        tenant_id: TenantId,
        currencies: Vec<String>,
    ) -> StoreResult<Vec<String>>;

    async fn find_tenant_by_id(&self, tenant_id: TenantId) -> StoreResult<TenantWithOrganization>;

    /// Resolve multiple tenants in one call, cached.
    /// Transient error returns a partial result instead of failing the entire batch.
    async fn find_tenants_by_ids(
        &self,
        tenant_ids: &[TenantId],
    ) -> StoreResult<Vec<TenantWithOrganization>>;
}

/// The test gateway is refused at connect time on production tenants; flipping the environment
/// afterwards must not smuggle one in.
async fn refuse_production_with_test_gateway(
    conn: &mut PgConn,
    tenant_id: TenantId,
) -> StoreResult<()> {
    use crate::adapters::payment::payment_capabilities;
    use diesel_models::connectors::ConnectorRow;
    use diesel_models::enums::ConnectorTypeEnum;

    let has_test_gateway = ConnectorRow::list_connectors(
        conn,
        tenant_id,
        Some(ConnectorTypeEnum::PaymentProvider),
        None,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .any(|row| payment_capabilities(&row.provider.into()).is_some_and(|caps| caps.is_test_gateway));
    if has_test_gateway {
        return Err(Report::new(StoreError::FailedPrecondition(
            "Disconnect the test gateway before making this tenant a production tenant".to_string(),
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl TenantInterface for Store {
    async fn insert_tenant(
        &self,
        tenant: TenantNew,
        organization_id: OrganizationId,
    ) -> StoreResult<Tenant> {
        let OrganizationWithTenants { organization, .. } = self
            .get_organizations_with_tenants_by_id(organization_id)
            .await?;

        if tenant.environment == TenantEnvironmentEnum::Production {
            self.organization_hooks
                .ensure_production_tenant_allowed(organization_id)
                .await?;
        }

        let invoicing_entity = tenant.invoicing_entity.clone().unwrap_or_default();

        self.transaction(|conn| {
            async move {
                self.internal
                    .insert_tenant_with_default_entities(
                        conn,
                        tenant,
                        organization_id,
                        organization.trade_name.clone(),
                        organization.default_country.clone(),
                        invoicing_entity,
                    )
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_tenant(
        &self,
        actor: Actor,
        tenant: TenantUpdate,
        organization_id: OrganizationId,
        tenant_id: TenantId,
    ) -> StoreResult<Tenant> {
        let res = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    // Snapshot before the update so the entity.updated audit row records
                    // only fields whose value actually changes — the settings form submits
                    // every field, changed or not.
                    // Locked: the delivery-mode rules read the environment and mode they patch.
                    let before = TenantRow::find_by_id_for_update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if tenant.environment == Some(domain::enums::TenantEnvironmentEnum::Production)
                        && before.environment
                            != diesel_models::enums::TenantEnvironmentEnum::Production
                    {
                        refuse_production_with_test_gateway(conn, tenant_id).await?;
                    }
                    let mut tenant = tenant;
                    if tenant.email_delivery_mode.is_none()
                        && tenant.legacy_disable_emails == Some(true)
                        && before.email_delivery_mode
                            == diesel_models::enums::EmailDeliveryModeEnum::Live
                    {
                        tenant.email_delivery_mode = Some(EmailDeliveryMode::Off);
                    }
                    if let Some(recipients) = tenant.email_test_recipients.take() {
                        tenant.email_test_recipients =
                            Some(domain::tenants::normalize_test_recipients(recipients)?);
                    }
                    let environment = tenant
                        .environment
                        .clone()
                        .unwrap_or_else(|| before.environment.clone().into());
                    let mode = tenant
                        .email_delivery_mode
                        .unwrap_or_else(|| before.email_delivery_mode.into());
                    let was_live = before.email_delivery_mode
                        == diesel_models::enums::EmailDeliveryModeEnum::Live;
                    // A workspace already live keeps it only while its environment is unchanged.
                    let kept_live = was_live && before.environment == environment.clone().into();
                    if mode == EmailDeliveryMode::Live
                        && !EmailDeliveryMode::can_go_live(&environment)
                        && !kept_live
                    {
                        if tenant.email_delivery_mode == Some(EmailDeliveryMode::Live) {
                            return Err(StoreError::InvalidArgument(
                                "Only a production workspace can deliver emails to customers"
                                    .to_string(),
                            )
                            .into());
                        }
                        // Leaving production: emails go to the team until it is production again.
                        tenant.email_delivery_mode = Some(EmailDeliveryMode::Test);
                    }
                    let before_trade_name = OrganizationRow::get_by_id(conn, organization_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .trade_name;

                    let trade_name_changed = tenant
                        .trade_name
                        .as_ref()
                        .is_some_and(|v| v != &before_trade_name);

                    if let Some(trade_name) = &tenant.trade_name {
                        OrganizationRow::update_trade_name(conn, organization_id, trade_name)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    let patch: TenantRowPatch = tenant.into();

                    let updated_tenant = patch
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let mut changed_fields: Vec<&'static str> = Vec::new();
                    if trade_name_changed {
                        changed_fields.push("trade_name");
                    }
                    if before.name != updated_tenant.name {
                        changed_fields.push("name");
                    }
                    if before.slug != updated_tenant.slug {
                        changed_fields.push("slug");
                    }
                    if before.environment != updated_tenant.environment {
                        changed_fields.push("environment");
                    }
                    if before.reporting_currency != updated_tenant.reporting_currency {
                        changed_fields.push("reporting_currency");
                    }
                    if before.email_delivery_mode != updated_tenant.email_delivery_mode {
                        changed_fields.push("email_delivery_mode");
                    }
                    if before.email_test_recipients != updated_tenant.email_test_recipients {
                        changed_fields.push("email_test_recipients");
                    }
                    if before.keep_last_events != updated_tenant.keep_last_events {
                        changed_fields.push("keep_last_events");
                    }

                    if !changed_fields.is_empty() {
                        let activity = Activity::new(
                            ActivityType::EntityUpdated,
                            EntityType::Tenant,
                            tenant_id.as_uuid(),
                        )
                        .with_metadata(serde_json::json!({
                            "changes": changed_fields
                                .iter()
                                .map(|f| serde_json::json!({ "field": f }))
                                .collect::<Vec<_>>(),
                        }));
                        self.internal
                            .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    Ok(updated_tenant.into())
                }
                .scope_boxed()
            })
            .await?;

        invalidate_reporting_currency_cache(&tenant_id).await;
        invalidate_tenant_cache(&tenant_id).await;
        self.keep_last_events.invalidate(&tenant_id);

        Ok(res)
    }

    async fn find_tenant_by_id_and_organization(
        &self,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_id_and_organization_id(&mut conn, tenant_id, organization_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_tenant_by_slug_and_organization_slug(
        &self,
        slug: String,
        organization_slug: String,
    ) -> StoreResult<Tenant> {
        let mut conn = self.get_conn().await?;

        TenantRow::find_by_slug_and_organization_slug(&mut conn, slug, organization_slug)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_tenants_by_organization_id(
        &self,
        organization_id: OrganizationId,
    ) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_by_organization_id(&mut conn, organization_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn list_sandbox_tenants(&self) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_by_environment(&mut conn, TenantEnvironmentEnum::Sandbox.into())
            .await
            .map_err(Into::into)
            .map(|rows| rows.into_iter().map(Into::into).collect())
    }

    async fn touch_tenant_activity(&self, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let now = chrono::Utc::now().naive_utc();
        let stale_before = now - chrono::Duration::hours(1);

        TenantRow::touch_activity(&mut conn, tenant_id, now, stale_before)
            .await
            .map_err(Into::into)
            .map(|_| ())
    }

    async fn count_subscriptions_pending_catchup(&self, tenant_id: TenantId) -> StoreResult<u64> {
        let mut conn = self.get_conn().await?;
        let today = chrono::Utc::now().naive_utc().date();

        diesel_models::subscriptions::SubscriptionRow::count_pending_billing_catchup(
            &mut conn, tenant_id, today,
        )
        .await
        .map_err(Into::into)
        .map(|n| n.max(0) as u64)
    }

    async fn list_lifecycle_managed_tenants(&self) -> StoreResult<Vec<Tenant>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_lifecycle_managed(&mut conn)
            .await
            .map_err(Into::into)
            .map(|rows| rows.into_iter().map(Into::into).collect())
    }

    async fn pause_tenant(&self, tenant_id: TenantId) -> StoreResult<()> {
        set_tenant_status(
            self,
            tenant_id,
            diesel_models::enums::TenantStatusEnum::Paused,
        )
        .await
    }

    async fn resume_tenant(&self, tenant_id: TenantId) -> StoreResult<()> {
        set_tenant_status(
            self,
            tenant_id,
            diesel_models::enums::TenantStatusEnum::Active,
        )
        .await
    }

    async fn archive_tenant(&self, tenant_id: TenantId) -> StoreResult<()> {
        set_tenant_status(
            self,
            tenant_id,
            diesel_models::enums::TenantStatusEnum::Archived,
        )
        .await
    }

    async fn schedule_tenant_deletion(
        &self,
        actor: UserId,
        organization_id: OrganizationId,
        tenant_id: TenantId,
        request_context: crate::domain::org_security_audit_log::SecurityRequestContext,
    ) -> StoreResult<bool> {
        // Admin authorization is enforced at the gRPC layer (require_admin).
        let mut conn = self.get_conn().await?;

        // Scope + existence check.
        let tenant = self
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await?;

        if tenant.environment == TenantEnvironmentEnum::Production {
            return Err(Report::new(StoreError::FailedPrecondition(
                "production tenants cannot be deleted".to_string(),
            )));
        }

        // Marking sets `archived_at`, so already-marked tenants drop out of this
        // active list — this is the last active tenant iff none other remains.
        let active = TenantRow::list_by_organization_id(&mut conn, organization_id).await?;
        let archives_org = !active.iter().any(|t| t.id != tenant_id);

        let now = chrono::Utc::now().naive_utc();
        drop(conn);
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                TenantRow::set_status(
                    conn,
                    tenant_id,
                    diesel_models::enums::TenantStatusEnum::PendingDeletion,
                    now,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                if archives_org {
                    OrganizationRow::set_archived_at(conn, organization_id, now)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        invalidate_tenant_cache(&tenant_id).await;

        let mut conn = self.get_conn().await?;
        let actor_email = UserRow::find_by_id(&mut conn, actor.as_uuid())
            .await
            .ok()
            .map(|u| u.email);
        self.record_security_event_best_effort(
            crate::domain::org_security_audit_log::RecordSecurityEvent {
                organization_id,
                event_type: crate::domain::org_security_audit_log::SecurityEventType::TenantDeleted,
                actor: crate::domain::org_security_audit_log::Actor::User { id: actor },
                actor_email,
                target_user_id: None,
                target_email: None,
                metadata: Some(serde_json::json!({
                    "tenant_id": tenant_id.to_string(),
                    "tenant_name": tenant.name,
                    "environment": format!("{:?}", tenant.environment),
                })),
                context: request_context,
            },
        )
        .await;

        log::info!("Tenant {tenant_id} scheduled for deletion by user {actor}");

        Ok(archives_org)
    }

    async fn list_tenants_pending_deletion(&self, limit: i64) -> StoreResult<Vec<TenantId>> {
        let mut conn = self.get_conn().await?;
        TenantRow::list_ids_pending_deletion(&mut conn, limit)
            .await
            .map_err(Into::into)
    }

    async fn hard_delete_tenants(&self, tenant_ids: &[TenantId]) -> StoreResult<()> {
        if tenant_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.get_conn().await?;
        // Chunked so one cascading delete can't dominate a single transaction.
        for chunk in tenant_ids.chunks(TENANT_HARD_DELETE_CHUNK) {
            TenantRow::delete_many(&mut conn, chunk)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            for id in chunk {
                invalidate_tenant_cache(id).await;
            }
        }
        Ok(())
    }

    async fn get_reporting_currency_by_tenant_id(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Currency> {
        get_reporting_currency_by_tenant_id_cached(self, tenant_id).await
    }

    async fn list_tenant_currencies_with_customer_count(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<(String, u64)>> {
        let mut conn = self.get_conn().await?;

        TenantRow::list_tenant_currencies_with_customer_count(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn list_tenant_currencies(&self, tenant_id: TenantId) -> StoreResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        TenantRow::list_tenant_currencies(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn update_tenant_available_currencies(
        &self,
        tenant_id: TenantId,
        currencies: Vec<String>,
    ) -> StoreResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        let res = TenantRow::update_available_currencies(&mut conn, tenant_id, currencies)
            .await
            .map_err(Into::into);

        invalidate_reporting_currency_cache(&tenant_id).await;
        invalidate_tenant_cache(&tenant_id).await;

        res
    }

    async fn find_tenant_by_id(&self, tenant_id: TenantId) -> StoreResult<TenantWithOrganization> {
        find_tenant_by_id_cached(&self.pool, tenant_id).await
    }

    async fn find_tenants_by_ids(
        &self,
        tenant_ids: &[TenantId],
    ) -> StoreResult<Vec<TenantWithOrganization>> {
        if tenant_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut resolved: Vec<TenantWithOrganization> = Vec::with_capacity(tenant_ids.len());
        let mut missing: Vec<TenantId> = Vec::new();

        {
            let mut cache = FIND_TENANT_BY_ID_CACHED.lock().await;
            for &tid in tenant_ids {
                match cache.cache_get(&tid) {
                    Some(hit) => resolved.push(hit.clone()),
                    None => missing.push(tid),
                }
            }
        }

        if missing.is_empty() {
            return Ok(resolved);
        }

        // Try a single batched query first; on failure, fall back to per-id concurrent
        // lookups so a transient batch error doesn't fail every caller's record.
        let batch_attempt: StoreResult<Vec<TenantWithOrganization>> = async {
            let mut conn = self.get_conn().await?;
            let rows = TenantRow::list_by_ids_with_org(&mut conn, &missing)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
        .await;

        let fetched: Vec<TenantWithOrganization> = match batch_attempt {
            Ok(list) => list,
            Err(e) => {
                log::warn!(
                    "Batched tenants lookup failed, falling back to per-id resolution: {e:?}"
                );
                let lookups = missing.iter().copied().map(|tid| async move {
                    (tid, find_tenant_by_id_cached(&self.pool, tid).await)
                });
                let results = futures::future::join_all(lookups).await;
                let mut list = Vec::with_capacity(results.len());
                for (tid, res) in results {
                    match res {
                        Ok(t) => list.push(t),
                        Err(err) => log::error!("Failed to resolve tenant {tid}: {err:?}"),
                    }
                }
                list
            }
        };

        {
            let mut cache = FIND_TENANT_BY_ID_CACHED.lock().await;
            for t in &fetched {
                cache.cache_set(t.tenant.id, t.clone());
            }
        }

        resolved.extend(fetched);
        Ok(resolved)
    }
}

impl StoreInternal {
    pub async fn insert_tenant_with_default_entities(
        &self,
        conn: &mut PgConn,
        tenant: TenantNew,
        organization_id: OrganizationId,
        trade_name: String,
        country: CountryCode,
        invoicing_entity: InvoicingEntityNew,
    ) -> StoreResult<Tenant> {
        let currency = self.get_currency_from_country(&country)?;

        // De-dup against every slug in the org, archived rows included: the unique
        // index `(organization_id, slug)` covers archived tenants.
        let existing_tenant_slugs =
            TenantRow::list_slugs_by_organization_id(conn, organization_id).await?;
        let slug = next_available_slug(tenant.environment.slug_base(), &existing_tenant_slugs);

        let email_delivery_mode = tenant
            .email_delivery_mode
            .unwrap_or_else(|| EmailDeliveryMode::default_for(&tenant.environment));
        if email_delivery_mode == EmailDeliveryMode::Live
            && !EmailDeliveryMode::can_go_live(&tenant.environment)
        {
            return Err(StoreError::InvalidArgument(
                "Only a production workspace can deliver emails to customers".to_string(),
            )
            .into());
        }
        let insertable_tenant: TenantRowNew = TenantRowNew {
            id: TenantId::new(),
            environment: tenant.environment.into(),
            reporting_currency: currency.clone(),
            name: tenant.name,
            slug,
            organization_id,
            available_currencies: vec![Some(currency)],
            disable_emails: email_delivery_mode != EmailDeliveryMode::Live,
            email_delivery_mode: email_delivery_mode.into(),
            email_test_recipients: vec![],
        };

        let inserted: Tenant = insertable_tenant
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        let _ = self
            .create_invoicing_entity(conn, invoicing_entity, inserted.id, country, trade_name)
            .await?;

        // TODO think about making it easier in the api (default with optional)
        let _ = self
            .insert_product_family(
                conn,
                domain::ProductFamilyNew {
                    name: "Default".to_string(),
                    tenant_id: inserted.id,
                },
            )
            .await?;

        Ok(inserted)
    }
}

#[cached(
    result = true,
    size = 100,
    time = 3600, // 1h
    key = "TenantId",
    convert = r#"{ tenant_id }"#
)]
async fn get_reporting_currency_by_tenant_id_cached(
    store: &Store,
    tenant_id: TenantId,
) -> StoreResult<Currency> {
    // Acquire the connection inside the cached body so a cache hit (the common case,
    // 1h TTL) never touches the pool — otherwise every stats/explore call checks out a
    // connection just to read a cached value, exhausting the pool under load.
    let mut conn = store.get_conn().await?;
    let currency = TenantRow::get_reporting_currency_by_id(&mut conn, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let res = Currencies::resolve_currency(&currency).ok_or_else(|| {
        StoreError::ValueNotFound(format!("Currency not found for code {currency}"))
    })?;

    Ok(res)
}

pub async fn invalidate_reporting_currency_cache(tenant_id: &TenantId) {
    GET_REPORTING_CURRENCY_BY_TENANT_ID_CACHED
        .lock()
        .await
        .cache_remove(tenant_id);
}

#[cached(
    result = true,
    size = 200,
    time = 300, // 5 min
    key = "TenantId",
    convert = r#"{ tenant_id }"#
)]
async fn find_tenant_by_id_cached(
    pool: &PgPool,
    tenant_id: TenantId,
) -> StoreResult<TenantWithOrganization> {
    let mut conn = pool_conn(pool).await?;
    TenantRow::find_by_id_with_org(&mut conn, tenant_id)
        .await
        .map_err(Into::into)
        .map(Into::into)
}

type SlugKey = (String, Option<String>);
type SlugCache = TimedSizedCache<SlugKey, (OrganizationId, Option<TenantId>)>;

/// Organization and tenant ids by their slugs, as the auth layer resolves request contexts.
/// Held by its store, never process-wide: stores sharing a process each read their own database.
#[derive(Clone)]
pub struct SlugIds {
    cache: Arc<Mutex<SlugCache>>,
}

impl SlugIds {
    pub(crate) fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(TimedSizedCache::with_size_and_lifespan(
                1_000,
                Duration::from_secs(86_400),
            ))),
        }
    }

    fn entries(
        &self,
    ) -> MutexGuard<'_, TimedSizedCache<SlugKey, (OrganizationId, Option<TenantId>)>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn get(
        &self,
        organization_slug: &str,
        tenant_slug: Option<&str>,
    ) -> Option<(OrganizationId, Option<TenantId>)> {
        let key = (organization_slug.to_owned(), tenant_slug.map(str::to_owned));
        self.entries().cache_get(&key).copied()
    }

    pub fn insert(
        &self,
        organization_slug: String,
        tenant_slug: Option<String>,
        ids: (OrganizationId, Option<TenantId>),
    ) {
        self.entries()
            .cache_set((organization_slug, tenant_slug), ids);
    }

    pub fn remove(&self, organization_slug: &str, tenant_slug: &str) {
        let key = (organization_slug.to_owned(), Some(tenant_slug.to_owned()));
        self.entries().cache_remove(&key);
    }
}

/// How long a read of `keep_last_events` may wait for its connection and load.
const KEEP_LAST_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// A bool per tenant: room for every tenant reading usage within two minutes on a large install.
const KEEP_LAST_CACHE_SIZE: usize = 10_000;

type Gates = Mutex<HashMap<TenantId, Arc<tokio::sync::Mutex<()>>>>;

/// A tenant's `keep_last_events`, cached for the 2 minutes ADR-0011 allows a change to take.
/// Held by its store, never process-wide: stores sharing a process each read their own database.
#[derive(Clone)]
pub struct KeepLastEventsLookup {
    pool: PgPool,
    cache: Arc<Mutex<TimedSizedCache<TenantId, bool>>>,
    loading: Arc<Gates>,
    timeout: Duration,
}

impl KeepLastEventsLookup {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self::with_timeout(pool, KEEP_LAST_LOOKUP_TIMEOUT)
    }

    fn with_timeout(pool: PgPool, timeout: Duration) -> Self {
        Self {
            pool,
            cache: Arc::new(Mutex::new(TimedSizedCache::with_size_and_lifespan(
                KEEP_LAST_CACHE_SIZE,
                Duration::from_secs(120),
            ))),
            loading: Arc::default(),
            timeout,
        }
    }

    fn entries(&self) -> MutexGuard<'_, TimedSizedCache<TenantId, bool>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cached(&self, tenant_id: TenantId) -> Option<bool> {
        self.entries().cache_get(&tenant_id).copied()
    }

    /// For a caller holding no connection: misses of one tenant share one read.
    pub async fn get(&self, tenant_id: TenantId) -> StoreResult<bool> {
        if let Some(keep_last) = self.cached(tenant_id) {
            return Ok(keep_last);
        }
        let pool = self.pool.clone();
        self.coalesced(tenant_id, || async move {
            let mut conn = pool_conn(&pool).await?;
            Ok(TenantRow::find_keep_last_events_by_id(&mut conn, tenant_id).await?)
        })
        .await
    }

    async fn coalesced<F, Fut>(&self, tenant_id: TenantId, load: F) -> StoreResult<bool>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = StoreResult<bool>>,
    {
        let read = async {
            let turn = InFlight::join(&self.loading, tenant_id);
            let _turn = turn.gate.lock().await;
            if let Some(keep_last) = self.cached(tenant_id) {
                return Ok(keep_last);
            }
            let keep_last = load().await?;
            self.entries().cache_set(tenant_id, keep_last);
            Ok(keep_last)
        };
        match tokio::time::timeout(self.timeout, read).await {
            Ok(read) => read,
            Err(_) => Err(
                Report::new(StoreError::DatabaseConnectionError).attach(format!(
                    "keep_last_events of tenant {tenant_id} not read within {:?}",
                    self.timeout
                )),
            ),
        }
    }

    /// For a caller already holding a connection: a miss reads on it, never waiting on the pool.
    pub async fn get_with_conn(&self, conn: &mut PgConn, tenant_id: TenantId) -> StoreResult<bool> {
        if let Some(keep_last) = self.cached(tenant_id) {
            return Ok(keep_last);
        }
        let keep_last = TenantRow::find_keep_last_events_by_id(conn, tenant_id).await?;
        self.entries().cache_set(tenant_id, keep_last);
        Ok(keep_last)
    }

    pub(crate) fn invalidate(&self, tenant_id: &TenantId) {
        self.entries().cache_remove(tenant_id);
    }
}

/// A read of one tenant's flag in progress; the last one out removes the tenant's gate.
struct InFlight<'a> {
    loading: &'a Gates,
    tenant_id: TenantId,
    gate: Arc<tokio::sync::Mutex<()>>,
}

impl<'a> InFlight<'a> {
    fn join(loading: &'a Gates, tenant_id: TenantId) -> Self {
        let gate = loading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(tenant_id)
            .or_default()
            .clone();
        Self {
            loading,
            tenant_id,
            gate,
        }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut loading = self.loading.lock().unwrap_or_else(PoisonError::into_inner);
        let last = loading
            .get(&self.tenant_id)
            .is_some_and(|gate| Arc::ptr_eq(gate, &self.gate) && Arc::strong_count(gate) == 2);
        if last {
            loading.remove(&self.tenant_id);
        }
    }
}

pub async fn invalidate_tenant_cache(tenant_id: &TenantId) {
    FIND_TENANT_BY_ID_CACHED
        .lock()
        .await
        .cache_remove(tenant_id);
}

async fn set_tenant_status(
    store: &Store,
    tenant_id: TenantId,
    status: diesel_models::enums::TenantStatusEnum,
) -> StoreResult<()> {
    let now = chrono::Utc::now().naive_utc();
    // The status decides whether the tenant's customers are evaluated at all.
    store
        .transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                TenantRow::set_status(conn, tenant_id, status, now).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
}

#[cfg(test)]
mod keep_last_events_tests {
    use super::*;
    use crate::store::{PgConfig, diesel_make_pg_pool};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn lookup(timeout: Duration) -> KeepLastEventsLookup {
        let pool = diesel_make_pg_pool(PgConfig::new(
            "postgres://unused@localhost:1/unused".to_owned(),
        ))
        .expect("a pool that never connects");
        KeepLastEventsLookup::with_timeout(pool, timeout)
    }

    fn gates(lookup: &KeepLastEventsLookup) -> usize {
        lookup.loading.lock().unwrap().len()
    }

    #[tokio::test]
    async fn a_stuck_tenant_does_not_hold_up_another() {
        let lookup = lookup(Duration::from_secs(30));
        let (stuck, other) = (TenantId::new(), TenantId::new());
        let (entered, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let waiting = {
            let (lookup, entered, release) = (lookup.clone(), entered.clone(), release.clone());
            tokio::spawn(async move {
                lookup
                    .coalesced(stuck, || async move {
                        entered.notify_one();
                        release.notified().await;
                        Ok(true)
                    })
                    .await
            })
        };
        entered.notified().await;

        let read = tokio::time::timeout(
            Duration::from_secs(1),
            lookup.coalesced(other, || async { Ok(false) }),
        )
        .await
        .expect("the other tenant is read meanwhile");
        assert!(!read.unwrap());

        release.notify_one();
        assert!(waiting.await.unwrap().unwrap());
        assert_eq!(gates(&lookup), 0);
    }

    #[tokio::test]
    async fn concurrent_misses_of_one_tenant_share_one_read() {
        let lookup = lookup(Duration::from_secs(30));
        let tenant_id = TenantId::new();
        let reads = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let misses = (0..4)
            .map(|_| {
                let (lookup, reads, release) = (lookup.clone(), reads.clone(), release.clone());
                tokio::spawn(async move {
                    lookup
                        .coalesced(tenant_id, || async move {
                            reads.fetch_add(1, Ordering::SeqCst);
                            release.notified().await;
                            Ok(true)
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();
        while reads.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        release.notify_one();
        for miss in misses {
            assert!(miss.await.unwrap().unwrap());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(gates(&lookup), 0);
    }

    #[tokio::test]
    async fn a_stuck_load_times_out() {
        let lookup = lookup(Duration::from_millis(50));
        let tenant_id = TenantId::new();
        let read = lookup
            .coalesced(tenant_id, std::future::pending::<StoreResult<bool>>)
            .await;
        assert!(read.is_err());
        assert_eq!(gates(&lookup), 0, "a timed-out read leaves no gate behind");
        assert!(lookup.cached(tenant_id).is_none());
    }
}

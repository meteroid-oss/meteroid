use crate::StoreResult;
use crate::domain::custom_property_definitions::CustomPropertyMode;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::CustomPropertyEntityType;
use crate::domain::filters::Filter;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::pgmq::{
    HubspotSyncCustomerDomain, HubspotSyncRequestEvent, PennylaneSyncCustomer,
    PennylaneSyncRequestEvent, PgmqMessageNew, PgmqQueue, VatValidationRequestEvent,
};
use crate::domain::{
    ConnectorProviderEnum, Customer, CustomerBatchResult, CustomerBrief, CustomerNew,
    CustomerNewWrapper, CustomerPatch, CustomerUpdate, CustomerWithMetrics, PaginatedVec,
    PaginationRequest, VatNumberValidationStatus,
};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::filter_resolution::{ListEntity, resolve_list_filters};
use crate::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use crate::repositories::organizations::ensure_org_member;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::orchestration::payment_transaction_failed::rearm_hard_declined_retries;
use crate::store::{PgConn, Store, StoreInternal};
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId, UserId};
use common_domain::lifecycle::{LifecycleSignals, derive_lifecycle_state};
use common_eventbus::Event;
use diesel_models::customers::{
    CustomerLifecycleSignalRow, CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate,
};
use diesel_models::lock::CustomerLock;
use diesel_models::query::credit_inputs;
use diesel_models::query::customers::connected_account_platform_org;
use diesel_models::subscriptions::SubscriptionRow;
use diesel_models::tenants::TenantRow;
use error_stack::{Report, bail};
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

/// Map the Postgres signal row onto the shared `common_domain::lifecycle` signals so
/// the badge derives from the same rule as the analytics projection.
fn signals_from_row(r: CustomerLifecycleSignalRow) -> LifecycleSignals {
    LifecycleSignals {
        has_active_paid_sub: r.has_active_paid_sub,
        is_trialing: r.is_trialing,
        has_active_free_sub: r.has_active_free_sub,
        has_active_sub: r.has_active_sub,
        has_suspended_sub: r.has_suspended_sub,
        has_paused_sub: r.has_paused_sub,
        had_paid_sub_ended: r.had_paid_sub_ended,
        ever_paid: r.ever_paid,
        trial_expired: r.trial_expired,
        has_closed_lost_quote: r.has_closed_lost_quote,
        has_open_quote: r.has_open_quote,
        reactivated: r.reactivated,
    }
}

fn validate_customer_currency(
    currency: &str,
    available_currencies: &[Option<String>],
) -> StoreResult<()> {
    if !available_currencies
        .iter()
        .any(|c| c.as_deref() == Some(currency))
    {
        return Err(StoreError::InvalidArgument(format!(
            "Currency '{}' is not available for this tenant",
            currency
        ))
        .into());
    }
    Ok(())
}

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait CustomersInterface {
    #[delegated]
    async fn find_customer_by_id(
        &self,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

    async fn find_customer_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

    async fn find_customer_id_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<CustomerBrief>;

    async fn find_customer_ids_by_aliases(
        &self,
        tenant_id: TenantId,
        aliases: Vec<String>,
    ) -> StoreResult<Vec<CustomerBrief>>;

    async fn list_customers(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
    ) -> StoreResult<PaginatedVec<Customer>>;

    async fn list_customers_with_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
        lifecycle_states: Vec<String>,
        countries: Vec<String>,
        filters: Vec<Filter>,
    ) -> StoreResult<PaginatedVec<CustomerWithMetrics>>;

    /// Distinct country codes in use by the tenant's non-archived customers.
    async fn list_customer_countries(&self, tenant_id: TenantId) -> StoreResult<Vec<String>>;

    async fn list_customers_by_ids_global(
        &self,
        ids: Vec<CustomerId>,
    ) -> StoreResult<Vec<Customer>>;

    async fn list_customers_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<CustomerId>,
    ) -> StoreResult<Vec<Customer>>;

    async fn insert_customer(
        &self,
        actor: Actor,
        customer: CustomerNew,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

    async fn insert_customer_batch(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>>;

    async fn upsert_customer_batch(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>>;

    /// Like `upsert_customer_batch` but does not fail the entire batch on per-row
    /// validation errors. Instead, invalid rows are collected as failures and valid
    /// rows are upserted. Used by CSV import where partial success is expected.
    async fn upsert_customer_batch_lenient(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<CustomerBatchResult>;

    async fn patch_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>>;

    /// Sets (or clears, with `None`) the owner; the owner must be an org member.
    async fn assign_customer_owner(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
        owner_user_id: Option<UserId>,
    ) -> StoreResult<usize>;

    async fn find_customer_by_id_or_alias(
        &self,
        id_or_alias: AliasOr<CustomerId>,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

    /// Live lifecycle state (snake_case repr) derived from Postgres signals via
    /// the shared `common_domain::lifecycle` rule. Returns `unknown` if the customer
    /// has no signal-bearing rows.
    async fn derive_customer_lifecycle_state(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<String>;

    async fn update_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer: CustomerUpdate,
    ) -> StoreResult<Customer>;

    async fn archive_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()>;

    async fn unarchive_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()>;

    async fn patch_customer_conn_meta(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()>;

    async fn sync_customers_to_hubspot(
        &self,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn sync_customers_to_pennylane(
        &self,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn update_vat_number_validation(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        expected_vat_number: &str,
        status: VatNumberValidationStatus,
        checked_at: chrono::NaiveDateTime,
        vies_check: Option<meteroid_tax::ViesCheckData>,
    ) -> StoreResult<()>;

    /// Manual re-check: moves the customer's VAT validation back to `PENDING`
    /// and enqueues an immediate VIES verification, in one transaction. Fails
    /// if the customer has no format-valid, VIES-eligible VAT number.
    async fn request_vat_number_revalidation(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Customer>;

    /// Cross-tenant list of customers due for a best-effort periodic VIES
    /// re-check (never checked, or last checked before `checked_before`).
    async fn list_vat_revalidation_candidates(
        &self,
        checked_before: chrono::NaiveDateTime,
        created_before: chrono::NaiveDateTime,
        limit: i64,
    ) -> StoreResult<Vec<Customer>>;
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn find_customer_by_id_with_conn(
        &self,
        conn: &mut PgConn,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
        CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn find_customer_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_alias(&mut conn, alias, tenant_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn find_customer_id_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<CustomerBrief> {
        let mut conn = self.get_conn().await?;

        CustomerRow::resolve_id_by_alias(&mut conn, tenant_id, alias)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn find_customer_ids_by_aliases(
        &self,
        tenant_id: TenantId,
        aliases: Vec<String>,
    ) -> StoreResult<Vec<CustomerBrief>> {
        let mut conn = self.get_conn().await?;

        CustomerRow::resolve_ids_by_aliases(&mut conn, tenant_id, aliases)
            .await
            .map_err(Into::into)
            .map(|v| {
                v.into_iter()
                    .map(Into::into)
                    .collect::<Vec<CustomerBrief>>()
            })
    }

    async fn list_customers(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
    ) -> StoreResult<PaginatedVec<Customer>> {
        let mut conn = self.get_conn().await?;

        let rows = CustomerRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            order_by.as_deref(),
            query,
            archived,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Customer> = PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Vec<Result<Customer, Report<StoreError>>>>()
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn list_customers_with_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: Option<String>,
        query: Option<String>,
        archived: Option<bool>,
        lifecycle_states: Vec<String>,
        countries: Vec<String>,
        filters: Vec<Filter>,
    ) -> StoreResult<PaginatedVec<CustomerWithMetrics>> {
        let mut conn = self.get_conn().await?;

        let param_ids =
            resolve_customer_filter_ids(&self.internal, &mut conn, tenant_id, filters).await?;

        let rows = CustomerRow::list_with_metrics(
            &mut conn,
            tenant_id,
            pagination.into(),
            order_by.as_deref(),
            query,
            archived,
            lifecycle_states,
            countries,
            param_ids,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<StoreError>>>()?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_customer_countries(&self, tenant_id: TenantId) -> StoreResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        CustomerRow::list_distinct_countries(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn list_customers_by_ids_global(
        &self,
        ids: Vec<CustomerId>,
    ) -> StoreResult<Vec<Customer>> {
        let mut conn = self.get_conn().await?;

        CustomerRow::list_by_ids_global(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Vec<Result<Customer, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_customers_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<CustomerId>,
    ) -> StoreResult<Vec<Customer>> {
        let mut conn = self.get_conn().await?;

        CustomerRow::list_by_ids(&mut conn, &tenant_id, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Vec<Result<Customer, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
    }

    async fn insert_customer(
        &self,
        actor: Actor,
        customer: CustomerNew,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        validate_customer_currency(&customer.currency, &tenant.available_currencies)?;
        customer.validate_b2c()?;

        if let Some(ref ca_id) = customer.connected_account_id {
            let ca_org_id = connected_account_platform_org(&mut conn, *ca_id)
                .await
                .ok()
                .flatten()
                .ok_or_else(|| {
                    Report::new(StoreError::ValueNotFound(
                        "Connected account not found".into(),
                    ))
                })?;
            if ca_org_id != tenant.organization_id {
                return Err(Report::new(StoreError::InvalidArgument(
                    "Connected account does not belong to this organization".into(),
                )));
            }
        }

        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, customer.invoicing_entity_id)
            .await?;

        let vat_number_format_valid = customer.is_valid_vat_number_format();

        let customer: CustomerRowNew = CustomerNewWrapper {
            inner: customer,
            invoicing_entity_id: invoicing_entity.id,
            tenant_id,
            vat_number_format_valid,
        }
        .try_into()?;

        let res: Customer = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let mut customer = customer;
                    customer.custom_properties = self
                        .internal
                        .validate_and_normalize_custom_properties(
                            conn,
                            tenant_id,
                            CustomPropertyEntityType::Customer,
                            &customer.custom_properties,
                            &serde_json::json!({}),
                            CustomPropertyMode::Replace,
                        )
                        .await?;
                    let new_customer: Customer = customer.insert(conn).await?.try_into()?;
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::customer_created(new_customer.clone().into())],
                        )
                        .await?;
                    Ok(new_customer)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::customer_created(actor, res.id, res.tenant_id))
            .await;

        Ok(res)
    }

    async fn insert_customer_batch(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let prepared_batch = self.prepare_customer_batch(batch, tenant_id).await?;

        let res: Vec<Customer> = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let res: Vec<Customer> =
                        CustomerRow::insert_customer_batch(conn, prepared_batch)
                            .await
                            .map_err(Into::into)
                            .and_then(|v| v.into_iter().map(TryInto::try_into).collect())?;

                    let outbox_events: Vec<OutboxEvent> = res
                        .iter()
                        .map(|x| OutboxEvent::customer_created(x.clone().into()))
                        .collect();

                    self.internal
                        .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        self.publish_customer_created_events(&actor, &res).await;
        Ok(res)
    }

    async fn upsert_customer_batch(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let prepared_batch = self.prepare_customer_batch(batch, tenant_id).await?;

        let res: Vec<Customer> = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let res: Vec<Customer> =
                        CustomerRow::upsert_customer_batch(conn, prepared_batch)
                            .await
                            .map_err(Into::into)
                            .and_then(|v| v.into_iter().map(TryInto::try_into).collect())?;
                    bump_written_customers(conn, &*self.hooks.credit, tenant_id, &res).await?;

                    let outbox_events: Vec<OutboxEvent> = res
                        .iter()
                        .map(|x| OutboxEvent::customer_created(x.clone().into()))
                        .collect();

                    self.internal
                        .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        self.publish_customer_created_events(&actor, &res).await;
        Ok(res)
    }

    async fn upsert_customer_batch_lenient(
        &self,
        actor: Actor,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<CustomerBatchResult> {
        let (prepared, failures) = self
            .prepare_customer_batch_lenient(batch, tenant_id)
            .await?;

        if prepared.is_empty() {
            return Ok(CustomerBatchResult {
                created: vec![],
                failures,
            });
        }

        let res: Vec<Customer> = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let res: Vec<Customer> = CustomerRow::upsert_customer_batch(conn, prepared)
                        .await
                        .map_err(Into::into)
                        .and_then(|v| v.into_iter().map(TryInto::try_into).collect())?;
                    bump_written_customers(conn, &*self.hooks.credit, tenant_id, &res).await?;

                    let outbox_events: Vec<OutboxEvent> = res
                        .iter()
                        .map(|x| OutboxEvent::customer_created(x.clone().into()))
                        .collect();

                    self.internal
                        .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        self.publish_customer_created_events(&actor, &res).await;
        Ok(CustomerBatchResult {
            created: res,
            failures,
        })
    }

    async fn patch_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        mut customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>> {
        // Validate currency if provided
        if let Some(ref currency) = customer.currency {
            let mut conn = self.get_conn().await?;
            let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            validate_customer_currency(currency, &tenant.available_currencies)?;
        }

        // Enforce the B2C rule against the merged result — a patch can flip the type
        // while leaving names/country untouched — then re-derive an Individual's name.
        // Absent customer (Err) falls through to the update below, which returns None.
        {
            let mut conn = self.get_conn().await?;
            if let Ok(current) = CustomerRow::find_by_id(&mut conn, &customer.id, &tenant_id).await
            {
                let current: Customer = current.try_into()?;
                crate::domain::customers::validate_patch_b2c(
                    &customer,
                    current.customer_type,
                    &current.name,
                    current.first_name.as_deref(),
                    current.last_name.as_deref(),
                    current
                        .billing_address
                        .as_ref()
                        .and_then(|a| a.country.as_ref()),
                )?;
                crate::domain::customers::derive_patch_name(
                    &mut customer,
                    current.customer_type,
                    current.first_name.as_deref(),
                    current.last_name.as_deref(),
                );
            }
        }

        let currency_patched = customer.currency.is_some();
        let is_valid_vat_number_format = customer.is_valid_vat_number_format();
        // A patch that touches vat_number resets external validation for the new value.
        let vat_number_touched = customer.vat_number.clone();
        let customer_id = customer.id;
        let provided_props = customer.custom_properties.clone();
        let new_payment_method = customer.current_payment_method_id.flatten();
        let mut patch_model: CustomerRowPatch = customer.try_into()?;
        patch_model.vat_number_format_valid = is_valid_vat_number_format;
        if let Some(new_vat) = vat_number_touched {
            let status = crate::domain::customers::initial_vies_status(
                new_vat.as_deref(),
                is_valid_vat_number_format.unwrap_or(false),
            );
            patch_model.vat_number_validation_status = Some(status);
            patch_model.vat_number_checked_at = Some(None);
            patch_model.vat_number_vies_check = Some(None);
        }

        let updated = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let Some(lock) = CustomerRow::lock_many(conn, tenant_id, &[customer_id])
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .pop()
                    else {
                        return Ok(None);
                    };
                    let mut patch_model = patch_model;
                    if let Some(provided) = &provided_props {
                        let existing = CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        patch_model.custom_properties = Some(
                            self.internal
                                .validate_and_normalize_custom_properties(
                                    conn,
                                    tenant_id,
                                    CustomPropertyEntityType::Customer,
                                    provided,
                                    &existing.custom_properties,
                                    CustomPropertyMode::Merge,
                                )
                                .await?,
                        );
                    }
                    let entity_before = if patch_model.invoicing_entity_id.is_some() {
                        let before = CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        Some((
                            before.invoicing_entity_id,
                            before.payment_reminder_sequence_id,
                        ))
                    } else {
                        None
                    };
                    let updated: Option<CustomerRow> = patch_model
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    match updated {
                        None => Ok(None),
                        Some(mut updated) => {
                            // A reminder override belongs to the old entity's sequences.
                            if let Some((entity_before, Some(sequence_id))) = entity_before
                                && updated.invoicing_entity_id != entity_before
                            {
                                CustomerRow::set_payment_reminder_preference(
                                    conn,
                                    tenant_id,
                                    customer_id,
                                    updated.payment_reminders_opt_out,
                                    None,
                                )
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                                updated.payment_reminder_sequence_id = None;
                                let activity = Activity::new(
                                    ActivityType::CustomerUpdated,
                                    EntityType::Customer,
                                    customer_id.as_uuid(),
                                )
                                .with_metadata(
                                    serde_json::json!({ "changes": [{
                                    "field": "payment_reminder_sequence_id",
                                    "from": sequence_id.as_base62(),
                                    "to": null,
                                }] }),
                                );
                                self.internal
                                    .record_audit_tx(
                                        conn,
                                        tenant_id,
                                        actor,
                                        AuditInput::Activity(activity),
                                    )
                                    .await?;
                            }
                            if let Some(payment_method_id) = new_payment_method {
                                rearm_hard_declined_retries(
                                    self,
                                    conn,
                                    tenant_id,
                                    customer_id,
                                    payment_method_id,
                                )
                                .await?;
                            }
                            if currency_patched {
                                self.hooks
                                    .credit
                                    .wallet_price_changed(
                                        conn,
                                        &[&lock],
                                        common_domain::clock::now(),
                                    )
                                    .await?;
                            }
                            credit_inputs::bump_customer(conn, &lock).await?;
                            let updated: Customer = updated.try_into()?;
                            let outbox_events =
                                vec![OutboxEvent::customer_updated(updated.clone().into())];
                            self.internal
                                .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                                .await?;
                            Ok(Some(updated))
                        }
                    }
                }
                .scope_boxed()
            })
            .await?;

        match updated {
            None => Ok(None),
            Some(updated) => {
                let _ = self
                    .eventbus
                    .publish(Event::customer_patched(actor, updated.id, tenant_id))
                    .await;

                Ok(Some(updated))
            }
        }
    }

    async fn assign_customer_owner(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
        owner_user_id: Option<UserId>,
    ) -> StoreResult<usize> {
        if customer_ids.is_empty() {
            return Ok(0);
        }

        let updated = self
            .transaction(|conn| {
                let actor = &actor;
                let customer_ids = customer_ids.clone();
                async move {
                    if let Some(owner_id) = owner_user_id {
                        ensure_org_member(conn, tenant_id, owner_id).await?;
                    }

                    let n = CustomerRow::assign_owner(
                        conn,
                        tenant_id,
                        customer_ids.clone(),
                        owner_user_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // A bare owner update bumps `store_committed_at` (trigger), so analytics
                    // re-projects the owner attribute on its own. The outbox events keep
                    // connector reconciliation consistent with the fresh rows.
                    let rows = CustomerRow::list_by_ids(conn, &tenant_id, customer_ids)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let outbox_events = rows
                        .into_iter()
                        .map(|r| {
                            let c: Customer = r.try_into()?;
                            Ok(OutboxEvent::customer_updated(c.into()))
                        })
                        .collect::<Result<Vec<_>, Report<StoreError>>>()?;
                    self.internal
                        .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                        .await?;

                    Ok::<usize, Report<StoreError>>(n)
                }
                .scope_boxed()
            })
            .await?;

        Ok(updated)
    }

    async fn find_customer_by_id_or_alias(
        &self,
        id_or_alias: AliasOr<CustomerId>,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, id_or_alias)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn derive_customer_lifecycle_state(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<String> {
        let mut conn = self.get_conn().await?;

        let row =
            CustomerLifecycleSignalRow::fetch_for_customer(&mut conn, *customer_id, *tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let signals = row.map(signals_from_row).unwrap_or_default();

        Ok(derive_lifecycle_state(&signals).as_str().to_string())
    }

    async fn update_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer: CustomerUpdate,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        // Validate currency
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        validate_customer_currency(&customer.currency, &tenant.available_currencies)?;
        customer.validate_b2c()?;

        let by_id_or_alias =
            CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, customer.id_or_alias.clone())
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let vat_number_format_valid = customer.is_valid_vat_number_format();

        // Reset external validation only when the number actually changed; otherwise
        // preserve the prior result so a definitive INVALID/VALID isn't re-checked.
        let (vat_number_validation_status, vat_number_checked_at, vat_number_vies_check) =
            if by_id_or_alias.vat_number != customer.vat_number {
                (
                    crate::domain::customers::initial_vies_status(
                        customer.vat_number.as_deref(),
                        vat_number_format_valid,
                    ),
                    None,
                    None,
                )
            } else {
                (
                    by_id_or_alias.vat_number_validation_status,
                    by_id_or_alias.vat_number_checked_at,
                    by_id_or_alias.vat_number_vies_check.clone(),
                )
            };

        let custom_properties = self
            .internal
            .validate_and_normalize_custom_properties(
                &mut conn,
                tenant_id,
                CustomPropertyEntityType::Customer,
                &customer.custom_properties,
                &by_id_or_alias.custom_properties,
                CustomPropertyMode::Replace,
            )
            .await?;

        let update_model = CustomerRowUpdate {
            id: by_id_or_alias.id,
            name: crate::domain::customers::resolve_name(
                customer.customer_type,
                customer.name,
                customer.first_name.as_deref(),
                customer.last_name.as_deref(),
            ),
            alias: customer.alias,
            billing_email: customer.billing_email,
            invoicing_emails: customer.invoicing_emails.into_iter().map(Some).collect(),
            legal_number: customer.legal_number,
            buyer_reference: customer.buyer_reference,
            phone: customer.phone,
            currency: customer.currency,
            billing_address: customer
                .billing_address
                .map(TryInto::try_into)
                .transpose()?,
            shipping_address: customer
                .shipping_address
                .map(TryInto::try_into)
                .transpose()?,
            invoicing_entity_id: invoicing_entity.id,
            vat_number: customer.vat_number,
            custom_taxes: serde_json::to_value(&customer.custom_taxes).map_err(|e| {
                StoreError::SerdeError("Failed to serialize custom_taxes".to_string(), e)
            })?,
            vat_number_format_valid,
            tax_status: customer.tax_status.into(),
            exemption_reason: customer.exemption_reason,
            vat_number_validation_status,
            vat_number_checked_at,
            vat_number_vies_check,
            custom_properties,
            preferred_locales: crate::domain::customers::locales_to_row(customer.preferred_locales),
            customer_type: customer.customer_type.into(),
            first_name: crate::utils::text::blank_to_none(customer.first_name),
            last_name: crate::utils::text::blank_to_none(customer.last_name),
        };

        let previous_currency = by_id_or_alias.currency.clone();
        let updated = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = CustomerRow::lock_many(conn, tenant_id, &[update_model.id])
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .pop()
                        .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;
                    let updated = update_model
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;
                    if updated.currency != previous_currency {
                        self.hooks
                            .credit
                            .wallet_price_changed(conn, &[&lock], common_domain::clock::now())
                            .await?;
                    }
                    credit_inputs::bump_customer(conn, &lock).await?;

                    let updated: Customer = updated.try_into()?;

                    let outbox_events = vec![OutboxEvent::customer_updated(updated.clone().into())];
                    self.internal
                        .record_outbox_batch_tx(conn, tenant_id, actor, outbox_events)
                        .await?;

                    Ok(updated)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::customer_updated(actor, updated.id, tenant_id))
            .await;

        Ok(updated)
    }

    async fn archive_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()> {
        use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
        use diesel_models::enums::SubscriptionStatusEnum as DieselSubscriptionStatusEnum;

        let mut conn = self.get_conn().await?;

        let customer = CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, id_or_alias)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Check for blocking subscriptions (active, trial, etc.). One pending activation would be
        // activated and billed later, for a customer the live credit recompute no longer follows.
        let blocking_statuses = vec![
            DieselSubscriptionStatusEnum::PendingActivation,
            DieselSubscriptionStatusEnum::Active,
            DieselSubscriptionStatusEnum::TrialActive,
            DieselSubscriptionStatusEnum::TrialExpired,
            DieselSubscriptionStatusEnum::PendingCharge,
            DieselSubscriptionStatusEnum::Paused,
            DieselSubscriptionStatusEnum::Suspended,
        ];

        let blocking_subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
            Some(customer.id),
            None,
            Some(blocking_statuses),
            None,
            vec![],
            Some("id.desc"),
            PaginationRequest {
                per_page: Some(1), // We only need to know if any exist
                page: 0,
            }
            .into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        if blocking_subscriptions.total_results > 0 {
            return Err(StoreError::InvalidArgument(
                "Cannot archive customer with active subscriptions. Cancel all active subscriptions before archiving.".to_string(),
            )
            .into());
        }

        let customer_id = customer.id;
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let lock = CustomerRow::lock(conn, tenant_id, customer_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                CustomerRow::archive(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                credit_inputs::bump_customer(conn, &lock).await?;

                let activity = Activity::new(
                    ActivityType::CustomerArchived,
                    EntityType::Customer,
                    customer_id.as_uuid(),
                );
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn unarchive_customer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()> {
        use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};

        let mut conn = self.get_conn().await?;
        let customer = CustomerRow::find_by_id_or_alias_including_archived(
            &mut conn,
            tenant_id,
            id_or_alias.clone(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let customer_id = customer.id;
        drop(conn);

        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let lock = CustomerRow::lock(conn, tenant_id, customer_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                CustomerRow::unarchive(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                credit_inputs::bump_customer(conn, &lock).await?;

                let activity = Activity::new(
                    ActivityType::CustomerUnarchived,
                    EntityType::Customer,
                    customer_id.as_uuid(),
                );
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    async fn patch_customer_conn_meta(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        // Update the JSON metadata field (legacy)
        CustomerRowPatch::upsert_conn_meta(
            &mut conn,
            provider.into(),
            customer_id,
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        use common_domain::ids::BaseId;
        let connection_row = diesel_models::customer_connection::CustomerConnectionRow {
            id: common_domain::ids::CustomerConnectionId::new(),
            customer_id,
            connector_id,
            supported_payment_types: None,
            external_customer_id: external_id.to_string(),
        };

        diesel_models::customer_connection::CustomerConnectionRow::upsert(
            &mut conn,
            &tenant_id,
            connection_row,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    async fn sync_customers_to_hubspot(
        &self,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let connector = self.get_hubspot_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Hubspot connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let customers = CustomerRow::find_by_ids_or_aliases(&mut conn, tenant_id, ids_or_aliases)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::HubspotSync,
            customers
                .into_iter()
                .map(|customer| {
                    HubspotSyncRequestEvent::CustomerDomain(Box::new(HubspotSyncCustomerDomain {
                        id: customer.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn sync_customers_to_pennylane(
        &self,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Pennylane connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let customers = CustomerRow::find_by_ids_or_aliases(&mut conn, tenant_id, ids_or_aliases)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::PennylaneSync,
            customers
                .into_iter()
                .map(|customer| {
                    PennylaneSyncRequestEvent::Customer(Box::new(PennylaneSyncCustomer {
                        id: customer.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn update_vat_number_validation(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        expected_vat_number: &str,
        status: VatNumberValidationStatus,
        checked_at: chrono::NaiveDateTime,
        vies_check: Option<meteroid_tax::ViesCheckData>,
    ) -> StoreResult<()> {
        let vies_check = vies_check.and_then(|c| serde_json::to_value(c).ok());
        let status = status.into();
        self.transaction(|conn| {
            async move {
                let Some(lock) = CustomerRow::lock_many(conn, tenant_id, &[customer_id])
                    .await?
                    .pop()
                else {
                    return Ok(());
                };
                CustomerRowPatch::update_vat_validation(
                    conn,
                    tenant_id,
                    customer_id,
                    expected_vat_number,
                    status,
                    checked_at,
                    vies_check,
                )
                .await?;
                credit_inputs::bump_customer(conn, &lock).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn request_vat_number_revalidation(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Customer> {
        self.transaction(|conn| {
            async move {
                let lock = CustomerRow::lock(conn, tenant_id, customer_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let row = CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let vat_number = match row.vat_number.as_deref() {
                    Some(vat)
                        if row.vat_number_format_valid
                            && meteroid_tax::vies::is_vies_eligible(vat) =>
                    {
                        vat.to_string()
                    }
                    _ => bail!(StoreError::InvalidArgument(
                        "Customer has no VIES-verifiable VAT number".to_string()
                    )),
                };

                CustomerRowPatch::mark_vat_validation_pending(
                    conn,
                    tenant_id,
                    customer_id,
                    &vat_number,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                credit_inputs::bump_customer(conn, &lock).await?;

                let message: PgmqMessageNew = VatValidationRequestEvent {
                    tenant_id,
                    customer_id,
                    vat_number,
                    attempt: 0,
                    revalidate: false,
                }
                .try_into()?;
                self.pgmq_send_batch_tx(conn, PgmqQueue::VatValidation, vec![message])
                    .await?;

                CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
                    .await
                    .map_err(Into::into)
                    .and_then(TryInto::try_into)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_vat_revalidation_candidates(
        &self,
        checked_before: chrono::NaiveDateTime,
        created_before: chrono::NaiveDateTime,
        limit: i64,
    ) -> StoreResult<Vec<Customer>> {
        let mut conn = self.get_conn().await?;
        CustomerRow::list_vat_revalidation_candidates(
            &mut conn,
            checked_before,
            created_before,
            limit,
        )
        .await
        .map_err(Into::into)
        .and_then(|rows| rows.into_iter().map(TryInto::try_into).collect())
    }
}

impl Store {
    async fn prepare_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CustomerRowNew>> {
        let mut conn = self.get_conn().await?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Validate all currencies + B2C rules upfront
        for customer in &batch {
            validate_customer_currency(&customer.currency, &tenant.available_currencies)?;
            customer.validate_b2c()?;
        }

        let invoicing_entities = self.list_invoicing_entities(tenant_id).await?;
        let default_invoicing_entity =
            invoicing_entities
                .iter()
                .find(|ie| ie.is_default)
                .ok_or(StoreError::ValueNotFound(
                    "Default invoicing entity not found".to_string(),
                ))?;

        batch
            .into_iter()
            .map(|c| {
                let invoicing_entity = c
                    .invoicing_entity_id
                    .as_ref()
                    .and_then(|id| invoicing_entities.iter().find(|ie| ie.id == *id))
                    .unwrap_or(default_invoicing_entity);

                let vat_number_format_valid = c.is_valid_vat_number_format();

                CustomerNewWrapper {
                    inner: c,
                    invoicing_entity_id: invoicing_entity.id,
                    tenant_id,
                    vat_number_format_valid,
                }
                .try_into()
            })
            .collect::<Vec<Result<CustomerRowNew, Report<StoreError>>>>()
            .into_iter()
            .collect()
    }

    /// Like `prepare_customer_batch` but collects per-row validation errors instead
    /// of failing the entire batch. Returns (valid_rows, failures) where failures
    /// are (original_index, error_message).
    async fn prepare_customer_batch_lenient(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<(Vec<CustomerRowNew>, Vec<(usize, String)>)> {
        let mut conn = self.get_conn().await?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entities = self.list_invoicing_entities(tenant_id).await?;
        let default_invoicing_entity =
            invoicing_entities
                .iter()
                .find(|ie| ie.is_default)
                .ok_or(StoreError::ValueNotFound(
                    "Default invoicing entity not found".to_string(),
                ))?;

        let mut valid = Vec::with_capacity(batch.len());
        let mut failures = Vec::new();

        for (idx, c) in batch.into_iter().enumerate() {
            if let Err(e) = validate_customer_currency(&c.currency, &tenant.available_currencies) {
                failures.push((idx, e.current_context().to_string()));
                continue;
            }

            if let Err(e) = c.validate_b2c() {
                failures.push((idx, e.to_string()));
                continue;
            }

            let invoicing_entity = c
                .invoicing_entity_id
                .as_ref()
                .and_then(|id| invoicing_entities.iter().find(|ie| ie.id == *id))
                .unwrap_or(default_invoicing_entity);

            let vat_number_format_valid = c.is_valid_vat_number_format();

            let row: Result<CustomerRowNew, Report<StoreError>> = (CustomerNewWrapper {
                inner: c,
                invoicing_entity_id: invoicing_entity.id,
                tenant_id,
                vat_number_format_valid,
            })
            .try_into();

            match row {
                Ok(mut row) => {
                    // Batch rows carry the raw provided bag; validate it per row so one bad value
                    // fails its own row instead of the batch. `Replace`, like a full update: the
                    // file owns the properties it declares. A null bag declares none and is left
                    // untouched by the upsert.
                    if !row.custom_properties.is_null() {
                        match self
                            .internal
                            .validate_and_normalize_custom_properties(
                                &mut conn,
                                tenant_id,
                                CustomPropertyEntityType::Customer,
                                &row.custom_properties,
                                &serde_json::json!({}),
                                CustomPropertyMode::Replace,
                            )
                            .await
                        {
                            Ok(normalized) => row.custom_properties = normalized,
                            Err(e) => {
                                failures.push((idx, e.current_context().to_string()));
                                continue;
                            }
                        }
                    }
                    valid.push(row)
                }
                Err(e) => {
                    failures.push((idx, e.current_context().to_string()));
                }
            }
        }

        Ok((valid, failures))
    }

    async fn publish_customer_created_events(&self, actor: &Actor, customers: &[Customer]) {
        let _ = futures::future::join_all(customers.iter().map(|customer| {
            self.eventbus.publish(Event::customer_created(
                actor.clone(),
                customer.id,
                customer.tenant_id,
            ))
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>();
    }
}

/// Customer filters (native columns, owner included, and custom properties) resolved to the ids
/// they match through a typed query, keeping raw list SQL static. `None` = no filter.
pub(crate) async fn resolve_customer_filter_ids(
    internal: &StoreInternal,
    conn: &mut PgConn,
    tenant_id: TenantId,
    filters: Vec<Filter>,
) -> StoreResult<Option<Vec<CustomerId>>> {
    if filters.is_empty() {
        return Ok(None);
    }
    let definitions = internal
        .custom_property_definitions
        .load_active(conn, tenant_id, CustomPropertyEntityType::Customer)
        .await?;
    let resolved = resolve_list_filters(ListEntity::Customer, definitions.as_slice(), filters)?;
    CustomerRow::list_matching_ids(conn, tenant_id, resolved)
        .await
        .map(Some)
        .map_err(Into::into)
}

/// Customers an upsert batch just wrote (so already held by its transaction), in id order.
async fn bump_written_customers(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    customers: &[Customer],
) -> StoreResult<()> {
    let ids: Vec<CustomerId> = customers.iter().map(|customer| customer.id).collect();
    let locks = CustomerRow::lock_many(conn, tenant_id, &ids).await?;
    let locks: Vec<&CustomerLock> = locks.iter().collect();
    // An upsert may change existing customers' currency.
    controls
        .wallet_price_changed(conn, &locks, common_domain::clock::now())
        .await?;
    Ok(credit_inputs::bump_customers(conn, &locks).await?)
}

#[cfg(test)]
mod tests {

    mod lifecycle_enum_anchors {
        use diesel_models::enums::{PlanTypeEnum, QuoteStatusEnum, SubscriptionStatusEnum};

        /// Anchor the rule to the REAL enums so a future enum rename trips these tests.
        fn is_not_terminal(status: &SubscriptionStatusEnum) -> bool {
            SubscriptionStatusEnum::not_terminal().contains(status)
        }

        #[test]
        fn real_enum_anchors_hold() {
            // not_terminal() is the canonical "alive" set used by has_active_sub.
            assert!(is_not_terminal(&SubscriptionStatusEnum::Active));
            assert!(is_not_terminal(&SubscriptionStatusEnum::TrialActive));
            assert!(is_not_terminal(&SubscriptionStatusEnum::PendingCharge));
            assert!(is_not_terminal(&SubscriptionStatusEnum::Paused));
            assert!(!is_not_terminal(&SubscriptionStatusEnum::TrialExpired));
            assert!(!is_not_terminal(&SubscriptionStatusEnum::Cancelled));
            assert!(!is_not_terminal(&SubscriptionStatusEnum::Completed));
            assert!(!is_not_terminal(&SubscriptionStatusEnum::Suspended));
            // Free is the plan_type that demotes a paid state to Free.
            assert_eq!(PlanTypeEnum::default(), PlanTypeEnum::Free);
            // Closed-lost quote states.
            for st in [
                QuoteStatusEnum::Declined,
                QuoteStatusEnum::Expired,
                QuoteStatusEnum::Cancelled,
            ] {
                assert!(matches!(
                    st,
                    QuoteStatusEnum::Declined
                        | QuoteStatusEnum::Expired
                        | QuoteStatusEnum::Cancelled
                ));
            }
            for st in [QuoteStatusEnum::Draft, QuoteStatusEnum::Pending] {
                assert!(matches!(
                    st,
                    QuoteStatusEnum::Draft | QuoteStatusEnum::Pending
                ));
            }
        }
    }

    use super::*;
    use crate::domain::custom_property_definitions::CustomPropertyDefinition;
    use crate::domain::enums::CustomPropertyType;
    use crate::domain::filters::{FieldRef, FilterOperator};
    use diesel_models::extend::filtering::{CmpOp, CpKind, ListFilter};

    fn resolve_customer_filters(
        defs: &[CustomPropertyDefinition],
        filters: Vec<Filter>,
    ) -> StoreResult<Vec<ListFilter>> {
        resolve_list_filters(ListEntity::Customer, defs, filters)
    }

    fn empty_row() -> CustomerLifecycleSignalRow {
        CustomerLifecycleSignalRow {
            has_active_paid_sub: false,
            is_trialing: false,
            has_active_free_sub: false,
            has_active_sub: false,
            has_suspended_sub: false,
            has_paused_sub: false,
            had_paid_sub_ended: false,
            ever_paid: false,
            trial_expired: false,
            has_closed_lost_quote: false,
            has_open_quote: false,
            reactivated: false,
        }
    }

    fn state(row: CustomerLifecycleSignalRow) -> String {
        derive_lifecycle_state(&signals_from_row(row))
            .as_str()
            .to_string()
    }

    #[test]
    fn active_paid_row_maps_to_active_paying() {
        let row = CustomerLifecycleSignalRow {
            has_active_paid_sub: true,
            has_active_sub: true,
            ..empty_row()
        };
        assert_eq!(state(row), "active_paying");
    }

    #[test]
    fn suspended_paying_row_maps_to_past_due() {
        let row = CustomerLifecycleSignalRow {
            has_active_paid_sub: true,
            has_suspended_sub: true,
            ever_paid: true,
            has_active_sub: true,
            ..empty_row()
        };
        assert_eq!(state(row), "past_due");
    }

    #[test]
    fn closed_lost_quote_only_row_maps_to_closed_lost() {
        let row = CustomerLifecycleSignalRow {
            has_closed_lost_quote: true,
            ..empty_row()
        };
        assert_eq!(state(row), "closed_lost");
    }

    #[test]
    fn empty_signal_row_falls_through_to_unknown() {
        assert_eq!(state(empty_row()), "unknown");
    }

    fn cp_def(key: &str, property_type: CustomPropertyType) -> CustomPropertyDefinition {
        CustomPropertyDefinition {
            id: common_domain::ids::CustomPropertyDefinitionId::new(),
            tenant_id: TenantId::new(),
            entity_type: CustomPropertyEntityType::Customer,
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            property_type,
            config: crate::domain::custom_property_definitions::PropertyConfig::default(),
            required: false,
            default_value: None,
            display_order: 0,
            archived_at: None,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
            updated_at: None,
        }
    }

    fn cp_filter(key: &str, op: FilterOperator, values: &[&str]) -> Filter {
        Filter {
            field: FieldRef::CustomProperty(key.to_string()),
            op,
            values: values.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn single_select_equals_resolves_to_containment() {
        let defs = vec![cp_def("region", CustomPropertyType::SingleSelect)];
        let resolved = resolve_customer_filters(
            &defs,
            vec![cp_filter("region", FilterOperator::Equals, &["EU"])],
        )
        .unwrap();
        match &resolved[0] {
            ListFilter::CustomProperty(cp) => {
                assert_eq!(cp.key, "region");
                assert!(
                    matches!(&cp.kind, CpKind::Contains(v) if v == &serde_json::json!({"region": "EU"}))
                );
            }
            other => panic!("expected custom property, got {other:?}"),
        }
    }

    #[test]
    fn number_in_resolves_to_numeric_or() {
        let defs = vec![cp_def("seats", CustomPropertyType::Number)];
        let resolved = resolve_customer_filters(
            &defs,
            vec![cp_filter("seats", FilterOperator::In, &["1", "5"])],
        )
        .unwrap();
        match &resolved[0] {
            ListFilter::CustomProperty(cp) => {
                assert!(matches!(&cp.kind, CpKind::NumIn(ns) if ns == &vec![1.0, 5.0]));
            }
            other => panic!("expected custom property, got {other:?}"),
        }
    }

    #[test]
    fn is_set_takes_no_value() {
        let defs = vec![cp_def("region", CustomPropertyType::SingleSelect)];
        let resolved =
            resolve_customer_filters(&defs, vec![cp_filter("region", FilterOperator::IsSet, &[])])
                .unwrap();
        assert!(matches!(
            &resolved[0],
            ListFilter::CustomProperty(cp) if matches!(cp.kind, CpKind::KeyExists)
        ));
    }

    #[test]
    fn date_equals_normalizes_and_compares_lexically() {
        let defs = vec![cp_def("signed", CustomPropertyType::Date)];
        let resolved = resolve_customer_filters(
            &defs,
            vec![cp_filter("signed", FilterOperator::Equals, &["2026-01-02"])],
        )
        .unwrap();
        match &resolved[0] {
            ListFilter::CustomProperty(cp) => {
                assert!(matches!(&cp.kind, CpKind::LexCmp(CmpOp::Eq, s) if s == "2026-01-02"));
            }
            other => panic!("expected custom property, got {other:?}"),
        }
    }

    #[test]
    fn unknown_key_is_rejected() {
        let defs: Vec<CustomPropertyDefinition> = vec![];
        assert!(
            resolve_customer_filters(&defs, vec![cp_filter("nope", FilterOperator::IsSet, &[])])
                .is_err()
        );
    }

    #[test]
    fn operator_not_allowed_for_type_is_rejected() {
        // GT is not a valid operator on a TEXT property.
        let defs = vec![cp_def("note", CustomPropertyType::Text)];
        assert!(
            resolve_customer_filters(&defs, vec![cp_filter("note", FilterOperator::Gt, &["x"])])
                .is_err()
        );
    }

    #[test]
    fn unsupported_native_column_is_rejected() {
        let filter = Filter {
            field: FieldRef::Native("mrr".to_string()),
            op: FilterOperator::Gt,
            values: vec!["100".to_string()],
        };
        assert!(resolve_customer_filters(&[], vec![filter]).is_err());
    }
}

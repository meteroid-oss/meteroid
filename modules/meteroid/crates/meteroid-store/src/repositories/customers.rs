use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::pgmq::{
    HubspotSyncCustomerDomain, HubspotSyncRequestEvent, PennylaneSyncCustomer,
    PennylaneSyncRequestEvent, PgmqMessageNew, PgmqQueue, VatValidationRequestEvent,
};
use crate::domain::{
    ConnectorProviderEnum, Customer, CustomerBatchResult, CustomerBrief, CustomerNew,
    CustomerNewWrapper, CustomerPatch, CustomerTopUpBalance, CustomerUpdate, PaginatedVec,
    PaginationRequest, VatNumberValidationStatus,
};
use crate::errors::StoreError;
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use crate::repositories::pgmq::PgmqInterface;
use crate::store::{PgConn, Store};
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId};
use common_eventbus::Event;
use diesel_models::customers::{CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate};
use diesel_models::subscriptions::SubscriptionRow;
use diesel_models::tenants::TenantRow;
use error_stack::{Report, bail};
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

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

    async fn top_up_customer_balance(&self, req: CustomerTopUpBalance) -> StoreResult<Customer>;

    async fn find_customer_by_id_or_alias(
        &self,
        id_or_alias: AliasOr<CustomerId>,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

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

        if let Some(_ca_id) = customer.connected_account_id {
            // enterprise placeholder
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
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>> {
        // Validate currency if provided
        if let Some(ref currency) = customer.currency {
            let mut conn = self.get_conn().await?;
            let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            validate_customer_currency(currency, &tenant.available_currencies)?;
        }

        let is_valid_vat_number_format = customer.is_valid_vat_number_format();
        // A patch that touches vat_number resets external validation for the new value.
        let vat_number_touched = customer.vat_number.clone();
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
                    let updated: Option<CustomerRow> = patch_model
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    match updated {
                        None => Ok(None),
                        Some(updated) => {
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

    async fn top_up_customer_balance(&self, req: CustomerTopUpBalance) -> StoreResult<Customer> {
        self.transaction(|conn| {
            async move {
                CustomerBalance::update(conn, req.customer_id, req.tenant_id, req.cents, None)
                    .await
                    .map(|x| x.customer)
            }
            .scope_boxed()
        })
        .await
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

        let update_model = CustomerRowUpdate {
            id: by_id_or_alias.id,
            name: customer.name,
            alias: customer.alias,
            billing_email: customer.billing_email,
            invoicing_emails: customer.invoicing_emails.into_iter().map(Some).collect(),
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
        };

        let updated = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let updated = update_model
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;

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

        // Check for blocking subscriptions (active, trial, etc.)
        let blocking_statuses = vec![
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
                CustomerRow::archive(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
                CustomerRow::unarchive(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
        let mut conn = self.get_conn().await?;
        let vies_check = vies_check.and_then(|c| serde_json::to_value(c).ok());
        CustomerRowPatch::update_vat_validation(
            &mut conn,
            tenant_id,
            customer_id,
            expected_vat_number,
            status.into(),
            checked_at,
            vies_check,
        )
        .await
        .map_err(Into::into)
    }

    async fn request_vat_number_revalidation(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Customer> {
        self.transaction(|conn| {
            async move {
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

        // Validate all currencies upfront
        for customer in &batch {
            validate_customer_currency(&customer.currency, &tenant.available_currencies)?;
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

            let invoicing_entity = c
                .invoicing_entity_id
                .as_ref()
                .and_then(|id| invoicing_entities.iter().find(|ie| ie.id == *id))
                .unwrap_or(default_invoicing_entity);

            let vat_number_format_valid = c.is_valid_vat_number_format();

            match (CustomerNewWrapper {
                inner: c,
                invoicing_entity_id: invoicing_entity.id,
                tenant_id,
                vat_number_format_valid,
            })
            .try_into()
            {
                Ok(row) => valid.push(row),
                Err(e) => {
                    let report: Report<StoreError> = e;
                    failures.push((idx, report.current_context().to_string()));
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

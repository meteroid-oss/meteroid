use crate::StoreResult;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::pgmq::{
    HubspotSyncCustomerDomain, HubspotSyncRequestEvent, PennylaneSyncCustomer,
    PennylaneSyncRequestEvent, PgmqQueue,
};
use crate::domain::{
    ConnectorProviderEnum, Customer, CustomerBrief, CustomerNew, CustomerNewWrapper, CustomerPatch,
    CustomerTopUpBalance, CustomerUpdate, OrderByRequest, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::store::Store;
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::customers::{CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, bail};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait CustomersInterface {
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
        order_by: OrderByRequest,
        query: Option<String>,
        archived: Option<bool>,
    ) -> StoreResult<PaginatedVec<Customer>>;

    async fn list_customers_by_ids_global(
        &self,
        ids: Vec<CustomerId>,
    ) -> StoreResult<Vec<Customer>>;

    async fn insert_customer(
        &self,
        customer: CustomerNew,
        tenant_id: TenantId,
    ) -> StoreResult<Customer>;

    async fn insert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>>;

    async fn upsert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>>;

    async fn patch_customer(
        &self,
        actor: Uuid,
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
        actor: Uuid,
        tenant_id: TenantId,
        customer: CustomerUpdate,
    ) -> StoreResult<Customer>;

    async fn archive_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()>;

    async fn unarchive_customer(
        &self,
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
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn find_customer_by_id(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id)
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
        order_by: OrderByRequest,
        query: Option<String>,
        archived: Option<bool>,
    ) -> StoreResult<PaginatedVec<Customer>> {
        let mut conn = self.get_conn().await?;

        let rows = CustomerRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            order_by.into(),
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

    async fn insert_customer(
        &self,
        customer: CustomerNew,
        tenant_id: TenantId,
    ) -> StoreResult<Customer> {
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
                async move {
                    let new_customer: Customer = customer.insert(conn).await?.try_into()?;
                    self.internal
                        .insert_outbox_events_tx(
                            conn,
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
            .publish(Event::customer_created(
                res.created_by,
                res.id.as_uuid(),
                res.tenant_id.as_uuid(),
            ))
            .await;

        Ok(res)
    }

    async fn insert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let prepared_batch = self.prepare_customer_batch(batch, tenant_id).await?;

        let res: Vec<Customer> = self
            .transaction(|conn| {
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
                        .insert_outbox_events_tx(conn, outbox_events)
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        self.publish_customer_created_events(&res).await;
        Ok(res)
    }

    async fn upsert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let prepared_batch = self.prepare_customer_batch(batch, tenant_id).await?;

        let res: Vec<Customer> = self
            .transaction(|conn| {
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
                        .insert_outbox_events_tx(conn, outbox_events)
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        self.publish_customer_created_events(&res).await;
        Ok(res)
    }

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>> {
        let is_valid_vat_number_format = customer.is_valid_vat_number_format();
        let mut patch_model: CustomerRowPatch = customer.try_into()?;
        patch_model.vat_number_format_valid = is_valid_vat_number_format;

        let updated = self
            .transaction(|conn| {
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
                                .insert_outbox_events_tx(conn, outbox_events)
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
                    .publish(Event::customer_patched(
                        actor,
                        updated.id.as_uuid(),
                        tenant_id.as_uuid(),
                    ))
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
        actor: Uuid,
        tenant_id: TenantId,
        customer: CustomerUpdate,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        let by_id_or_alias =
            CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, customer.id_or_alias.clone())
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let vat_number_format_valid = customer.is_valid_vat_number_format();

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
            updated_by: actor,
            invoicing_entity_id: invoicing_entity.id,
            vat_number: customer.vat_number,
            custom_taxes: serde_json::to_value(&customer.custom_taxes).map_err(|e| {
                StoreError::SerdeError("Failed to serialize custom_taxes".to_string(), e)
            })?,
            bank_account_id: customer.bank_account_id,
            vat_number_format_valid,
            is_tax_exempt: customer.is_tax_exempt,
        };

        let updated = self
            .transaction(|conn| {
                async move {
                    let updated = update_model
                        .update(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;

                    let updated: Customer = updated.try_into()?;

                    let outbox_events = vec![OutboxEvent::customer_updated(updated.clone().into())];
                    self.internal
                        .insert_outbox_events_tx(conn, outbox_events)
                        .await?;

                    Ok(updated)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::customer_updated(
                actor,
                updated.id.as_uuid(),
                tenant_id.as_uuid(),
            ))
            .await;

        Ok(updated)
    }

    async fn archive_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()> {
        use diesel_models::enums::SubscriptionStatusEnum as DieselSubscriptionStatusEnum;

        let mut conn = self.get_conn().await?;

        let customer = CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, id_or_alias)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Check for blocking subscriptions (active, trial, etc.)
        let blocking_statuses = vec![
            DieselSubscriptionStatusEnum::Active,
            DieselSubscriptionStatusEnum::TrialActive,
            DieselSubscriptionStatusEnum::PendingCharge,
            // DieselSubscriptionStatusEnum::PendingActivation,
            DieselSubscriptionStatusEnum::TrialExpired,
            DieselSubscriptionStatusEnum::Paused,
            DieselSubscriptionStatusEnum::Suspended,
        ];

        let blocking_subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
            Some(customer.id),
            None,
            Some(blocking_statuses),
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

        CustomerRow::archive(&mut conn, customer.id, tenant_id, actor)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn unarchive_customer(
        &self,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let customer = CustomerRow::find_by_id_or_alias_including_archived(
            &mut conn,
            tenant_id,
            id_or_alias.clone(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        CustomerRow::unarchive(&mut conn, customer.id, tenant_id)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
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

        // Also upsert to customer_connection table (new approach)
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
}

impl Store {
    async fn prepare_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CustomerRowNew>> {
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

    async fn publish_customer_created_events(&self, customers: &[Customer]) {
        let _ = futures::future::join_all(customers.iter().map(|customer| {
            self.eventbus.publish(Event::customer_created(
                customer.created_by,
                customer.id.as_uuid(),
                customer.tenant_id.as_uuid(),
            ))
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>();
    }
}

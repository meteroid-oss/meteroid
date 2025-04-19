use crate::StoreResult;
use crate::domain::enums::{InvoiceStatusEnum, InvoiceType};
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::pgmq::{HubspotSyncCustomerDomain, HubspotSyncRequestEvent, PgmqQueue};
use crate::domain::{
    ConnectorProviderEnum, Customer, CustomerBrief, CustomerBuyCredits, CustomerNew,
    CustomerNewWrapper, CustomerPatch, CustomerTopUpBalance, CustomerUpdate, DetailedInvoice,
    InlineCustomer, InlineInvoicingEntity, InvoiceNew, InvoiceTotals, InvoiceTotalsParams,
    InvoicingEntity, LineItem, OrderByRequest, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::store::Store;
use crate::utils::local_id::{IdType, LocalId};
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRowNew;
use diesel_models::customers::{CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate};
use diesel_models::invoicing_entities::InvoicingEntityRow;
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

    async fn find_customer_by_alias(&self, alias: String) -> StoreResult<Customer>;

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

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>>;

    async fn top_up_customer_balance(&self, req: CustomerTopUpBalance) -> StoreResult<Customer>;

    async fn buy_customer_credits(&self, req: CustomerBuyCredits) -> StoreResult<DetailedInvoice>;

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

    async fn patch_customer_conn_meta(
        &self,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
    ) -> StoreResult<()>;

    async fn sync_customers_to_hubspot(
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

    async fn find_customer_by_alias(&self, alias: String) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_alias(&mut conn, alias)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn find_customer_ids_by_aliases(
        &self,
        tenant_id: TenantId,
        aliases: Vec<String>,
    ) -> StoreResult<Vec<CustomerBrief>> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_aliases(&mut conn, tenant_id, aliases)
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
    ) -> StoreResult<PaginatedVec<Customer>> {
        let mut conn = self.get_conn().await?;

        let rows = CustomerRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            order_by.into(),
            query,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Customer> = PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(|s| s.try_into())
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
            .map(|s| s.try_into())
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

        let customer: CustomerRowNew = CustomerNewWrapper {
            inner: customer,
            invoicing_entity_id: invoicing_entity.id,
            tenant_id,
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
        let invoicing_entities = self.list_invoicing_entities(tenant_id).await?;
        let default_invoicing_entity =
            invoicing_entities
                .iter()
                .find(|ie| ie.is_default)
                .ok_or(StoreError::ValueNotFound(
                    "Default invoicing entity not found".to_string(),
                ))?;

        let insertable_batch: Vec<CustomerRowNew> = batch
            .into_iter()
            .map(|c| {
                let invoicing_entity = c
                    .invoicing_entity_id
                    .as_ref()
                    .and_then(|id| invoicing_entities.iter().find(|ie| ie.id == *id))
                    .unwrap_or(default_invoicing_entity);

                let c: CustomerRowNew = CustomerNewWrapper {
                    inner: c,
                    invoicing_entity_id: invoicing_entity.id,
                    tenant_id,
                }
                .try_into()?;

                Ok(c)
            })
            .collect::<Vec<Result<CustomerRowNew, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let res: Vec<Customer> = self
            .transaction(|conn| {
                async move {
                    let res: Vec<Customer> =
                        CustomerRow::insert_customer_batch(conn, insertable_batch)
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

        let _ = futures::future::join_all(res.clone().into_iter().map(|res| {
            self.eventbus.publish(Event::customer_created(
                res.created_by,
                res.id.as_uuid(),
                res.tenant_id.as_uuid(),
            ))
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>();

        Ok(res)
    }

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>> {
        let mut conn = self.get_conn().await?;

        let patch_model: CustomerRowPatch = customer.try_into()?;

        let updated = patch_model
            .update(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        match updated {
            None => Ok(None),
            Some(updated) => {
                let updated: Customer = updated.try_into()?;

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

    async fn buy_customer_credits(&self, req: CustomerBuyCredits) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        let customer: Customer =
            CustomerRow::find_by_id(&mut conn, &req.customer_id, &req.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

        let invoice = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let now = chrono::Utc::now().naive_utc();

                    let line_items = vec![LineItem {
                        local_id: LocalId::generate_for(IdType::Other),
                        name: "Purchase credits".into(),
                        total: req.cents as i64,
                        subtotal: req.cents as i64,
                        quantity: Some(req.cents.into()),
                        unit_price: Some(1.into()),
                        start_date: now.date(),
                        end_date: now.date(),
                        sub_lines: vec![],
                        is_prorated: false,
                        price_component_id: None,
                        product_id: None,
                        metric_id: None,
                        description: None,
                    }];

                    let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
                        line_items: &line_items,
                        total: 0,
                        amount_due: 0,
                        tax_rate: 0,
                        customer_balance_cents: 0,
                        subscription_applied_coupons: &vec![],
                        invoice_currency: customer.currency.as_str(),
                    });

                    let invoicing_entity: InvoicingEntity =
                        InvoicingEntityRow::select_for_update_by_id_and_tenant(
                            conn,
                            customer.invoicing_entity_id,
                            req.tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into();

                    let address = invoicing_entity.address();

                    let due_at = if invoicing_entity.net_terms > 0 {
                        Some(
                            (now.date()
                                + chrono::Duration::days(invoicing_entity.net_terms as i64))
                            .and_time(chrono::NaiveTime::MIN),
                        )
                    } else {
                        None
                    };

                    let invoice_new = InvoiceNew {
                        status: InvoiceStatusEnum::Finalized,
                        external_status: None,
                        tenant_id: req.tenant_id,
                        customer_id: req.customer_id,
                        subscription_id: None,
                        currency: customer.currency,
                        due_at,
                        plan_name: None,
                        external_invoice_id: None, // todo check later if we want it sync (instead of issue_worker)
                        invoice_number: self.internal.format_invoice_number(
                            invoicing_entity.next_invoice_number,
                            invoicing_entity.invoice_number_pattern,
                            now.date(),
                        ),
                        line_items,
                        issued: false,
                        issue_attempts: 0,
                        last_issue_attempt_at: None,
                        last_issue_error: None,
                        data_updated_at: None,
                        invoice_date: now.date(),
                        total: totals.total,
                        amount_due: totals.amount_due,
                        net_terms: invoicing_entity.net_terms,
                        reference: None,
                        memo: None, // TODO
                        plan_version_id: None,
                        invoice_type: InvoiceType::OneOff,
                        finalized_at: Some(now),
                        subtotal: totals.subtotal,
                        subtotal_recurring: totals.subtotal_recurring,
                        tax_rate: 0, // TODO
                        tax_amount: totals.tax_amount,
                        customer_details: InlineCustomer {
                            billing_address: customer.billing_address.clone(),
                            id: req.customer_id,
                            name: customer.name,
                            alias: customer.alias,
                            email: customer.billing_email,
                            vat_number: customer.vat_number,
                            snapshot_at: now,
                        },
                        seller_details: InlineInvoicingEntity {
                            address,
                            id: invoicing_entity.id,
                            legal_name: invoicing_entity.legal_name.clone(),
                            vat_number: invoicing_entity.vat_number.clone(),
                            snapshot_at: now,
                        },
                    };

                    let inserted_invoice = insert_invoice_tx(self, conn, invoice_new).await?;

                    InvoicingEntityRow::update_invoicing_entity_number(
                        conn,
                        invoicing_entity.id,
                        req.tenant_id,
                        invoicing_entity.next_invoice_number,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let tx = CustomerBalancePendingTxRowNew {
                        id: Uuid::now_v7(),
                        amount_cents: req.cents,
                        note: req.notes,
                        invoice_id: inserted_invoice.id,
                        tenant_id: req.tenant_id,
                        customer_id: req.customer_id,
                        tx_id: None,
                        created_by: req.created_by,
                    };

                    tx.insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(inserted_invoice)
                }
                .scope_boxed()
            })
            .await?;

        self.find_invoice_by_id(req.tenant_id, invoice.id).await
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
        };

        let updated = update_model
            .update(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;

        let _ = self
            .eventbus
            .publish(Event::customer_updated(
                actor,
                updated.id.as_uuid(),
                tenant_id.as_uuid(),
            ))
            .await;

        CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, customer.id_or_alias.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(TryInto::try_into)
    }

    async fn archive_customer(
        &self,
        actor: Uuid,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let customer = CustomerRow::find_by_id_or_alias(&mut conn, tenant_id, id_or_alias)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
            Some(customer.id),
            None,
            PaginationRequest {
                per_page: Some(1),
                page: 0,
            }
            .into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        // this is a temp solution that will be replaced with a more complex logic
        if subscriptions.total_results > 0 {
            return Err(StoreError::InvalidArgument(
                "Cannot archive customer with active subscriptions".to_string(),
            )
            .into());
        }

        CustomerRow::archive(&mut conn, customer.id, tenant_id, actor)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn patch_customer_conn_meta(
        &self,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        CustomerRowPatch::upsert_conn_meta(
            &mut conn,
            provider.into(),
            customer_id,
            connector_id,
            external_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
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
}

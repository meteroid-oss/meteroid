use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use uuid::Uuid;

use crate::domain::enums::{InvoiceStatusEnum, InvoiceType};
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{
    Customer, CustomerBrief, CustomerBuyCredits, CustomerForDisplay, CustomerNew,
    CustomerNewWrapper, CustomerPatch, CustomerTopUpBalance, CustomerUpdate, DetailedInvoice,
    Identity, InlineCustomer, InlineInvoicingEntity, InvoiceNew, InvoiceTotals,
    InvoiceTotalsParams, InvoicingEntity, LineItem, OrderByRequest, PaginatedVec,
    PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::InvoiceInterface;
use crate::store::Store;
use crate::utils::local_id::{IdType, LocalId};
use crate::StoreResult;
use common_eventbus::Event;
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRowNew;
use diesel_models::customers::{
    CustomerForDisplayRow, CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate,
};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::query::IdentityDb;

#[async_trait::async_trait]
pub trait CustomersInterface {
    async fn find_customer_by_id(&self, id: Identity, tenant_id: Uuid) -> StoreResult<Customer>;

    async fn find_customer_by_alias(&self, alias: String) -> StoreResult<Customer>;

    async fn find_customer_ids_by_aliases(
        &self,
        tenant_id: Uuid,
        aliases: Vec<String>,
    ) -> StoreResult<Vec<CustomerBrief>>;

    async fn list_customers(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<Customer>>;

    async fn list_customers_by_ids(&self, ids: Vec<Uuid>) -> StoreResult<Vec<Customer>>;

    async fn insert_customer(
        &self,
        customer: CustomerNew,
        tenant_id: Uuid,
    ) -> StoreResult<Customer>;

    async fn insert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<Customer>>;

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: Uuid,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>>;

    async fn top_up_customer_balance(&self, req: CustomerTopUpBalance) -> StoreResult<Customer>;

    async fn buy_customer_credits(&self, req: CustomerBuyCredits) -> StoreResult<DetailedInvoice>;

    async fn find_customer_by_local_id_or_alias(
        &self,
        id_or_alias: String,
        tenant_id: Uuid,
    ) -> StoreResult<CustomerForDisplay>;

    async fn list_customers_for_display(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<CustomerForDisplay>>;

    async fn update_customer(
        &self,
        actor: Uuid,
        tenant_id: Uuid,
        customer: CustomerUpdate,
    ) -> StoreResult<CustomerForDisplay>;
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn find_customer_by_id(
        &self,
        customer_id: Identity,
        tenant_id: Uuid,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_id(&mut conn, customer_id.into(), tenant_id)
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
        tenant_id: Uuid,
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
        tenant_id: Uuid,
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

    async fn list_customers_by_ids(&self, ids: Vec<Uuid>) -> StoreResult<Vec<Customer>> {
        let mut conn = self.get_conn().await?;

        CustomerRow::list_by_ids(&mut conn, ids)
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
        tenant_id: Uuid,
    ) -> StoreResult<Customer> {
        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, customer.invoicing_entity_id.clone())
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
                res.id,
                res.tenant_id,
            ))
            .await;

        Ok(res)
    }

    async fn insert_customer_batch(
        &self,
        batch: Vec<CustomerNew>,
        tenant_id: Uuid,
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
                    .and_then(|id| {
                        invoicing_entities.iter().find(|ie| match id {
                            Identity::UUID(id) => ie.id == *id,
                            Identity::LOCAL(id) => ie.local_id == *id,
                        })
                    })
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
                res.id,
                res.tenant_id,
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
        tenant_id: Uuid,
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

    async fn buy_customer_credits(&self, req: CustomerBuyCredits) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        let customer =
            CustomerRow::find_by_id(&mut conn, IdentityDb::UUID(req.customer_id), req.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

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
                            &customer.invoicing_entity_id,
                            &req.tenant_id,
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
                            billing_address: None, // TODO
                            id: req.customer_id,
                            name: customer.name,
                            alias: customer.alias,
                            email: customer.email,
                            vat_number: None, // TODO
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
                        &invoicing_entity.id,
                        &req.tenant_id,
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

    async fn find_customer_by_local_id_or_alias(
        &self,
        id_or_alias: String,
        tenant_id: Uuid,
    ) -> StoreResult<CustomerForDisplay> {
        let mut conn = self.get_conn().await?;

        CustomerForDisplayRow::find_by_local_id_or_alias(&mut conn, tenant_id, id_or_alias)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn list_customers_for_display(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<CustomerForDisplay>> {
        let mut conn = self.get_conn().await?;

        let rows = CustomerForDisplayRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            order_by.into(),
            query,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<CustomerForDisplay> = PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Vec<Result<CustomerForDisplay, Report<StoreError>>>>()
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn update_customer(
        &self,
        actor: Uuid,
        tenant_id: Uuid,
        customer: CustomerUpdate,
    ) -> StoreResult<CustomerForDisplay> {
        let mut conn = self.get_conn().await?;

        let by_id_or_alias = CustomerForDisplayRow::find_by_local_id_or_alias(
            &mut conn,
            tenant_id,
            customer.local_id_or_alias.clone(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let update_model = CustomerRowUpdate {
            id: by_id_or_alias.id,
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
            invoicing_email: customer.invoicing_email,
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
            billing_config: customer.billing_config.try_into()?,
            invoicing_entity_id: invoicing_entity.id,
        };

        let updated = update_model
            .update(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or(StoreError::ValueNotFound("Customer not found".to_string()))?;

        let _ = self
            .eventbus
            .publish(Event::customer_updated(actor, updated.id, tenant_id))
            .await;

        CustomerForDisplayRow::find_by_local_id_or_alias(
            &mut conn,
            tenant_id,
            customer.local_id_or_alias.clone(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(TryInto::try_into)
    }
}

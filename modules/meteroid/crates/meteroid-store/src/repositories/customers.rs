use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use uuid::Uuid;

use crate::domain::{
    Customer, CustomerBrief, CustomerNew, CustomerPatch, CustomerTopUpBalance, OrderByRequest,
    PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::customer_balance::CustomerBalance;
use crate::store::Store;
use crate::StoreResult;
use common_eventbus::Event;
use diesel_models::customers::{CustomerRow, CustomerRowNew, CustomerRowPatch};

#[async_trait::async_trait]
pub trait CustomersInterface {
    async fn find_customer_by_id(&self, id: Uuid, tenant_id: Uuid) -> StoreResult<Customer>;

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

    async fn insert_customer(&self, customer: CustomerNew) -> StoreResult<Customer>;

    async fn insert_customer_batch(&self, batch: Vec<CustomerNew>) -> StoreResult<Vec<Customer>>;

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: Uuid,
        customer: CustomerPatch,
    ) -> StoreResult<Option<Customer>>;

    async fn top_up_customer_balance(&self, req: CustomerTopUpBalance) -> StoreResult<Customer>;
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn find_customer_by_id(
        &self,
        customer_id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        CustomerRow::find_by_id(&mut conn, customer_id, tenant_id)
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

    async fn insert_customer(&self, customer: CustomerNew) -> StoreResult<Customer> {
        let mut conn = self.get_conn().await?;

        let insertable_entity: CustomerRowNew = customer.try_into()?;

        let res: Customer = insertable_entity
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)?;

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

    async fn insert_customer_batch(&self, batch: Vec<CustomerNew>) -> StoreResult<Vec<Customer>> {
        let mut conn = self.get_conn().await?;

        let insertable_batch: Vec<CustomerRowNew> = batch
            .into_iter()
            .map(|c| c.try_into())
            .collect::<Vec<Result<CustomerRowNew, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let res: Vec<Customer> = CustomerRow::insert_customer_batch(&mut conn, insertable_batch)
            .await
            .map_err(Into::into)
            .and_then(|v| v.into_iter().map(TryInto::try_into).collect())?;

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

        let patch_model: CustomerRowPatch = CustomerRowPatch {
            id: customer.id,
            name: customer.name,
            alias: customer.alias,
            email: customer.email,
            invoicing_email: customer.invoicing_email,
            phone: customer.phone,
            balance_value_cents: customer.balance_value_cents,
            balance_currency: customer.balance_currency,
            billing_address: customer.billing_address,
            shipping_address: customer.shipping_address,
        };

        let updated = patch_model
            .update(&mut conn, customer.id)
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
                CustomerBalance::update(conn, req.customer_id, req.tenant_id, req.cents, None).await
            }
            .scope_boxed()
        })
        .await
    }
}

use error_stack::Report;
use uuid::Uuid;

use common_eventbus::Event;

use crate::domain::{Customer, CustomerPatch, OrderByRequest, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};

#[async_trait::async_trait]
pub trait CustomersInterface {
    async fn find_customer_by_id(&self, id: Uuid) -> StoreResult<domain::Customer>;
    async fn find_customer_by_alias(&self, alias: String) -> StoreResult<domain::Customer>;
    async fn list_customers(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<domain::Customer>>;

    async fn insert_customer(&self, customer: domain::CustomerNew)
        -> StoreResult<domain::Customer>;

    async fn insert_customer_batch(
        &self,
        batch: Vec<domain::CustomerNew>,
    ) -> StoreResult<Vec<domain::Customer>>;

    async fn patch_customer(
        &self,
        actor: Uuid,
        tenant_id: Uuid,
        customer: domain::CustomerPatch,
    ) -> StoreResult<Option<domain::Customer>>;
}

#[async_trait::async_trait]
impl CustomersInterface for Store {
    async fn find_customer_by_id(&self, customer_id: Uuid) -> StoreResult<domain::Customer> {
        let mut conn = self.get_conn().await?;

        diesel_models::customers::Customer::find_by_id(&mut conn, customer_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn find_customer_by_alias(&self, alias: String) -> StoreResult<domain::Customer> {
        let mut conn = self.get_conn().await?;

        diesel_models::customers::Customer::find_by_alias(&mut conn, alias)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn list_customers(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        query: Option<String>,
    ) -> StoreResult<PaginatedVec<Customer>> {
        let mut conn = self.get_conn().await?;

        let rows = diesel_models::customers::Customer::list(
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

    async fn insert_customer(
        &self,
        customer: domain::CustomerNew,
    ) -> StoreResult<domain::Customer> {
        let mut conn = self.get_conn().await?;

        let insertable_entity: diesel_models::customers::CustomerNew = customer.try_into()?;

        let res: domain::Customer = insertable_entity
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

    async fn insert_customer_batch(
        &self,
        batch: Vec<domain::CustomerNew>,
    ) -> StoreResult<Vec<domain::Customer>> {
        let mut conn = self.get_conn().await?;

        let insertable_batch: Vec<diesel_models::customers::CustomerNew> = batch
            .into_iter()
            .map(|c| c.try_into())
            .collect::<Vec<Result<diesel_models::customers::CustomerNew, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let res: Vec<Customer> =
            diesel_models::customers::Customer::insert_customer_batch(&mut conn, insertable_batch)
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

        let patch_model: diesel_models::customers::CustomerPatch =
            diesel_models::customers::CustomerPatch {
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
}

use crate::StoreResult;
use crate::domain::{CustomerBrief, CustomerHierarchy};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::query::customer_hierarchy::{list_ancestors, list_children};
use error_stack::Report;

/// Bounds the walk up the hierarchy; far deeper than any real one.
pub const MAX_ANCESTORS: i32 = 1000;

#[async_trait::async_trait]
pub trait CustomerHierarchyInterface {
    async fn get_customer_hierarchy(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<CustomerHierarchy>;
}

#[async_trait::async_trait]
impl CustomerHierarchyInterface for Store {
    async fn get_customer_hierarchy(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<CustomerHierarchy> {
        let mut conn = self.get_conn().await?;
        CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        load_hierarchy(&mut conn, tenant_id, customer_id).await
    }
}

/// The customer's ancestors, nearest first.
pub async fn ancestors_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<Vec<CustomerBrief>> {
    Ok(list_ancestors(conn, tenant_id, customer_id, MAX_ANCESTORS)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|a| CustomerBrief {
            id: a.id,
            name: a.name,
            alias: a.alias,
        })
        .collect())
}

pub async fn load_hierarchy(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<CustomerHierarchy> {
    let ancestors = ancestors_of(conn, tenant_id, customer_id).await?;
    let children = list_children(conn, tenant_id, customer_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(CustomerHierarchy {
        ancestors,
        children,
    })
}

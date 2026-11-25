//! Customers as API v2 reads them (ADR-0013 §11): with when they last changed, listed by keyset in
//! SQL, the collection being unbounded.
use crate::StoreResult;
use crate::domain::Customer;
use crate::errors::StoreError;
use crate::store::Store;
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::customers::CustomerRow;
use error_stack::Report;

/// What a page of customers is narrowed to.
#[derive(Debug, Clone, Default)]
pub struct CustomerPageFilter {
    /// Matched anywhere in the name, the alias or the billing email, ignoring case.
    pub search: Option<String>,
    /// The archived customers instead of the live ones.
    pub archived: bool,
    /// Only those written after it.
    pub updated_after: Option<DateTime<Utc>>,
}

/// A customer with when its row was last written (`store_committed_at`, trigger-maintained: the
/// `updated_at` column is not written by every path).
#[derive(Debug, Clone)]
pub struct ListedCustomer {
    pub customer: Customer,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<CustomerRow> for ListedCustomer {
    type Error = Report<StoreError>;

    fn try_from(row: CustomerRow) -> Result<Self, Self::Error> {
        let updated_at = row.store_committed_at.and_utc();
        Ok(ListedCustomer {
            customer: row.try_into()?,
            updated_at,
        })
    }
}

#[async_trait::async_trait]
pub trait CustomerPagesInterface {
    /// The tenant's customers of `filter` in id order, those after `after`, at most `limit`.
    async fn list_customers_after(
        &self,
        tenant_id: TenantId,
        filter: CustomerPageFilter,
        after: Option<CustomerId>,
        limit: usize,
    ) -> StoreResult<Vec<ListedCustomer>>;

    /// The customer, archived or not.
    async fn get_listed_customer(
        &self,
        tenant_id: TenantId,
        id: CustomerId,
    ) -> StoreResult<ListedCustomer>;
}

#[async_trait::async_trait]
impl CustomerPagesInterface for Store {
    async fn list_customers_after(
        &self,
        tenant_id: TenantId,
        filter: CustomerPageFilter,
        after: Option<CustomerId>,
        limit: usize,
    ) -> StoreResult<Vec<ListedCustomer>> {
        let mut conn = self.get_conn().await?;
        CustomerRow::list_page(
            &mut conn,
            tenant_id,
            filter
                .search
                .map(|search| search.trim().to_owned())
                .filter(|search| !search.is_empty()),
            filter.archived,
            filter.updated_after,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(ListedCustomer::try_from)
        .collect()
    }

    async fn get_listed_customer(
        &self,
        tenant_id: TenantId,
        id: CustomerId,
    ) -> StoreResult<ListedCustomer> {
        let mut conn = self.get_conn().await?;
        CustomerRow::find_by_id(&mut conn, &id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()
    }
}

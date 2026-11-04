//! Row-lock order: the customer row, then its rows (subscriptions, invoices, grants, notes,
//! settlements, checkout sessions, payments, operations), then tenant rows. Locking a customer's row
//! takes a `CustomerLock`; claims (`SKIP LOCKED`) never wait. See check-lock-sites.sh.

use crate::errors::DatabaseError;
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomerId, TenantId};
use diesel_async::AsyncPgConnection;

/// The customer row is locked (`FOR NO KEY UPDATE`) in the transaction of the connection it was
/// taken on, until that transaction ends.
#[derive(Debug)]
pub struct CustomerLock {
    tenant_id: TenantId,
    customer_id: CustomerId,
    connection: usize,
}

impl CustomerLock {
    pub(crate) fn taken(conn: &PgConn, tenant_id: TenantId, customer_id: CustomerId) -> Self {
        Self {
            tenant_id,
            customer_id,
            connection: connection_of(conn),
        }
    }

    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    pub fn customer_id(&self) -> CustomerId {
        self.customer_id
    }

    /// Checks that a row about to be locked belongs to this customer, on the same connection.
    pub(crate) fn covers(
        &self,
        conn: &PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<()> {
        self.on(conn);
        if self.tenant_id != tenant_id || self.customer_id != customer_id {
            return Err(DatabaseError::Others(format!(
                "lock order: a row of customer {customer_id} locked under the lock of customer {}",
                self.customer_id
            ))
            .into());
        }
        Ok(())
    }

    /// Checks that the lock was taken on this connection: a lock held by another transaction
    /// orders nothing here.
    pub(crate) fn on(&self, conn: &PgConn) {
        debug_assert_eq!(
            self.connection,
            connection_of(conn),
            "lock order: customer {} was locked on another connection",
            self.customer_id
        );
    }
}

fn connection_of(conn: &PgConn) -> usize {
    std::ptr::from_ref::<AsyncPgConnection>(conn) as usize
}

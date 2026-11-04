use crate::enums::CreditInputChangeKind;
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, TenantId};
use diesel::{QueryableByName, sql_types};

/// Feed shards, as `credit_input_shard()` computes them.
pub const CREDIT_INPUT_SHARDS: i16 = 16;

/// A position in the change feed: the poll returns rows strictly after it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreditFeedCursor {
    pub store_xid: i64,
    pub id: i64,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct CreditInputChangeRow {
    #[diesel(sql_type = sql_types::BigInt)]
    pub id: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub store_xid: i64,
    #[diesel(sql_type = sql_types::SmallInt)]
    pub shard: i16,
    #[diesel(sql_type = sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Uuid>)]
    pub customer_id: Option<CustomerId>,
    #[diesel(sql_type = crate::schema::sql_types::CreditInputChangeKindEnum)]
    pub kind: CreditInputChangeKind,
    #[diesel(sql_type = sql_types::Date)]
    pub created_on: NaiveDate,
}

impl CreditInputChangeRow {
    pub fn cursor(&self) -> CreditFeedCursor {
        CreditFeedCursor {
            store_xid: self.store_xid,
            id: self.id,
        }
    }
}

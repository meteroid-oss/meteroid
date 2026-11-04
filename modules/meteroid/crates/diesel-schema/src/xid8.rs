//! `xid8`, a transaction id, read as the `i64` it fits in (its epoch stays far below 2^31).

use crate::schema::sql_types::Xid8;
use diesel::deserialize::{self, FromSql};
use diesel::pg::{Pg, PgValue};

impl FromSql<Xid8, Pg> for i64 {
    fn from_sql(bytes: PgValue<'_>) -> deserialize::Result<Self> {
        <i64 as FromSql<diesel::sql_types::BigInt, Pg>>::from_sql(bytes)
    }
}

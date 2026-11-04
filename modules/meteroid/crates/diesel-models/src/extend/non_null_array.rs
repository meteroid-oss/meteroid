use diesel::deserialize::{self, FromSql, FromSqlRow};
use diesel::pg::{Pg, PgValue};
use diesel::sql_types::{Array, Nullable};

/// Reads a Postgres array whose elements are never NULL in practice (PG cannot declare it),
/// failing on a NULL element rather than dropping it.
#[derive(Debug, Clone, FromSqlRow)]
pub struct NonNullArray<T>(pub Vec<T>);

impl<ST, T> FromSql<Array<Nullable<ST>>, Pg> for NonNullArray<T>
where
    Vec<Option<T>>: FromSql<Array<Nullable<ST>>, Pg>,
{
    fn from_sql(bytes: PgValue<'_>) -> deserialize::Result<Self> {
        let items = <Vec<Option<T>> as FromSql<Array<Nullable<ST>>, Pg>>::from_sql(bytes)?;
        items
            .into_iter()
            .collect::<Option<Vec<T>>>()
            .map(NonNullArray)
            .ok_or_else(|| "unexpected NULL element in a non-null array column".into())
    }
}

impl<T> From<NonNullArray<T>> for Vec<T> {
    fn from(value: NonNullArray<T>) -> Self {
        value.0
    }
}

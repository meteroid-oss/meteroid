//! Every two tables may share a query, as `allow_tables_to_appear_in_same_query!` allows, from 4
//! impls per table instead of ~10 per pair (coherence over those took ~55 s of this crate's build).
//! Each table gets a base-16 id (`build.rs`); two tables differ when one of their digits does.

use diesel::Table;
use diesel::internal::table_macro::TablesampleMethod;
use diesel::query_builder::{Only, Tablesample};
use diesel::query_source::TableNotEqual;

pub struct Nil;
pub struct Digit<Head, Tail>(Head, Tail);

pub trait TableId {
    type Id;
}

pub trait IdNe<Other> {}

impl<H, A: IdNe<B>, B> IdNe<Digit<H, B>> for Digit<H, A> {}

macro_rules! digits {
    ($($digit:ident)*) => {$(
        pub struct $digit;
    )*};
}

macro_rules! digits_differ {
    ($($left:ident $right:ident),*) => {$(
        impl<A, B> IdNe<Digit<$right, B>> for Digit<$left, A> {}
    )*};
}

impl<T: TableId> TableId for Only<T> {
    type Id = T::Id;
}

impl<T: TableId, TSM: TablesampleMethod> TableId for Tablesample<T, TSM> {
    type Id = T::Id;
}

macro_rules! table_ids {
    ($($table:ident => $id:ty;)*) => {$(
        impl TableId for crate::schema::$table::table {
            type Id = $id;
        }

        impl<T> TableNotEqual<T> for crate::schema::$table::table
        where
            T: Table + TableId,
            $id: IdNe<T::Id>,
        {
        }

        impl<T> TableNotEqual<crate::schema::$table::table> for Only<T>
        where
            T: TableId,
            Only<T>: Table,
            $id: IdNe<T::Id>,
        {
        }

        impl<T, TSM> TableNotEqual<crate::schema::$table::table> for Tablesample<T, TSM>
        where
            T: TableId,
            TSM: TablesampleMethod,
            Tablesample<T, TSM>: Table,
            $id: IdNe<T::Id>,
        {
        }
    )*};
}

include!(concat!(env!("OUT_DIR"), "/table_ids.rs"));

use diesel::pg::Pg;
use diesel::query_builder::{AstPass, Query, QueryFragment};
use diesel::sql_types;
use diesel::sql_types::{BigInt, SqlType};
use diesel::{Expression, QueryId, QueryResult};
use std::borrow::Cow;

use diesel_async::methods::LoadQuery;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

pub struct CursorPaginationRequest {
    pub limit: Option<u32>,
    pub cursor: Option<uuid::Uuid>,
}

impl Default for CursorPaginationRequest {
    fn default() -> Self {
        CursorPaginationRequest {
            limit: None,
            cursor: None,
        }
    }
}

pub struct CursorPaginatedVec<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, QueryId)]
pub struct CursorPaginated<T, C> {
    query: T,
    per_page: i64,
    // borrow checker won this round
    per_page_plus_one: i64,
    cursor: Option<Uuid>,
    cursor_column: C,
    cursor_column_name: String,
}

pub trait CursorPaginate: Sized {
    fn cursor_paginate<C, S>(
        self,
        pagination: CursorPaginationRequest,
        cursor_column: C,
        cursor_column_name: S,
    ) -> CursorPaginated<Self, C>
    where
        C: Expression,
        C::SqlType: SqlType,
        S: Into<Cow<'static, str>>;
}

impl<T> CursorPaginate for T {
    fn cursor_paginate<C, S>(
        self,
        pagination: CursorPaginationRequest,
        cursor_column: C,
        cursor_column_name: S,
    ) -> CursorPaginated<Self, C>
    where
        C: Expression,
        C::SqlType: SqlType,
        S: Into<Cow<'static, str>>,
    {
        let per_page = pagination.limit.unwrap_or(DEFAULT_PER_PAGE) as i64;

        CursorPaginated {
            query: self,
            per_page,
            cursor: pagination.cursor,
            per_page_plus_one: per_page + 1,
            cursor_column,
            cursor_column_name: cursor_column_name.into().into_owned(),
        }
    }
}

const DEFAULT_PER_PAGE: u32 = 25;

impl<T, C> CursorPaginated<T, C>
where
    C: Expression,
    C::SqlType: SqlType,
{
    pub fn per_page(self, per_page: u32) -> Self {
        CursorPaginated {
            per_page: per_page as i64,
            per_page_plus_one: per_page as i64 + 1,
            ..self
        }
    }

    pub fn load_and_get_next_cursor<'a, U, F>(
        self,
        conn: &'a mut AsyncPgConnection,
        get_cursor_value: F,
    ) -> impl std::future::Future<Output = QueryResult<CursorPaginatedVec<U>>> + Send + 'a
    where
        Self: LoadQuery<'a, AsyncPgConnection, U> + 'a,
        U: Send + 'a,
        T: 'a,
        F: Fn(&U) -> uuid::Uuid + Send + 'a,
    {
        let per_page = self.per_page;
        let results = self.get_results::<U>(conn);

        async move {
            let mut results = results.await?;
            let next_cursor = if results.len() > per_page as usize {
                let last_item = results.pop().unwrap();
                Some(get_cursor_value(&last_item))
            } else {
                None
            };
            Ok(CursorPaginatedVec {
                items: results,
                next_cursor,
            })
        }
    }
}

impl<T: Query, C> Query for CursorPaginated<T, C>
where
    T: QueryFragment<Pg>,
    C: QueryFragment<Pg>,
{
    type SqlType = T::SqlType;
}

// impl<T> diesel_async::RunQueryDsl<AsyncPgConnection> for CursorPaginated<T> {}
impl<T, C> QueryFragment<Pg> for CursorPaginated<T, C>
where
    T: QueryFragment<Pg>,
    C: QueryFragment<Pg>,
{
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.push_sql("SELECT * FROM (");
        self.query.walk_ast(out.reborrow())?;
        out.push_sql(") t");
        if let Some(ref cursor) = self.cursor {
            out.push_sql(" WHERE t.");
            // self.cursor_column.walk_ast(out.reborrow())?;
            out.push_sql(&self.cursor_column_name);
            out.push_sql(" >= ");
            out.push_bind_param::<sql_types::Uuid, _>(cursor)?;
        }

        out.push_sql(" ORDER BY t.");
        out.push_sql(&self.cursor_column_name);
        out.push_sql(" LIMIT ");
        out.push_bind_param::<BigInt, _>(&self.per_page_plus_one)?;

        Ok(())
    }
}

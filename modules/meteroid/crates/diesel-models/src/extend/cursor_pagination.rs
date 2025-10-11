use diesel::pg::Pg;
use diesel::query_builder::{AstPass, Query, QueryFragment};
use diesel::sql_types;
use diesel::sql_types::BigInt;
use diesel::{QueryId, QueryResult};
use std::borrow::Cow;

use diesel_async::methods::LoadQuery;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use uuid::Uuid;

#[derive(Default)]
pub struct CursorPaginationRequest {
    pub limit: Option<u32>,
    pub cursor: Option<uuid::Uuid>,
}

pub struct CursorPaginatedVec<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<uuid::Uuid>,
}

#[derive(Debug, QueryId)]
pub struct CursorPaginated<T> {
    query: T,
    per_page: i64,
    // borrow checker won this round
    per_page_plus_one: i64,
    cursor: Option<Uuid>,
    cursor_column_name: String,
}

pub trait CursorPaginate: Sized {
    fn cursor_paginate<S>(
        self,
        pagination: CursorPaginationRequest,
        cursor_column_name: S,
    ) -> CursorPaginated<Self>
    where
        S: Into<Cow<'static, str>>;
}

impl<T> CursorPaginate for T {
    fn cursor_paginate<S>(
        self,
        pagination: CursorPaginationRequest,
        cursor_column_name: S,
    ) -> CursorPaginated<Self>
    where
        S: Into<Cow<'static, str>>,
    {
        let per_page = i64::from(pagination.limit.unwrap_or(DEFAULT_PER_PAGE));

        CursorPaginated {
            query: self,
            per_page,
            cursor: pagination.cursor,
            per_page_plus_one: per_page + 1,
            cursor_column_name: cursor_column_name.into().into_owned(),
        }
    }
}

const DEFAULT_PER_PAGE: u32 = 25;

impl<T> CursorPaginated<T> {
    pub fn per_page(self, per_page: u32) -> Self {
        CursorPaginated {
            per_page: i64::from(per_page),
            per_page_plus_one: i64::from(per_page) + 1,
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

impl<T: Query> Query for CursorPaginated<T>
where
    T: QueryFragment<Pg>,
{
    type SqlType = T::SqlType;
}

// impl<T> diesel_async::RunQueryDsl<AsyncPgConnection> for CursorPaginated<T> {}
impl<T> QueryFragment<Pg> for CursorPaginated<T>
where
    T: QueryFragment<Pg>,
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

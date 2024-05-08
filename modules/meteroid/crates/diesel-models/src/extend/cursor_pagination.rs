use diesel::pg::Pg;
use diesel::query_builder::{AstPass, Query, QueryFragment};
use diesel::{QueryId, QueryResult};

use diesel_async::methods::LoadQuery;
use diesel_async::AsyncPgConnection;

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
    pub total: i64,
}

pub trait CursorPaginate: Sized {
    /**
     * Paginate the query using a cursor (id -> Uuid)
     */
    fn cursor_paginate(self, page: CursorPaginationRequest) -> CursorPaginated<Self>;

    /**
     * Paginate the query using a cursor (id -> Uuid)
     */
    fn cursor_paginate_opt(self, page: Option<CursorPaginationRequest>) -> CursorPaginated<Self>;
}

impl<T> CursorPaginate for T {
    fn cursor_paginate(self, page: CursorPaginationRequest) -> CursorPaginated<Self> {
        let limit = page.limit.unwrap_or(DEFAULT_PER_PAGE) as i64;

        CursorPaginated {
            query: self,
            limit,
            cursor: page.cursor,
        }
    }

    fn cursor_paginate_opt(self, page: Option<CursorPaginationRequest>) -> CursorPaginated<Self> {
        self.cursor_paginate(page.unwrap_or_default())
    }
}

const DEFAULT_PER_PAGE: u32 = 25;

#[derive(Debug, Clone, Copy, QueryId)]
pub struct CursorPaginated<T> {
    query: T,
    cursor: Option<uuid::Uuid>,
    limit: i64,
}

impl<T> CursorPaginated<T> {
    pub fn limit(self, limit: u32) -> Self {
        CursorPaginated {
            limit: limit as i64,
            ..self
        }
    }

    pub async fn load_and_count_pages<'a, U, F>(
        self,
        conn: &mut AsyncPgConnection,
        cursor_fn: F,
    ) -> QueryResult<CursorPaginatedVec<U>>
    where
        Self: LoadQuery<'a, AsyncPgConnection, (U, i64)>,
        F: Fn(&U) -> uuid::Uuid,
        U: Send,
        T: 'a,
    {
        let results: Vec<(U, i64)> = <CursorPaginated<T> as diesel_async::RunQueryDsl<
            AsyncPgConnection,
        >>::load::<(U, i64)>(self, conn)
        .await?;

        let total = results.first().map(|x| x.1).unwrap_or(0);
        let records: Vec<U> = results.into_iter().map(|x| x.0).collect();
        let cursor = records.last().map(|a| cursor_fn(a));

        Ok(CursorPaginatedVec {
            items: records,
            total,
            next_cursor: cursor,
        })
    }
}

impl<T: Query> Query for CursorPaginated<T> {
    type SqlType = T::SqlType;
}

// impl<T> diesel_async::RunQueryDsl<AsyncPgConnection> for CursorPaginated<T> {}

impl<T> QueryFragment<Pg> for CursorPaginated<T>
where
    T: QueryFragment<Pg>,
{
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        self.query.walk_ast(out.reborrow())?;
        out.push_sql(" LIMIT ");
        out.push_bind_param::<diesel::sql_types::BigInt, _>(&self.limit)?;

        if let Some(ref cursor) = self.cursor {
            out.push_sql(" WHERE id > ");
            out.push_bind_param::<diesel::sql_types::Uuid, _>(cursor)?;
        }

        Ok(())
    }
}

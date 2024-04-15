use diesel::{
    pg::Pg,
    query_builder::{AstPass, Query, QueryFragment},
    sql_types::BigInt,
    PgConnection, QueryId, QueryResult,
};
use diesel_async::{methods::LoadQuery, AsyncPgConnection, RunQueryDsl};

pub struct PaginationRequest {
    pub per_page: Option<u32>,
    pub page: u32,
}

impl Default for PaginationRequest {
    fn default() -> Self {
        PaginationRequest {
            per_page: None,
            page: 1,
        }
    }
}

pub struct PaginatedVec<T> {
    pub items: Vec<T>,
    pub total_pages: u32,
    pub total_results: u64,
}

pub trait Paginate: Sized {
    fn paginate(self, page: PaginationRequest) -> Paginated<Self>;
    fn paginate_opt(self, page: Option<PaginationRequest>) -> Paginated<Self>;
}

impl<T> Paginate for T {
    fn paginate(self, page: PaginationRequest) -> Paginated<Self> {
        let per_page = page.per_page.unwrap_or(DEFAULT_PER_PAGE) as i64;

        Paginated {
            query: self,
            per_page,
            page: page.page as i64,
            offset: (page.page as i64 - 1) * per_page,
        }
    }

    fn paginate_opt(self, page: Option<PaginationRequest>) -> Paginated<Self> {
        self.paginate(page.unwrap_or_default())
    }
}

const DEFAULT_PER_PAGE: u32 = 25;

#[derive(Debug, Clone, Copy, QueryId)]
pub struct Paginated<T> {
    query: T,
    page: i64,
    per_page: i64,
    offset: i64,
}

impl<T> Paginated<T> {
    pub fn per_page(self, per_page: u32) -> Self {
        Paginated {
            per_page: per_page as i64,
            offset: (self.page - 1) * (per_page as i64),
            ..self
        }
    }

    // We manually "type" the Future instead of declaring this method as async because the
    // lifetime of the returned Future is tied to the lifetime of the connection and need
    // to be Send, and the compiler cannot infer that by itself.
    pub fn load_and_count_pages<'a, U>(
        self,
        conn: &'a mut AsyncPgConnection,
    ) -> impl std::future::Future<Output = QueryResult<PaginatedVec<U>>> + Send + 'a
    where
        Self: LoadQuery<'a, AsyncPgConnection, (U, i64)> + 'a,
        U: Send + 'a,
        T: 'a,
    {
        // Ignore those linting errors. `get(0)` cannot be replaced with `first()`.
        #![allow(clippy::get_first)]

        let per_page = self.per_page;

        let results = self.get_results::<(U, i64)>(conn);

        async move {
            let results = results.await?;
            let total = results.get(0).map(|x| x.1).unwrap_or(0);
            let records = results.into_iter().map(|x| x.0).collect();
            let total_pages = (total as f64 / per_page as f64).ceil() as i64;
            Ok(PaginatedVec {
                items: records,
                total_pages: total_pages as u32,
                total_results: total as u64,
            })
        }
    }
}

impl<T: Query> Query for Paginated<T> {
    type SqlType = (T::SqlType, BigInt);
}

// impl<T> diesel_async::RunQueryDsl<AsyncPgConnection> for Paginated<T> {}
impl<T> diesel::RunQueryDsl<PgConnection> for Paginated<T> {}

impl<T> diesel::RunQueryDsl<AsyncPgConnection> for Paginated<T> {}

impl<T> QueryFragment<Pg> for Paginated<T>
where
    T: QueryFragment<Pg>,
{
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.push_sql("SELECT *, COUNT(*) OVER () FROM (");
        self.query.walk_ast(out.reborrow())?;
        out.push_sql(") t LIMIT ");
        out.push_bind_param::<BigInt, _>(&self.per_page)?;
        out.push_sql(" OFFSET ");
        out.push_bind_param::<BigInt, _>(&self.offset)?;
        Ok(())
    }
}

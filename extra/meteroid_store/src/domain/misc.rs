use chrono::NaiveDate;
use uuid::Uuid;

pub struct PaginationRequest {
    pub per_page: Option<u32>,
    pub page: u32,
}

impl Into<diesel_models::extend::pagination::PaginationRequest> for PaginationRequest {
    fn into(self) -> diesel_models::extend::pagination::PaginationRequest {
        diesel_models::extend::pagination::PaginationRequest {
            per_page: self.per_page,
            page: self.page,
        }
    }
}

pub struct PaginatedVec<T> {
    pub items: Vec<T>,
    pub total_pages: u32,
    pub total_results: u64,
}

impl<T> Into<PaginatedVec<T>> for diesel_models::extend::pagination::PaginatedVec<T> {
    fn into(self) -> PaginatedVec<T> {
        PaginatedVec {
            items: self.items.into_iter().map(|x| x.into()).collect(),
            total_pages: self.total_pages,
            total_results: self.total_results,
        }
    }
}

pub struct CursorPaginationRequest {
    pub limit: Option<u32>,
    pub cursor: Option<Uuid>,
}

impl Into<diesel_models::extend::cursor_pagination::CursorPaginationRequest>
    for CursorPaginationRequest
{
    fn into(self) -> diesel_models::extend::cursor_pagination::CursorPaginationRequest {
        diesel_models::extend::cursor_pagination::CursorPaginationRequest {
            limit: self.limit,
            cursor: self.cursor,
        }
    }
}

pub struct CursorPaginatedVec<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<Uuid>,
    pub total: i64,
}

impl<T> Into<CursorPaginatedVec<T>>
    for diesel_models::extend::cursor_pagination::CursorPaginatedVec<T>
{
    fn into(self) -> CursorPaginatedVec<T> {
        CursorPaginatedVec {
            items: self.items,
            next_cursor: self.next_cursor,
            total: self.total,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TenantContext {
    pub actor: Actor,
    pub tenant_id: Uuid,
}

#[derive(Clone, Copy, Debug)]
pub struct OrganizationContext {
    pub actor: Actor,
    pub organization_id: Uuid,
}

#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub actor: Actor,
    pub tenant_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug)]
pub enum Actor {
    System,
    User(Uuid),
    ApiKey(Uuid),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[derive(Debug, Clone)]
pub struct ComponentPeriods {
    pub arrear: Option<Period>,
    pub advance: Option<Period>,
    pub proration_factor: Option<f64>,
}

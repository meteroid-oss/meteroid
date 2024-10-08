use chrono::NaiveDate;
use o2o::o2o;
use uuid::Uuid;

pub struct PaginationRequest {
    pub per_page: Option<u32>,
    pub page: u32,
}

impl From<PaginationRequest> for diesel_models::extend::pagination::PaginationRequest {
    fn from(val: PaginationRequest) -> Self {
        diesel_models::extend::pagination::PaginationRequest {
            per_page: val.per_page,
            page: val.page,
        }
    }
}

pub struct PaginatedVec<T> {
    pub items: Vec<T>,
    pub total_pages: u32,
    pub total_results: u64,
}

impl<T> From<diesel_models::extend::pagination::PaginatedVec<T>> for PaginatedVec<T> {
    fn from(val: diesel_models::extend::pagination::PaginatedVec<T>) -> Self {
        PaginatedVec {
            items: val.items.into_iter().collect(),
            total_pages: val.total_pages,
            total_results: val.total_results,
        }
    }
}

pub struct CursorPaginationRequest {
    pub limit: Option<u32>,
    pub cursor: Option<Uuid>,
}

impl From<CursorPaginationRequest>
    for diesel_models::extend::cursor_pagination::CursorPaginationRequest
{
    fn from(val: CursorPaginationRequest) -> Self {
        diesel_models::extend::cursor_pagination::CursorPaginationRequest {
            limit: val.limit,
            cursor: val.cursor,
        }
    }
}

pub struct CursorPaginatedVec<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<Uuid>,
}

impl<T> From<diesel_models::extend::cursor_pagination::CursorPaginatedVec<T>>
    for CursorPaginatedVec<T>
{
    fn from(val: diesel_models::extend::cursor_pagination::CursorPaginatedVec<T>) -> Self {
        CursorPaginatedVec {
            items: val.items,
            next_cursor: val.next_cursor,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TenantContext {
    // pub actor: Actor, // TODO
    pub actor: Uuid,
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
    pub advance: Period,
    pub proration_factor: Option<f64>,
}

#[derive(Debug, Clone, o2o)]
#[map_owned(diesel_models::extend::order::OrderByRequest)]
pub enum OrderByRequest {
    IdAsc,
    IdDesc,
    DateAsc,
    DateDesc,
    NameAsc,
    NameDesc,
}

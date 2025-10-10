use common_grpc::meteroid::common::v1::{Pagination, PaginationResponse};
use tonic::Status;

pub mod uuid_gen {

    pub fn v7() -> uuid::Uuid {
        uuid::Uuid::from_bytes(*uuid::Uuid::now_v7().as_bytes())
    }
}

pub fn parse_uuid(uuid: &str, resource_name: &str) -> Result<uuid::Uuid, Status> {
    uuid::Uuid::parse_str(uuid).map_err(|e| {
        Status::invalid_argument(format!("Failed to parse UUID at {resource_name}: {e}"))
    })
}

#[macro_export]
macro_rules! parse_uuid {
    ($uuid:expr) => {
        parse_uuid(&$uuid, stringify!($uuid))
    };
}

// let's do a parse_uuid_opt
pub fn parse_uuid_opt(
    uuid: &Option<String>,
    resource_name: &str,
) -> Result<Option<uuid::Uuid>, Status> {
    match uuid {
        Some(uuid_str) if !uuid_str.is_empty() => {
            uuid::Uuid::parse_str(uuid_str).map(Some).map_err(|e| {
                Status::invalid_argument(format!("Failed to parse UUID at {resource_name}: {e}"))
            })
        }
        _ => Ok(None),
    }
}

pub trait PaginationExt {
    #[allow(clippy::wrong_self_convention)]
    fn into_response(&self, total_pages: u32, total_items: u64) -> Option<PaginationResponse>;

    #[allow(clippy::wrong_self_convention)]
    fn into_domain(&self) -> meteroid_store::domain::PaginationRequest;
}

impl PaginationExt for Option<Pagination> {
    fn into_response(&self, total_pages: u32, total_items: u64) -> Option<PaginationResponse> {
        self.as_ref().map(|p| PaginationResponse {
            page: p.page,
            per_page: p.per_page.unwrap_or(10),
            total_items: total_items as u32,
            total_pages,
        })
    }

    fn into_domain(&self) -> meteroid_store::domain::PaginationRequest {
        meteroid_store::domain::PaginationRequest {
            page: self.as_ref().map_or(0, |p| p.page),
            per_page: self.as_ref().and_then(|p| p.per_page),
        }
    }
}

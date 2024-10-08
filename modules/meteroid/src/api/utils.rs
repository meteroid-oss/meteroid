use tonic::Status;

use common_grpc::meteroid::common::v1::{Pagination, PaginationResponse};

pub mod uuid_gen {
    pub fn v7() -> uuid::Uuid {
        uuid::Uuid::from_bytes(*uuid7::uuid7().as_bytes())
    }
}

pub fn parse_uuid(uuid: &str, resource_name: &str) -> Result<uuid::Uuid, Status> {
    uuid::Uuid::parse_str(uuid).map_err(|e| {
        Status::invalid_argument(format!("Failed to parse UUID at {}: {}", resource_name, e))
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
                Status::invalid_argument(format!(
                    "Failed to parse UUID at {}: {}",
                    resource_name, e
                ))
            })
        }
        _ => Ok(None),
    }
}

pub trait PaginationExt {
    fn limit(&self) -> i64;
    fn limit_or(&self, default: u32) -> i64;

    fn offset(&self) -> i64;
    fn offset_or(&self, default: u32) -> i64;
    #[allow(clippy::wrong_self_convention)]
    fn into_response(&self, total: u32) -> Option<PaginationResponse>;
}

impl PaginationExt for Option<Pagination> {
    fn limit(&self) -> i64 {
        self.limit_or(100)
    }
    fn limit_or(&self, default: u32) -> i64 {
        self.as_ref().map(|p| p.limit).unwrap_or(default) as i64
    }
    fn offset(&self) -> i64 {
        self.offset_or(0)
    }
    fn offset_or(&self, default: u32) -> i64 {
        self.as_ref().map(|p| p.offset).unwrap_or(default) as i64
    }

    fn into_response(&self, total: u32) -> Option<PaginationResponse> {
        self.as_ref().map(|p| PaginationResponse {
            total,
            limit: p.limit,
            offset: p.offset,
        })
    }
}

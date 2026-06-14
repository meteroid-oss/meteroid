#![allow(non_snake_case)]

use anyhow::{Result, anyhow};
use http::Uri;
use rust_decimal::Decimal;
use std::fmt::Formatter;
use std::str::FromStr;

pub mod code;
pub mod middleware;

pub mod meteroid {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("meteroid.common.v1");
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum GrpcKind {
    CLIENT,
    SERVER,
}

impl std::fmt::Display for GrpcKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GrpcKind::CLIENT => f.write_str("client"),
            GrpcKind::SERVER => f.write_str("server"),
        }
    }
}

// note: same struct as tonic::GrpcMethod
//       but without extract/parse
#[derive(Debug, Clone)]
pub struct GrpcServiceMethod {
    pub service: String,
    pub method: String,
}
impl GrpcServiceMethod {
    pub fn extract(uri: &Uri) -> GrpcServiceMethod {
        let mut parts = uri.path().split('/').filter(|x| !x.is_empty());
        let service = parts.next().unwrap_or_default();
        let method = parts.next().unwrap_or_default();

        Self {
            service: service.to_string(),
            method: method.to_string(),
        }
    }
}

use meteroid::common::v1 as common;

impl From<Decimal> for common::Decimal {
    fn from(rd: Decimal) -> Self {
        common::Decimal {
            value: rd.to_string(),
        }
    }
}

impl TryFrom<common::Decimal> for Decimal {
    type Error = anyhow::Error;
    fn try_from(d: common::Decimal) -> Result<Self> {
        Decimal::from_str(&d.value)
            .map_err(|e| anyhow!("Failed to convert string to Decimal: {}", e))
    }
}

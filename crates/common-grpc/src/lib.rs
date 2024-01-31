#![allow(non_snake_case)]

use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};
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
    service: String,
    method: String,
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

impl From<NaiveDate> for common::Date {
    fn from(nd: NaiveDate) -> Self {
        common::Date {
            year: nd.year(),
            month: nd.month(),
            day: nd.day(),
        }
    }
}

impl From<time::Date> for common::Date {
    fn from(d: time::Date) -> Self {
        common::Date {
            year: d.year(),
            month: (d.month() as u8).into(),
            day: d.day().into(),
        }
    }
}

impl TryFrom<common::Date> for NaiveDate {
    type Error = anyhow::Error;
    fn try_from(d: common::Date) -> Result<Self> {
        NaiveDate::from_ymd_opt(d.year, d.month, d.day).ok_or_else(|| {
            anyhow!(
                "Invalid date provided for year: {}, month: {}, day: {}",
                d.year,
                d.month,
                d.day
            )
        })
    }
}

impl TryFrom<common::Date> for time::Date {
    type Error = anyhow::Error;
    fn try_from(d: common::Date) -> Result<Self> {
        let month: time::Month = (d.month as u8).try_into()?;

        time::Date::from_calendar_date(d.year, month, d.day as u8).map_err(|_| {
            anyhow!(
                "Invalid date provided for year: {}, month: {}, day: {}",
                d.year,
                d.month,
                d.day
            )
        })
    }
}

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

use crate::mapping;
use crate::mapping::MappingError;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use time::PrimitiveDateTime;

pub fn chrono_utc_now() -> DateTime<Utc> {
    Utc::now()
}

pub trait ChronoExt {
    fn to_primitive_dt(&self) -> Result<PrimitiveDateTime, MappingError>;
}

impl ChronoExt for DateTime<Utc> {
    fn to_primitive_dt(&self) -> Result<PrimitiveDateTime, MappingError> {
        mapping::common::chrono_to_datetime(self.naive_utc())
    }
}

pub trait TimeExt {
    fn to_chrono(&self) -> Result<NaiveDateTime, MappingError>;
}

impl TimeExt for PrimitiveDateTime {
    fn to_chrono(&self) -> Result<NaiveDateTime, MappingError> {
        mapping::common::datetime_to_chrono(self)
    }
}

pub trait DateExt {
    fn to_chrono(&self) -> Result<NaiveDate, MappingError>;
}

impl DateExt for time::Date {
    fn to_chrono(&self) -> Result<NaiveDate, MappingError> {
        mapping::common::date_to_chrono(*self)
    }
}

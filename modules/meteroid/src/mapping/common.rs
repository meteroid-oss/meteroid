use crate::mapping::MappingError;
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use time::Month;

pub fn date_to_chrono(time_date: time::Date) -> Result<NaiveDate, MappingError> {
    NaiveDate::from_ymd_opt(
        time_date.year(),
        time_date.month() as u32,
        time_date.day() as u32,
    )
    .ok_or(MappingError::new("Failed to convert date to chrono"))
}

pub fn chrono_to_date(date: NaiveDate) -> Result<time::Date, MappingError> {
    let year = date.year();
    let day = date.day() as u8;
    let month: Month = Month::try_from(date.month() as u8)
        .map_err(|_| MappingError::new("Failed to convert u8 to time::Month"))?;

    time::Date::from_calendar_date(year, month, day)
        .map_err(|_| MappingError::new("Failed to convert chrono to date"))
}

pub fn chrono_to_datetime(
    datetime: NaiveDateTime,
) -> Result<time::PrimitiveDateTime, MappingError> {
    let date = chrono_to_date(datetime.date())?;
    let time = time::Time::from_hms_milli(
        datetime.time().hour() as u8,
        datetime.time().minute() as u8,
        datetime.time().second() as u8,
        datetime.and_utc().timestamp_subsec_millis() as u16,
    )
    .map_err(|_| MappingError::new("Failed to convert chrono::Time to time::Time"))?;

    Ok(time::PrimitiveDateTime::new(date, time))
}

pub fn time_to_chrono(time_date: time::Time) -> Result<NaiveTime, MappingError> {
    NaiveTime::from_hms_nano_opt(
        time_date.hour() as u32,
        time_date.minute() as u32,
        time_date.second() as u32,
        time_date.nanosecond(),
    )
    .ok_or(MappingError::new("Failed to convert time to chrono"))
}

pub fn datetime_to_chrono(
    time_date: &time::PrimitiveDateTime,
) -> Result<NaiveDateTime, MappingError> {
    let time = time_to_chrono(time_date.time())?;
    let date = date_to_chrono(time_date.date())?;

    Ok(NaiveDateTime::new(date, time))
}

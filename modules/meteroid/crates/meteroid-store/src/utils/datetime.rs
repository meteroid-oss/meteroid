use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

pub fn start_of_week(date: NaiveDate) -> NaiveDate {
    if date.weekday() == Weekday::Mon {
        return date;
    }
    date - chrono::Duration::days(date.weekday().num_days_from_monday().into())
}

pub fn end_of_week(date: NaiveDate) -> NaiveDate {
    if date.weekday() == Weekday::Sun {
        return date;
    }
    let days_until_sunday = 7 - date.weekday().num_days_from_sunday();
    date + chrono::Duration::days(days_until_sunday.into())
}

pub fn start_of_month(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(i64::from(date.day()) - 1)
}

pub fn end_of_month(date: NaiveDate) -> NaiveDate {
    let next_month = date
        .checked_add_months(Months::new(1))
        .expect("month overflow");

    start_of_month(next_month)
        .pred_opt()
        .expect("date underflow")
}

pub fn start_of_quarter(date: NaiveDate) -> NaiveDate {
    let month = date.month();
    let quarter_start_month: u32 = match month {
        1..=3 => 1,
        4..=6 => 4,
        7..=9 => 7,
        10..=12 => 10,
        _ => unreachable!(),
    };
    NaiveDate::from_ymd_opt(date.year(), quarter_start_month, 1).expect("invalid quarter")
}

pub fn end_of_quarter(date: NaiveDate) -> NaiveDate {
    let month = date.month();
    let quarter_end_month: u32 = match month {
        1..=3 => 3,
        4..=6 => 6,
        7..=9 => 9,
        10..=12 => 12,
        _ => unreachable!(),
    };

    let quarter_end_month_start_day =
        NaiveDate::from_ymd_opt(date.year(), quarter_end_month, 1).expect("invalid quarter");

    end_of_month(quarter_end_month_start_day)
}

pub fn start_of_year(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(i64::from(date.ordinal()) - 1)
}

pub fn end_of_year(date: NaiveDate) -> NaiveDate {
    let next_year = date
        .checked_add_months(Months::new(12))
        .expect("date overflow");

    start_of_year(next_year).pred_opt().expect("date underflow")
}

pub fn sub_months(date: NaiveDate, n: u32) -> NaiveDate {
    date.checked_sub_months(Months::new(n))
        .expect("invalid date")
}

fn start_of(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN)
}

/// Share of `[start, end_exclusive)` elapsed at `now`, clamped to 0..=1; an empty period is 1.
pub fn elapsed_fraction(start: NaiveDate, end_exclusive: NaiveDate, now: NaiveDateTime) -> f64 {
    elapsed_between(start_of(start), start_of(end_exclusive), now)
}

/// [`elapsed_fraction`] between two instants.
pub fn elapsed_between(start: NaiveDateTime, end: NaiveDateTime, now: NaiveDateTime) -> f64 {
    let total = (end - start).num_seconds();
    if total <= 0 {
        return 1.0;
    }
    ((now - start).num_seconds() as f64 / total as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use crate::utils::datetime::*;
    use chrono::{Datelike, NaiveDate, Weekday};

    #[test]
    fn test_start_of_week() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap(); // wednesday
        let start_of_week = start_of_week(date);
        assert_eq!(start_of_week, NaiveDate::from_ymd_opt(2024, 2, 19).unwrap());
        assert_eq!(start_of_week.weekday(), Weekday::Mon);
    }

    #[test]
    fn test_end_of_week() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap(); // wednesday
        let end_of_week = end_of_week(date);
        assert_eq!(end_of_week, NaiveDate::from_ymd_opt(2024, 2, 25).unwrap());
        assert_eq!(end_of_week.weekday(), Weekday::Sun);
    }

    #[test]
    fn test_start_of_month() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let start_of_month = start_of_month(date);
        assert_eq!(start_of_month, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());
    }

    #[test]
    fn test_end_of_month() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let end_of_month = end_of_month(date);
        assert_eq!(end_of_month, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
    }

    #[test]
    fn test_start_of_quarter() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let start_of_quarter = start_of_quarter(date);
        assert_eq!(
            start_of_quarter,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        );
    }

    #[test]
    fn test_end_of_quarter() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let end_of_quarter = end_of_quarter(date);
        assert_eq!(
            end_of_quarter,
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
        );
    }

    #[test]
    fn test_start_of_year() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let start_of_year = start_of_year(date);
        assert_eq!(start_of_year, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }

    #[test]
    fn test_end_of_year() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();

        let end_of_year = end_of_year(date);
        assert_eq!(end_of_year, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
    }

    #[test]
    fn test_sub_months() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 21).unwrap();
        let res = sub_months(date, 2);
        assert_eq!(res, NaiveDate::from_ymd_opt(2023, 12, 21).unwrap());

        let res = sub_months(date, 12);
        assert_eq!(res, NaiveDate::from_ymd_opt(2023, 2, 21).unwrap());
    }

    #[test]
    fn elapsed_between_instants_is_clamped() {
        let at = |day: u32| {
            NaiveDate::from_ymd_opt(2026, 9, day)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        };
        let (start, end) = (at(1), at(11));
        assert_eq!(elapsed_between(start, end, at(6)), 0.5);
        assert_eq!(elapsed_between(start, end, at(20)), 1.0);
        assert_eq!(elapsed_between(start, start, at(6)), 1.0);
    }
}

pub mod date_utils {
    use time::util;
    use time::Month;

    pub fn start_of_week(date: time::Date) -> time::Date {
        if date.weekday() == time::Weekday::Monday {
            return date;
        }
        date.prev_occurrence(time::Weekday::Monday)
    }
    pub fn end_of_week(date: time::Date) -> time::Date {
        if date.weekday() == time::Weekday::Sunday {
            return date;
        }
        date.next_occurrence(time::Weekday::Sunday)
    }
    pub fn start_of_month(date: time::Date) -> time::Date {
        date - time::Duration::days(date.day() as i64 - 1)
    }
    pub fn end_of_month(date: time::Date) -> time::Date {
        let days_in_month = util::days_in_year_month(date.year(), date.month());
        date.replace_day(days_in_month).expect("day overflow")
    }
    pub fn start_of_quarter(date: time::Date) -> time::Date {
        let month = date.month() as u8;
        let quarter_start_month: u8 = match month {
            1 | 2 | 3 => 1,
            4 | 5 | 6 => 4,
            7 | 8 | 9 => 7,
            10 | 11 | 12 => 10,
            _ => unreachable!(),
        };
        time::Date::from_calendar_date(
            date.year(),
            Month::try_from(quarter_start_month).expect("invalid month"),
            1,
        )
        .expect("invalid quarter")
    }
    pub fn end_of_quarter(date: time::Date) -> time::Date {
        let month = date.month() as u8;
        let quarter_end_month: u8 = match month {
            1 | 2 | 3 => 3,
            4 | 5 | 6 => 6,
            7 | 8 | 9 => 9,
            10 | 11 | 12 => 12,
            _ => unreachable!(),
        };
        let month = Month::try_from(quarter_end_month).expect("invalid month");
        let days_in_month = util::days_in_year_month(date.year(), month);
        time::Date::from_calendar_date(date.year(), month, days_in_month).expect("invalid quarter")
    }
    pub fn start_of_year(date: time::Date) -> time::Date {
        date - time::Duration::days(date.ordinal() as i64 - 1)
    }
    pub fn end_of_year(date: time::Date) -> time::Date {
        let days_in_year = util::days_in_year(date.year());
        date + time::Duration::days(days_in_year as i64 - date.ordinal() as i64)
    }

    pub fn sub_months(date: time::Date, n: i32) -> time::Date {
        let mut year = date.year();
        let mut month = date.month() as i32 - n;
        while month <= 0 {
            year -= 1;
            month += 12;
        }
        time::Date::from_calendar_date(
            year,
            Month::try_from(month as u8).expect("invalid month"),
            date.day(),
        )
        .expect("invalid date")
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::date_utils;
    use time::{Date, Month};

    #[test]
    fn test_start_of_week() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap(); // wednesday
        let start_of_week = date_utils::start_of_week(date);
        assert_eq!(
            start_of_week,
            Date::from_calendar_date(2024, Month::February, 19).unwrap()
        );
        assert_eq!(start_of_week.weekday(), time::Weekday::Monday);
    }

    #[test]
    fn test_end_of_week() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap();
        let end_of_week = date_utils::end_of_week(date);
        assert_eq!(
            end_of_week,
            Date::from_calendar_date(2024, Month::February, 25).unwrap()
        );
        assert_eq!(end_of_week.weekday(), time::Weekday::Sunday);
    }

    #[test]
    fn test_start_of_month() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap();
        let start_of_month = date_utils::start_of_month(date);
        assert_eq!(
            start_of_month,
            Date::from_calendar_date(2024, Month::February, 1).unwrap()
        );
    }

    #[test]
    fn test_end_of_month() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap();
        let end_of_month = date_utils::end_of_month(date);
        assert_eq!(
            end_of_month,
            Date::from_calendar_date(2024, Month::February, 29).unwrap()
        );
    }

    #[test]
    fn test_start_of_quarter() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap();
        let start_of_quarter = date_utils::start_of_quarter(date);
        assert_eq!(
            start_of_quarter,
            Date::from_calendar_date(2024, Month::January, 1).unwrap()
        );
    }

    #[test]
    fn test_end_of_quarter() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap();
        let end_of_quarter = date_utils::end_of_quarter(date);
        assert_eq!(
            end_of_quarter,
            Date::from_calendar_date(2024, Month::March, 31).unwrap()
        );
    }

    #[test]
    fn test_start_of_year() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap(); // wednesday
        let start_of_year = date_utils::start_of_year(date);
        assert_eq!(
            start_of_year,
            Date::from_calendar_date(2024, Month::January, 1).unwrap()
        );
    }

    #[test]
    fn test_end_of_year() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap(); // wednesday
        let end_of_year = date_utils::end_of_year(date);
        assert_eq!(
            end_of_year,
            Date::from_calendar_date(2024, Month::December, 31).unwrap()
        );
    }

    #[test]
    fn test_sub_months() {
        let date = Date::from_calendar_date(2024, Month::February, 21).unwrap(); // wednesday
        let sub_months = date_utils::sub_months(date, 2);
        assert_eq!(
            sub_months,
            Date::from_calendar_date(2023, Month::December, 21).unwrap()
        );

        let sub_months = date_utils::sub_months(date, 12);
        assert_eq!(
            sub_months,
            Date::from_calendar_date(2023, Month::February, 21).unwrap()
        );
    }
}

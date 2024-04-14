use common_utils::date::NaiveDateExt;

use chrono::{Datelike, Months, NaiveDate};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

pub fn calculate_period_idx(
    billing_start_date: NaiveDate,
    billing_day: u32,
    invoice_date: NaiveDate,
    billing_period: BillingPeriod,
) -> i32 {
    let month_diff = invoice_date.year() * 12 + invoice_date.month() as i32
        - (billing_start_date.year() * 12 + billing_start_date.month() as i32);
    let day_adjustment =
        if invoice_date.day() >= billing_day && billing_start_date.day() < billing_day {
            1
        } else {
            0
        };

    match billing_period {
        BillingPeriod::Monthly => month_diff + day_adjustment,
        BillingPeriod::Quarterly => {
            // Quarterly billing divides the month difference by 3 (since a quarter is 3 months)
            (month_diff / 3) + day_adjustment
        }
        BillingPeriod::Annual => {
            // Annual billing divides the month difference by 12 (since a year is 12 months)
            (month_diff / 12) + day_adjustment
        }
    }
}

pub fn calculate_period_range(
    billing_start_date: NaiveDate,
    billing_day: u32,
    period_index: i32,
    billing_period: BillingPeriod,
) -> (NaiveDate, NaiveDate) {
    let months_in_period = billing_period.months_value();

    let start_day_after_billing_day = billing_start_date.day() >= billing_day;

    fn add_months_at_billing_day(
        date: NaiveDate,
        months_to_add: u32,
        billing_day: u32,
    ) -> Option<NaiveDate> {
        date.checked_add_months(Months::new(months_to_add as u32))
            .and_then(|d| d.with_day(d.days_in_month().min(billing_day)))
    }

    let period_start = if period_index > 0 {
        let month_adjustment: i32 = if start_day_after_billing_day { 0 } else { -1 };
        let months_to_add = (period_index + month_adjustment) as u32 * months_in_period;

        add_months_at_billing_day(billing_start_date, months_to_add, billing_day).unwrap()
    } else {
        billing_start_date
    };

    let period_end = if start_day_after_billing_day || period_index > 0 {
        add_months_at_billing_day(period_start, months_in_period, billing_day).unwrap()
    } else {
        period_start
            .with_day(period_start.days_in_month().min(billing_day))
            .unwrap()
    };

    (period_start, period_end)
}

#[cfg(test)]
mod test {
    use super::{calculate_period_idx, calculate_period_range};
    use chrono::NaiveDate;
    use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
    use rstest::rstest;

    #[rstest]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, 0, "2021-01-01", "2021-02-01")]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, 1, "2021-02-01", "2021-03-01")]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, 2, "2021-03-01", "2021-04-01")]
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, 0, "2021-01-10", "2021-02-01")]
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, 1, "2021-02-01", "2021-03-01")]
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, 2, "2021-03-01", "2021-04-01")]
    #[case(
        BillingPeriod::Monthly,
        "2021-01-01",
        10,
        0,
        "2021-01-01",
        "2021-01-10"
    )]
    #[case(
        BillingPeriod::Monthly,
        "2021-01-01",
        10,
        1,
        "2021-01-10",
        "2021-02-10"
    )]
    #[case(
        BillingPeriod::Monthly,
        "2021-01-01",
        10,
        2,
        "2021-02-10",
        "2021-03-10"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-10",
        1,
        0,
        "2021-01-10",
        "2021-04-01"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-10",
        1,
        1,
        "2021-04-01",
        "2021-07-01"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-10",
        1,
        2,
        "2021-07-01",
        "2021-10-01"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-01",
        10,
        0,
        "2021-01-01",
        "2021-01-10"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-01",
        10,
        1,
        "2021-01-10",
        "2021-04-10"
    )]
    #[case(
        BillingPeriod::Quarterly,
        "2021-01-01",
        10,
        2,
        "2021-04-10",
        "2021-07-10"
    )]
    #[case(BillingPeriod::Annual, "2021-01-10", 1, 0, "2021-01-10", "2022-01-01")]
    #[case(BillingPeriod::Annual, "2021-01-10", 1, 1, "2022-01-01", "2023-01-01")]
    #[case(BillingPeriod::Annual, "2021-01-10", 1, 2, "2023-01-01", "2024-01-01")]
    #[trace]
    fn test_calculate_period_range(
        #[case] billing_period: BillingPeriod,
        #[case] billing_start_date: NaiveDate,
        #[case] billing_day: u32,
        #[case] period_idx: i32,
        #[case] expected_period_start: NaiveDate,
        #[case] expected_period_end: NaiveDate,
    ) {
        let (start, stop) =
            calculate_period_range(billing_start_date, billing_day, period_idx, billing_period);
        assert_eq!(start, expected_period_start);
        assert_eq!(stop, expected_period_end);
    }

    #[rstest]
    #[case(BillingPeriod::Monthly, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 10, "2021-02-20", 2)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 10, "2022-01-02", 12)]
    // test Monthly with billing_start_date.day() after billing_day
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, "2021-01-31", 0)]
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, "2021-02-01", 1)]
    #[case(BillingPeriod::Monthly, "2021-01-10", 1, "2021-02-28", 1)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, "2021-01-31", 0)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, "2021-02-01", 1)]
    #[case(BillingPeriod::Monthly, "2021-01-01", 1, "2021-02-28", 1)]
    #[case(BillingPeriod::Quarterly, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriod::Quarterly, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriod::Quarterly, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriod::Quarterly, "2021-01-01", 10, "2022-01-01", 4)]
    #[case(BillingPeriod::Quarterly, "2021-01-01", 10, "2022-01-12", 5)]
    #[case(BillingPeriod::Annual, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriod::Annual, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriod::Annual, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriod::Annual, "2021-01-01", 10, "2022-01-01", 1)]
    #[case(BillingPeriod::Annual, "2021-01-01", 10, "2022-01-12", 2)]
    #[trace]
    fn test_calculate_period_idx(
        #[case] billing_period: BillingPeriod,
        #[case] billing_start_date: NaiveDate,
        #[case] billing_day: u32,
        #[case] current_date: NaiveDate,
        #[case] expected_period_idx: i32,
    ) {
        let period_idx = calculate_period_idx(
            billing_start_date,
            billing_day,
            current_date,
            billing_period,
        );
        assert_eq!(period_idx, expected_period_idx);
    }
}

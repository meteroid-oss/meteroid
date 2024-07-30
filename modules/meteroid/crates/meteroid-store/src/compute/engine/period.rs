use common_utils::date::NaiveDateExt;

use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{ComponentPeriods, Period};
use chrono::{Datelike, Months, NaiveDate};

pub fn calculate_component_period(
    billing_start_date: NaiveDate,
    billing_day: u32,
    invoice_date: NaiveDate,
    billing_period: &SubscriptionFeeBillingPeriod,
) -> Option<ComponentPeriods> {
    if !applies_this_period(billing_start_date, invoice_date, &billing_period) {
        return None;
    }

    let billing_period = match billing_period {
        SubscriptionFeeBillingPeriod::OneTime => None,
        SubscriptionFeeBillingPeriod::Monthly => Some(BillingPeriodEnum::Monthly),
        SubscriptionFeeBillingPeriod::Quarterly => Some(BillingPeriodEnum::Quarterly),
        SubscriptionFeeBillingPeriod::Annual => Some(BillingPeriodEnum::Annual),
    };

    match billing_period {
        None => Some(ComponentPeriods {
            proration_factor: None,
            advance: Period {
                start: invoice_date,
                end: invoice_date,
            },
            arrear: None,
        }),
        Some(billing_period) => {
            let period_idx = calculate_period_idx(
                billing_start_date,
                billing_day,
                invoice_date,
                &billing_period,
            );

            let advance_period = calculate_period_range(
                billing_start_date,
                billing_day,
                period_idx,
                &billing_period,
            );

            let arrear_period = if period_idx == 0 {
                None
            } else {
                Some(calculate_period_range(
                    billing_start_date,
                    billing_day,
                    period_idx - 1,
                    &billing_period,
                ))
            };

            let proration_factor = if period_idx == 0 {
                calculate_proration_factor(&advance_period)
            } else {
                None
            };

            Some(ComponentPeriods {
                proration_factor,
                advance: advance_period,
                arrear: arrear_period,
            })
        }
    }
}

fn calculate_proration_factor(period: &Period) -> Option<f64> {
    let days_in_period = period.end.signed_duration_since(period.start).num_days() as u64; // +1 ?
    let days_in_month_from = period.start.days_in_month() as u64;
    let days_in_month_to = period.end.days_in_month() as u64;

    // if from is end of month and from.day <= to.day. Ex: 2023-02-28 -> 2023-03-28+
    if period.start.day() == days_in_month_from as u32 && period.end.day() >= period.start.day() {
        return None;
    }

    if days_in_period >= days_in_month_from {
        return None;
    }

    // if to is end of month and from.day >= to.day. Ex: 2023-01-28+ -> 2023-02-28
    if period.end.day() == days_in_month_to as u32 && period.start.day() >= period.end.day() {
        return None;
    }

    let proration_factor = days_in_period as f64 / days_in_month_from as f64;

    Some(proration_factor)
}

fn applies_this_period(
    billing_start_date: NaiveDate,
    invoice_date: NaiveDate,
    billing_period: &SubscriptionFeeBillingPeriod,
) -> bool {
    let month_elapsed = (invoice_date.year() - billing_start_date.year()) * 12
        + (invoice_date.month() as i32)
        - (billing_start_date.month() as i32);
    let applies = month_elapsed % billing_period.as_months() == 0;
    applies
}

pub fn calculate_period_range(
    billing_start_date: NaiveDate,
    billing_day: u32,
    period_index: i32,
    billing_period: &BillingPeriodEnum,
) -> Period {
    let months_in_period = billing_period.as_months();

    let start_day_after_billing_day = billing_start_date.day() >= billing_day;

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

    Period {
        start: period_start,
        end: period_end,
    }
}

fn calculate_period_idx(
    billing_start_date: NaiveDate,
    billing_day: u32,
    invoice_date: NaiveDate,
    billing_period: &BillingPeriodEnum,
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
        BillingPeriodEnum::Monthly => month_diff + day_adjustment,
        BillingPeriodEnum::Quarterly => {
            // Quarterly billing divides the month difference by 3 (since a quarter is 3 months)
            (month_diff / 3) + day_adjustment
        }
        BillingPeriodEnum::Annual => {
            // Annual billing divides the month difference by 12 (since a year is 12 months)
            (month_diff / 12) + day_adjustment
        }
    }
}

fn add_months_at_billing_day(
    date: NaiveDate,
    months_to_add: u32,
    billing_day: u32,
) -> Option<NaiveDate> {
    date.checked_add_months(Months::new(months_to_add as u32))
        .and_then(|d| d.with_day(d.days_in_month().min(billing_day)))
}

#[cfg(test)]
mod test {
    use super::{calculate_period_idx, calculate_period_range};
    use crate::domain::enums::BillingPeriodEnum;

    use chrono::NaiveDate;
    use rstest::rstest;

    #[rstest]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        1,
        0,
        "2021-01-01",
        "2021-02-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        1,
        1,
        "2021-02-01",
        "2021-03-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        1,
        2,
        "2021-03-01",
        "2021-04-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-10",
        1,
        0,
        "2021-01-10",
        "2021-02-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-10",
        1,
        1,
        "2021-02-01",
        "2021-03-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-10",
        1,
        2,
        "2021-03-01",
        "2021-04-01"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        10,
        0,
        "2021-01-01",
        "2021-01-10"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        10,
        1,
        "2021-01-10",
        "2021-02-10"
    )]
    #[case(
        BillingPeriodEnum::Monthly,
        "2021-01-01",
        10,
        2,
        "2021-02-10",
        "2021-03-10"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-10",
        1,
        0,
        "2021-01-10",
        "2021-04-01"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-10",
        1,
        1,
        "2021-04-01",
        "2021-07-01"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-10",
        1,
        2,
        "2021-07-01",
        "2021-10-01"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-01",
        10,
        0,
        "2021-01-01",
        "2021-01-10"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-01",
        10,
        1,
        "2021-01-10",
        "2021-04-10"
    )]
    #[case(
        BillingPeriodEnum::Quarterly,
        "2021-01-01",
        10,
        2,
        "2021-04-10",
        "2021-07-10"
    )]
    #[case(
        BillingPeriodEnum::Annual,
        "2021-01-10",
        1,
        0,
        "2021-01-10",
        "2022-01-01"
    )]
    #[case(
        BillingPeriodEnum::Annual,
        "2021-01-10",
        1,
        1,
        "2022-01-01",
        "2023-01-01"
    )]
    #[case(
        BillingPeriodEnum::Annual,
        "2021-01-10",
        1,
        2,
        "2023-01-01",
        "2024-01-01"
    )]
    #[trace]
    fn test_calculate_period_range(
        #[case] billing_period: BillingPeriodEnum,
        #[case] billing_start_date: NaiveDate,
        #[case] billing_day: u32,
        #[case] period_idx: i32,
        #[case] expected_period_start: NaiveDate,
        #[case] expected_period_end: NaiveDate,
    ) {
        let super::Period { start, end } =
            calculate_period_range(billing_start_date, billing_day, period_idx, &billing_period);
        assert_eq!(start, expected_period_start);
        assert_eq!(end, expected_period_end);
    }

    #[rstest]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 10, "2021-02-20", 2)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 10, "2022-01-02", 12)]
    // test Monthly with billing_start_date.day() after billing_day
    #[case(BillingPeriodEnum::Monthly, "2021-01-10", 1, "2021-01-31", 0)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-10", 1, "2021-02-01", 1)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-10", 1, "2021-02-28", 1)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 1, "2021-01-31", 0)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 1, "2021-02-01", 1)]
    #[case(BillingPeriodEnum::Monthly, "2021-01-01", 1, "2021-02-28", 1)]
    #[case(BillingPeriodEnum::Quarterly, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriodEnum::Quarterly, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriodEnum::Quarterly, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriodEnum::Quarterly, "2021-01-01", 10, "2022-01-01", 4)]
    #[case(BillingPeriodEnum::Quarterly, "2021-01-01", 10, "2022-01-12", 5)]
    #[case(BillingPeriodEnum::Annual, "2021-01-01", 10, "2021-01-02", 0)]
    #[case(BillingPeriodEnum::Annual, "2021-01-01", 10, "2021-01-10", 1)]
    #[case(BillingPeriodEnum::Annual, "2021-01-01", 10, "2021-01-12", 1)]
    #[case(BillingPeriodEnum::Annual, "2021-01-01", 10, "2022-01-01", 1)]
    #[case(BillingPeriodEnum::Annual, "2021-01-01", 10, "2022-01-12", 2)]
    #[trace]
    fn test_calculate_period_idx(
        #[case] billing_period: BillingPeriodEnum,
        #[case] billing_start_date: NaiveDate,
        #[case] billing_day: u32,
        #[case] current_date: NaiveDate,
        #[case] expected_period_idx: i32,
    ) {
        let period_idx = calculate_period_idx(
            billing_start_date,
            billing_day,
            current_date,
            &billing_period,
        );
        assert_eq!(period_idx, expected_period_idx);
    }
}

use chrono::{Duration, NaiveDate};
use ndarray::Array;
use ndarray::Array1;
use ndarray_interp::interp1d;
use rand::distributions::Distribution;
use rand::Rng;
use rand_distr::Normal;

pub fn generate_smooth_growth(
    start_date: NaiveDate,
    end_date: NaiveDate,
    end_total: u64,
    growth_curve: Vec<f64>,
    randomness_factor: f64,
) -> Vec<(NaiveDate, u64, u64)> {
    let total_days = end_date.signed_duration_since(start_date).num_days() as usize + 1;
    let mut dates = Vec::with_capacity(total_days);
    let mut date = start_date;
    for _ in 0..total_days {
        dates.push(date);
        date += Duration::days(1);
    }

    let growth_curve = Array1::from_vec(growth_curve);
    let curve_points = Array::linspace(0.0, (total_days - 1) as f64, growth_curve.len());
    let interpolator = interp1d::Interp1DBuilder::new(growth_curve)
        .strategy(interp1d::CubicSpline)
        .x(curve_points)
        .build()
        .unwrap();

    let query = Array::linspace(0.0, (total_days - 1) as f64, total_days);
    let interpolated_curve: Vec<f64> = interpolator
        .interp_array(&query)
        .expect("Failed to interpolate")
        .into_iter()
        .collect();

    let curve_sum: f64 = interpolated_curve.iter().sum();
    let mut daily_count: Vec<f64> = interpolated_curve
        .iter()
        .map(|x| (x / curve_sum) * end_total as f64)
        .collect();

    let mut rng = rand::thread_rng();
    let normal = Normal::new(1.0, randomness_factor).unwrap();
    for customer in &mut daily_count {
        *customer *= normal.sample(&mut rng);
        *customer = customer.max(0.0).round();
    }

    let mut daily_count_random: Vec<u64> = daily_count.iter().map(|x| x.floor() as u64).collect();

    // this makes sure that the sum of daily customer match what we provide as argument, by adding or removing customers from random days
    let mut discrepancy = end_total as i64 - daily_count_random.iter().sum::<u64>() as i64;
    while discrepancy != 0 {
        let index = rng.gen_range(0..total_days);
        if discrepancy > 0 {
            daily_count_random[index] += 1;
            discrepancy -= 1;
        } else if daily_count_random[index] > 0 {
            daily_count_random[index] -= 1;
            discrepancy += 1;
        }
    }

    let mut result: Vec<(NaiveDate, u64, u64)> = Vec::with_capacity(total_days);
    for i in 0..total_days {
        let new_customers = daily_count_random[i];
        let total_customers = if i == 0 {
            new_customers
        } else {
            result[i - 1].2 + new_customers
        };
        result.push((dates[i], new_customers, total_customers));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_generate_smooth_growth() {
        let start_date = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();
        let end_date = NaiveDate::from_ymd_opt(2021, 12, 31).unwrap();
        let total_customers = 1000;
        let growth_curve = vec![5.0, 10.0, 20.0, 1000.0];
        let randomness_factor = 0.5;
        let result = generate_smooth_growth(
            start_date,
            end_date,
            total_customers,
            growth_curve,
            randomness_factor,
        );

        let print = true;
        if !!print {
            println!("Date                 | New Customers | Total Customers");
            for (date, new_customers, total_customers) in &result {
                println!(
                    "{:?} | {:<13} | {:<15}",
                    date, new_customers, total_customers
                );
            }
        }

        assert_eq!(result.len(), 365);
        assert_eq!(result[0].0, start_date);
        assert_eq!(result[364].0, end_date);
        assert_eq!(result[364].2, total_customers);
    }
}

use chrono::{Days, NaiveDateTime, NaiveTime};
use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionStatus;
use meteroid_store::domain::outbox_event::SubscriptionEvent;

pub trait DbSubscriptionExt {
    fn status_proto(&self) -> SubscriptionStatus;
}

impl DbSubscriptionExt for meteroid_store::domain::Subscription {
    fn status_proto(&self) -> SubscriptionStatus {
        derive_subscription_status_chrono(
            chrono::Utc::now().naive_utc(),
            self.trial_duration,
            self.activated_at,
            self.canceled_at,
            self.start_date,
            self.end_date,
        )
    }
}

impl DbSubscriptionExt for SubscriptionEvent {
    fn status_proto(&self) -> SubscriptionStatus {
        derive_subscription_status_chrono(
            chrono::Utc::now().naive_utc(),
            self.trial_duration,
            self.activated_at,
            self.canceled_at,
            self.start_date,
            self.end_date,
        )
    }
}

fn derive_subscription_status_chrono(
    timestamp: NaiveDateTime,
    trial_duration: Option<u32>,
    activated_at: Option<NaiveDateTime>,
    canceled_at: Option<NaiveDateTime>,
    start_date: chrono::NaiveDate,
    end_date: Option<chrono::NaiveDate>,
) -> SubscriptionStatus {
    let start_date = start_date.and_time(NaiveTime::MIN);
    let end_date = end_date
        .and_then(|x| NaiveTime::from_hms_milli_opt(23, 59, 59, 999).map(|y| x.and_time(y)));

    let trial_end_date =
        trial_duration.and_then(|x| start_date.checked_add_days(Days::new(x as u64)));

    // activated = paid or considered as paid. If not activated, then the trial fallback applies
    // trial

    if start_date > timestamp {
        return SubscriptionStatus::Pending;
    }
    if canceled_at.is_some() {
        return SubscriptionStatus::Canceled;
    }
    if end_date.is_some() && timestamp > end_date.unwrap() {
        return SubscriptionStatus::Ended;
    }

    match trial_end_date {
        // no trial, so it's either pending or active
        None => match activated_at {
            Some(activated_at) if activated_at <= timestamp => SubscriptionStatus::Active,
            _ => SubscriptionStatus::Pending,
        },
        // trial. It either still in trial, expired if not activated yet, or active
        Some(trial_end_date) => {
            if trial_end_date < timestamp {
                match activated_at {
                    Some(activated_at) if activated_at <= timestamp => SubscriptionStatus::Active,
                    _ => SubscriptionStatus::TrialExpired,
                }
            } else {
                SubscriptionStatus::Trialing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};
    use rstest::rstest;
    use std::str::FromStr;

    #[rstest]
    #[case(
        SubscriptionStatus::Pending,
        "2024-01-01T00:00:00",
        None,
        None,
        None,
        "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Pending,
        "2024-01-02T00:00:00",
        None,
        None,
        None,
        "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Pending,
        "2024-01-02T00:00:00",
        None,
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
        "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Pending,
        "2024-01-03T00:00:00",
        Some(3),
        None,
        None,
        "2024-01-04",
        None
    )]
    #[case(
        SubscriptionStatus::Trialing,
        "2024-01-02T00:00:00",
        Some(3),
        None,
        None,
        "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Trialing,
    "2024-01-02T00:00:00",
        Some(3),
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
    "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Active,
    "2024-01-03T00:00:00",
        Some(1), // TODO review if trial(2) should end at 2024-01-03T00:00:00 or 2024-01-03T23:59:59
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
    "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::TrialExpired,
        "2024-01-03T01:00:00",
        Some(1),
        Some(NaiveDateTime::from_str("2024-01-05T00:00:00").unwrap()),
        None,
        "2024-01-01",
        None
    )]
    #[case(
        SubscriptionStatus::Active,
    "2024-01-10T23:00:00",
        Some(3),
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
    "2024-01-03",
        Some(NaiveDate::from_str("2024-01-10").unwrap()),
    )]
    #[case(
        SubscriptionStatus::Ended,
        "2024-01-11T00:00:00",
        Some(3),
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
        "2024-01-03",
        Some(NaiveDate::from_str("2024-01-10").unwrap()),
    )]
    #[case(
        SubscriptionStatus::Canceled,
    "2024-01-11T23:00:00",
        Some(3),
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        Some(NaiveDateTime::from_str("2024-01-08T10:00:20").unwrap()),
    "2024-01-03",
        Some(NaiveDate::from_str("2024-01-10").unwrap()),
    )]
    #[case(
        SubscriptionStatus::Ended,
    "2024-01-11T23:00:00",
        Some(3),
        Some(NaiveDateTime::from_str("2024-01-03T00:00:00").unwrap()),
        None,
    "2024-01-03",
        Some(NaiveDate::from_str("2024-01-10").unwrap()),
    )]
    #[trace]
    fn test_derive_subscription_status(
        #[case] expected_status: SubscriptionStatus,
        #[case] timestamp: NaiveDateTime,
        #[case] trial_duration: Option<u32>,
        #[case] activated_at: Option<NaiveDateTime>,
        #[case] canceled_at: Option<NaiveDateTime>,
        #[case] start_date: NaiveDate,
        #[case] billing_end_date: Option<NaiveDate>,
    ) {
        let status = derive_subscription_status_chrono(
            timestamp,
            trial_duration,
            activated_at,
            canceled_at,
            start_date,
            billing_end_date,
        );

        assert_eq!(status, expected_status);
    }
}

use crate::datetime::{chrono_utc_now, DateExt, TimeExt};
use crate::mapping::MappingError;
use chrono::{NaiveDateTime, NaiveTime};
use meteroid_grpc::meteroid::api::subscriptions::v1_2::SubscriptionStatus;
use meteroid_repository::subscriptions::{Subscription, SubscriptionList};
use time::PrimitiveDateTime;

pub trait DbSubscriptionExt {
    fn status_proto(&self) -> Result<SubscriptionStatus, MappingError>;
}

impl DbSubscriptionExt for Subscription {
    fn status_proto(&self) -> Result<SubscriptionStatus, MappingError> {
        derive_subscription_status(
            chrono_utc_now().naive_utc(),
            self.trial_start_date,
            self.activated_at,
            self.canceled_at,
            self.billing_start_date,
            self.billing_end_date,
        )
    }
}

impl DbSubscriptionExt for SubscriptionList {
    fn status_proto(&self) -> Result<SubscriptionStatus, MappingError> {
        derive_subscription_status(
            chrono_utc_now().naive_utc(),
            self.trial_start_date,
            self.activated_at,
            self.canceled_at,
            self.billing_start_date,
            self.billing_end_date,
        )
    }
}

impl DbSubscriptionExt for meteroid_store::domain::Subscription {
    fn status_proto(&self) -> Result<SubscriptionStatus, MappingError> {
        derive_subscription_status_chrono(
            chrono_utc_now().naive_utc(),
            self.trial_start_date,
            self.activated_at,
            self.canceled_at,
            self.billing_start_date,
            self.billing_end_date,
        )
    }
}

impl DbSubscriptionExt for meteroid_store::domain::SubscriptionDetails {
    fn status_proto(&self) -> Result<SubscriptionStatus, MappingError> {
        derive_subscription_status_chrono(
            chrono_utc_now().naive_utc(),
            self.trial_start_date,
            self.activated_at,
            self.canceled_at,
            self.billing_start_date,
            self.billing_end_date,
        )
    }
}

fn derive_subscription_status_chrono(
    timestamp: NaiveDateTime,
    trial_start_date: Option<chrono::NaiveDate>,
    activated_at: Option<chrono::NaiveDateTime>,
    canceled_at: Option<chrono::NaiveDateTime>,
    billing_start_date: chrono::NaiveDate,
    billing_end_date: Option<chrono::NaiveDate>,
) -> Result<SubscriptionStatus, MappingError> {
    let trial_start_date = trial_start_date.map(|x| x.and_time(NaiveTime::MIN));
    let billing_start_date = billing_start_date.and_time(NaiveTime::MIN);
    let billing_end_date = billing_end_date
        .and_then(|x| NaiveTime::from_hms_opt(23, 59, 59).map(|y| x.and_time(y)))
        .unwrap_or(NaiveDateTime::MAX);

    match (trial_start_date, activated_at, canceled_at) {
        (None, None, _) => Ok(SubscriptionStatus::Pending),
        (Some(_), Some(active_at), _) if active_at > timestamp => Ok(SubscriptionStatus::Trial),
        (_, Some(active_at), _) if active_at > timestamp => Ok(SubscriptionStatus::Pending),
        (_, Some(active_at), _) if active_at <= timestamp && timestamp <= billing_end_date => {
            Ok(SubscriptionStatus::Active)
        }
        (_, Some(_), _) if canceled_at.is_some() => Ok(SubscriptionStatus::Canceled),
        (_, Some(_), _) => Ok(SubscriptionStatus::Ended),
        (Some(trial_start_date), _, _) => {
            if trial_start_date <= timestamp && timestamp <= billing_start_date {
                Ok(SubscriptionStatus::Trial)
            } else {
                Ok(SubscriptionStatus::Pending)
            }
        }
    }
}

fn derive_subscription_status(
    timestamp: NaiveDateTime,
    trial_start_date: Option<time::Date>,
    activated_at: Option<PrimitiveDateTime>,
    canceled_at: Option<PrimitiveDateTime>,
    billing_start_date: time::Date,
    billing_end_date: Option<time::Date>,
) -> Result<SubscriptionStatus, MappingError> {
    let trial_start_date = trial_start_date
        .map(|x| x.to_chrono())
        .transpose()?
        .map(|x| x.and_time(NaiveTime::MIN));
    let activated_at = activated_at.map(|x| x.to_chrono()).transpose()?;
    let canceled_at = canceled_at.map(|x| x.to_chrono()).transpose()?;
    let billing_start_date = billing_start_date.to_chrono()?.and_time(NaiveTime::MIN);
    let billing_end_date = billing_end_date
        .map(|x| x.to_chrono())
        .transpose()?
        .and_then(|x| NaiveTime::from_hms_opt(23, 59, 59).map(|y| x.and_time(y)))
        .unwrap_or(NaiveDateTime::MAX);

    match (trial_start_date, activated_at, canceled_at) {
        (None, None, _) => Ok(SubscriptionStatus::Pending),
        (Some(_), Some(active_at), _) if active_at > timestamp => Ok(SubscriptionStatus::Trial),
        (_, Some(active_at), _) if active_at > timestamp => Ok(SubscriptionStatus::Pending),
        (_, Some(active_at), _) if active_at <= timestamp && timestamp <= billing_end_date => {
            Ok(SubscriptionStatus::Active)
        }
        (_, Some(_), _) if canceled_at.is_some() => Ok(SubscriptionStatus::Canceled),
        (_, Some(_), _) => Ok(SubscriptionStatus::Ended),
        (Some(trial_start_date), _, _) => {
            if trial_start_date <= timestamp && timestamp <= billing_start_date {
                Ok(SubscriptionStatus::Trial)
            } else {
                Ok(SubscriptionStatus::Pending)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use time::macros::{date, datetime};

    #[rstest]
    #[case(
    SubscriptionStatus::Pending,
    "2024-01-01T00:00:00",
    None,
    None,
    None,
    date ! (2024 - 01 - 01),
    None
    )]
    #[case(
    SubscriptionStatus::Pending,
    "2024-01-02T00:00:00",
    None,
    None,
    None,
    date ! (2024 - 01 - 01),
    None
    )]
    #[case(
    SubscriptionStatus::Pending,
    "2024-01-02T00:00:00",
    None,
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    None,
    date ! (2024 - 01 - 01),
    None
    )]
    #[case(
    SubscriptionStatus::Pending,
    "2024-01-04T00:00:00",
    Some(date ! (2024 - 01 - 01)),
    None,
    None,
    date ! (2024 - 01 - 03),
    None
    )]
    #[case(
    SubscriptionStatus::Trial,
    "2024-01-02T00:00:00",
    Some(date ! (2024 - 01 - 01)),
    None,
    None,
    date ! (2024 - 01 - 03),
    None
    )]
    #[case(
    SubscriptionStatus::Trial,
    "2024-01-02T00:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    None,
    date ! (2024 - 01 - 03),
    None
    )]
    #[case(
    SubscriptionStatus::Active,
    "2024-01-03T00:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    None,
    date ! (2024 - 01 - 03),
    None
    )]
    #[case(
    SubscriptionStatus::Active,
    "2024-01-10T23:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    None,
    date ! (2024 - 01 - 03),
    Some(date ! (2024 - 01 - 10)),
    )]
    #[case(
    SubscriptionStatus::Active,
    "2024-01-10T23:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    Some(datetime ! (2024 - 01 - 08 10: 00: 20)),
    date ! (2024 - 01 - 03),
    Some(date ! (2024 - 01 - 10)),
    )]
    #[case(
    SubscriptionStatus::Canceled,
    "2024-01-11T23:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    Some(datetime ! (2024 - 01 - 08 10: 00: 20)),
    date ! (2024 - 01 - 03),
    Some(date ! (2024 - 01 - 10)),
    )]
    #[case(
    SubscriptionStatus::Ended,
    "2024-01-11T23:00:00",
    Some(date ! (2024 - 01 - 01)),
    Some(datetime ! (2024 - 01 - 03 00: 00: 00)),
    None,
    date ! (2024 - 01 - 03),
    Some(date ! (2024 - 01 - 10)),
    )]
    #[trace]
    fn test_derive_subscription_status(
        #[case] expected_status: SubscriptionStatus,
        #[case] timestamp: NaiveDateTime,
        #[case] trial_start_date: Option<time::Date>,
        #[case] activated_at: Option<PrimitiveDateTime>,
        #[case] canceled_at: Option<PrimitiveDateTime>,
        #[case] billing_start_date: time::Date,
        #[case] billing_end_date: Option<time::Date>,
    ) {
        let status = derive_subscription_status(
            timestamp,
            trial_start_date,
            activated_at,
            canceled_at,
            billing_start_date,
            billing_end_date,
        )
            .unwrap();

        assert_eq!(status, expected_status);
    }
}

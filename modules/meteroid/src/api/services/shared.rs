pub mod mapping {
    pub mod datetime {
        use time::PrimitiveDateTime;
        pub fn datetime_to_timestamp(dt: PrimitiveDateTime) -> prost_types::Timestamp {
            prost_types::Timestamp {
                seconds: dt.assume_utc().unix_timestamp(),
                nanos: dt.nanosecond() as i32,
            }
        }
        pub fn offset_datetime_to_timestamp(dt: time::OffsetDateTime) -> prost_types::Timestamp {
            datetime_to_timestamp(PrimitiveDateTime::new(dt.date(), dt.time()))
        }
    }
    pub mod date {
        use common_grpc::meteroid::common::v1::Date;

        pub fn from_proto(d: Date) -> Result<time::Date, time::error::ComponentRange> {
            let month: time::Month = (d.month as u8).try_into()?;

            time::Date::from_calendar_date(d.year, month, d.day as u8)
        }

        pub fn to_proto(d: time::Date) -> Date {
            Date {
                year: d.year(),
                month: (d.month() as u8).into(),
                day: d.day().into(),
            }
        }
    }

    pub mod period {
        use meteroid_grpc::meteroid::api::shared::v1 as shared_grpc;

        pub fn billing_period_to_server(
            freq: &meteroid_repository::BillingPeriodEnum,
        ) -> shared_grpc::BillingPeriod {
            match freq {
                meteroid_repository::BillingPeriodEnum::MONTHLY => {
                    shared_grpc::BillingPeriod::Monthly
                }
                meteroid_repository::BillingPeriodEnum::QUARTERLY => {
                    shared_grpc::BillingPeriod::Quarterly
                }
                meteroid_repository::BillingPeriodEnum::ANNUAL => {
                    shared_grpc::BillingPeriod::Annual
                }
            }
        }

        pub fn billing_period_to_db(
            freq: &shared_grpc::BillingPeriod,
        ) -> meteroid_repository::BillingPeriodEnum {
            match freq {
                shared_grpc::BillingPeriod::Monthly => {
                    meteroid_repository::BillingPeriodEnum::MONTHLY
                }
                shared_grpc::BillingPeriod::Annual => {
                    meteroid_repository::BillingPeriodEnum::ANNUAL
                }
                shared_grpc::BillingPeriod::Quarterly => {
                    meteroid_repository::BillingPeriodEnum::QUARTERLY
                }
            }
        }
    }
}

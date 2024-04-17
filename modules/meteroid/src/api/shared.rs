pub mod mapping {
    pub mod datetime {
        use chrono::NaiveDateTime;
        use time::PrimitiveDateTime;

        pub fn datetime_to_timestamp(dt: PrimitiveDateTime) -> prost_types::Timestamp {
            prost_types::Timestamp {
                seconds: dt.assume_utc().unix_timestamp(),
                nanos: dt.nanosecond() as i32,
            }
        }

        pub fn chrono_to_timestamp(dt: NaiveDateTime) -> prost_types::Timestamp {
            prost_types::Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }
        }

        pub fn offset_datetime_to_timestamp(dt: time::OffsetDateTime) -> prost_types::Timestamp {
            datetime_to_timestamp(PrimitiveDateTime::new(dt.date(), dt.time()))
        }
    }

    pub mod date {
        use chrono::Datelike;
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

        pub fn chrono_to_proto(d: chrono::NaiveDate) -> Date {
            Date {
                year: d.year(),
                month: d.month(),
                day: d.day(),
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

// v2 conversions, we should now encode dates/decimals etc as string
pub mod conversions {
    use std::str::FromStr;

    //  custom error instead of status ?

    pub trait ProtoConv<T> {
        fn as_proto(&self) -> T;
        fn from_proto(proto: T) -> Result<Self, tonic::Status>
        where
            Self: Sized,
        {
            Self::from_proto_ref(&proto)
        }
        fn from_proto_ref(proto: &T) -> Result<Self, tonic::Status>
        where
            Self: Sized;
    }

    pub trait AsProtoOpt<T> {
        fn as_proto(&self) -> Option<T>
        where
            Self: Sized;
    }

    pub trait FromProtoOpt<T>: ProtoConv<T> {
        fn from_proto_opt(proto: Option<T>) -> Result<Option<Self>, tonic::Status>
        where
            Self: Sized;
    }

    impl<T, U> AsProtoOpt<T> for Option<U>
    where
        U: ProtoConv<T>,
    {
        fn as_proto(&self) -> Option<T> {
            self.as_ref().map(|d| d.as_proto())
        }
    }

    impl<T, U> FromProtoOpt<T> for U
    where
        U: ProtoConv<T>,
    {
        fn from_proto_opt(proto: Option<T>) -> Result<Option<Self>, tonic::Status> {
            proto.map(U::from_proto).transpose()
        }
    }

    impl ProtoConv<String> for chrono::NaiveDate {
        fn as_proto(&self) -> String {
            self.format("%Y-%m-%d").to_string()
        }

        fn from_proto(proto: String) -> Result<Self, tonic::Status> {
            Self::from_proto_ref(&proto)
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            chrono::NaiveDate::parse_from_str(proto, "%Y-%m-%d")
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid date: {}", e)))
        }
    }

    impl ProtoConv<String> for chrono::NaiveDateTime {
        fn as_proto(&self) -> String {
            self.format("%Y-%m-%dT%H:%M:%S").to_string()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            chrono::NaiveDateTime::parse_from_str(proto, "%Y-%m-%dT%H:%M:%S")
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid datetime: {}", e)))
        }
    }

    impl ProtoConv<String> for rust_decimal::Decimal {
        fn as_proto(&self) -> String {
            self.to_string()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            rust_decimal::Decimal::from_str(&proto)
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid decimal: {}", e)))
        }
    }

    // TODO disable completely uuid in frontend ?
    impl ProtoConv<String> for uuid::Uuid {
        fn as_proto(&self) -> String {
            self.to_string()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            uuid::Uuid::parse_str(&proto)
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid uuid: {}", e)))
        }
    }
}

pub mod platform_admin {
    use common_grpc::middleware::server::auth::RequestExt;
    use meteroid_store::Store;
    use tonic::{Request, Status};
    use uuid::Uuid;

    pub fn require_platform_admin<T>(request: &Request<T>, store: &Store) -> Result<Uuid, Status> {
        let org_id = request.organization()?;
        let actor = request.actor()?;
        match store.settings.admin_organization {
            Some(admin_org) if admin_org == org_id => Ok(actor),
            Some(_) => Err(Status::permission_denied("Platform admin access required")),
            None => Err(Status::permission_denied(
                "Platform admin is not configured (ADMIN_ORGANIZATION_ID not set)",
            )),
        }
    }
}

// v2 conversions, we should now encode dates/decimals etc as string
pub mod conversions {

    use std::str::FromStr;

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
            self.as_ref().map(ProtoConv::as_proto)
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

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            chrono::NaiveDate::parse_from_str(proto, "%Y-%m-%d")
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid date: {e}")))
        }
    }

    impl ProtoConv<String> for chrono::NaiveDateTime {
        fn as_proto(&self) -> String {
            self.and_utc().to_rfc3339()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            chrono::DateTime::parse_from_rfc3339(proto)
                .map(|dt| dt.naive_utc())
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(proto, "%Y-%m-%dT%H:%M:%S"))
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid datetime: {e}")))
        }
    }

    impl ProtoConv<String> for chrono::DateTime<chrono::Utc> {
        fn as_proto(&self) -> String {
            self.to_rfc3339()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            chrono::DateTime::parse_from_rfc3339(proto)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid datetime: {e}")))
        }
    }

    impl ProtoConv<String> for rust_decimal::Decimal {
        fn as_proto(&self) -> String {
            self.to_string()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            rust_decimal::Decimal::from_str(proto)
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid decimal: {e}")))
        }
    }

    // TODO disable completely uuid in frontend ?
    impl ProtoConv<String> for uuid::Uuid {
        fn as_proto(&self) -> String {
            self.to_string()
        }

        fn from_proto_ref(proto: &String) -> Result<Self, tonic::Status> {
            uuid::Uuid::parse_str(proto)
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid uuid: {e}")))
        }
    }
}

pub mod usage {
    use crate::api::shared::conversions::ProtoConv;
    use meteroid_store::domain::{Period, Subscription};

    pub fn resolve_usage_period(
        start_date: Option<&String>,
        end_date: Option<&String>,
        subscription: &Subscription,
    ) -> Result<Period, tonic::Status> {
        match (start_date, end_date) {
            (Some(s), Some(e)) => Ok(Period {
                start: chrono::NaiveDate::from_proto_ref(s)?,
                end: chrono::NaiveDate::from_proto_ref(e)?,
            }),
            _ => Ok(Period {
                start: subscription.current_period_start,
                end: subscription
                    .current_period_end
                    .unwrap_or_else(|| chrono::Utc::now().date_naive() + chrono::Duration::days(1)),
            }),
        }
    }
}

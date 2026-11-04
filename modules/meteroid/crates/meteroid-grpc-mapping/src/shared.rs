/// Parse a JSON string field, mapping malformed input to `InvalidArgument`.
/// Shared by the segments / saved-views / alerts services.
pub fn parse_json(field: &str, value: &str) -> Result<serde_json::Value, tonic::Status> {
    serde_json::from_str(value)
        .map_err(|e| tonic::Status::invalid_argument(format!("invalid JSON in `{field}`: {e}")))
}

pub mod mapping {
    pub mod datetime {
        use chrono::{DateTime, NaiveDateTime};
        use prost_types::Timestamp;
        use time::PrimitiveDateTime;

        pub fn datetime_to_timestamp(dt: PrimitiveDateTime) -> prost_types::Timestamp {
            prost_types::Timestamp {
                seconds: dt.assume_utc().unix_timestamp(),
                nanos: dt.nanosecond() as i32,
            }
        }

        pub fn chrono_to_timestamp(dt: NaiveDateTime) -> prost_types::Timestamp {
            prost_types::Timestamp {
                seconds: dt.and_utc().timestamp(),
                nanos: dt.and_utc().timestamp_subsec_nanos() as i32,
            }
        }

        pub fn offset_datetime_to_timestamp(dt: time::OffsetDateTime) -> prost_types::Timestamp {
            datetime_to_timestamp(PrimitiveDateTime::new(dt.date(), dt.time()))
        }

        pub fn chrono_from_timestamp(t: Timestamp) -> Result<NaiveDateTime, tonic::Status> {
            DateTime::from_timestamp(t.seconds, t.nanos as u32)
                .map(|x| x.naive_utc())
                .ok_or(tonic::Status::invalid_argument("Invalid proto timestamp"))
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

        pub fn chrono_from_proto(d: Date) -> Option<chrono::NaiveDate> {
            chrono::NaiveDate::from_ymd_opt(d.year, d.month, d.day)
        }
    }
}

pub mod platform_admin {
    use common_domain::ids::{BaseId, OrganizationId};
    use common_grpc::middleware::server::auth::RequestExt;
    use meteroid_store::Store;
    use tonic::{Request, Status};
    use uuid::Uuid;

    pub fn require_platform_admin<T>(request: &Request<T>, store: &Store) -> Result<Uuid, Status> {
        check_platform_admin(request, store.settings.admin_organization)
    }

    /// An admin of the configured admin organization, signed in as a user (not an API key).
    fn check_platform_admin<T>(
        request: &Request<T>,
        admin_org: Option<OrganizationId>,
    ) -> Result<Uuid, Status> {
        let admin_org = admin_org.ok_or_else(|| {
            Status::permission_denied(
                "Platform admin is not configured (ADMIN_ORGANIZATION_ID not set)",
            )
        })?;
        let denied = || Status::permission_denied("Platform admin access required");
        if request.organization()? != admin_org {
            return Err(denied());
        }
        let actor = request.actor_user().map_err(|_| denied())?;
        request.require_admin().map_err(|_| denied())?;
        Ok(actor.as_uuid())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use common_domain::auth::OrgMemberRole;
        use common_domain::ids::{ApiTokenId, TenantId, UserId};
        use common_grpc::middleware::server::AuthorizedState;
        use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor, TenantEnv};
        use tonic::Code;

        fn request(state: AuthorizedState) -> Request<()> {
            let mut request = Request::new(());
            request.extensions_mut().insert(state);
            request
        }

        fn org_user(org: OrganizationId, role: OrgMemberRole) -> AuthorizedState {
            AuthorizedState::Organization {
                user_id: UserId::new(),
                organization_id: org,
                role,
            }
        }

        fn tenant(org: OrganizationId, actor: TenantActor) -> AuthorizedState {
            AuthorizedState::Tenant(AuthorizedAsTenant {
                actor,
                tenant_id: TenantId::new(),
                organization_id: org,
                tenant_env: TenantEnv::NonProduction,
            })
        }

        #[test]
        fn admin_of_admin_org_is_allowed() {
            let admin_org = OrganizationId::new();
            let user = UserId::new();
            let state = tenant(
                admin_org,
                TenantActor::User {
                    id: user,
                    role: OrgMemberRole::Admin,
                },
            );
            assert_eq!(
                check_platform_admin(&request(state), Some(admin_org)).unwrap(),
                user.as_uuid()
            );
            let state = org_user(admin_org, OrgMemberRole::Admin);
            assert!(check_platform_admin(&request(state), Some(admin_org)).is_ok());
        }

        #[test]
        fn non_admins_are_denied() {
            let admin_org = OrganizationId::new();
            let denied = [
                org_user(admin_org, OrgMemberRole::Member),
                org_user(OrganizationId::new(), OrgMemberRole::Admin),
                tenant(admin_org, TenantActor::ApiKey(ApiTokenId::new())),
            ];
            for state in denied {
                let err = check_platform_admin(&request(state), Some(admin_org)).unwrap_err();
                assert_eq!(err.code(), Code::PermissionDenied);
            }

            let state = org_user(admin_org, OrgMemberRole::Admin);
            let err = check_platform_admin(&request(state), None).unwrap_err();
            assert_eq!(err.code(), Code::PermissionDenied);
        }
    }
}

// v2 conversions, we should now encode dates/decimals etc as string

pub mod usage {
    use meteroid_app::conversions::ProtoConv;
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

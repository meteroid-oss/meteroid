pub mod applied {
    use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::coupons::v1 as server;
    use meteroid_store::domain;

    pub struct AppliedCouponForDisplayWrapper(pub server::AppliedCouponForDisplay);
    impl From<domain::AppliedCouponForDisplay> for AppliedCouponForDisplayWrapper {
        fn from(value: domain::AppliedCouponForDisplay) -> Self {
            Self(server::AppliedCouponForDisplay {
                id: value.id.as_proto(),
                coupon_id: value.coupon_id.as_proto(),
                customer_name: value.customer_name,
                customer_local_id: value.customer_id.as_proto(),
                customer_id: value.customer_id.as_proto(),
                subscription_id: value.subscription_id.as_proto(),
                plan_name: value.plan_name,
                plan_local_id: value.plan_id.as_proto(),
                plan_version: value.plan_version,
                is_active: value.is_active,
                applied_amount: value.applied_amount.as_proto(),
                applied_count: value.applied_count,
                last_applied_at: value.last_applied_at.as_proto(),
                created_at: value.created_at.as_proto(),
            })
        }
    }
}
pub mod coupons {
    use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::coupons::v1 as server;
    use meteroid_store::domain;

    pub struct CouponWrapper(pub server::Coupon);
    impl From<domain::coupons::Coupon> for CouponWrapper {
        fn from(value: domain::coupons::Coupon) -> Self {
            Self(server::Coupon {
                id: value.id.as_proto(),
                local_id: value.id.as_proto(), //todo remove me
                description: value.description,
                code: value.code,
                discount: Some(discount::to_server(&value.discount)),
                expires_at: value.expires_at.as_proto(),
                redemption_limit: value.redemption_limit,
                created_at: value.created_at.as_proto(),
                disabled: value.disabled,
                last_redemption_at: value.last_redemption_at.as_proto(),
                redemption_count: value.redemption_count as u32,
                archived_at: value.archived_at.as_proto(),
                recurring_value: value.recurring_value,
                reusable: value.reusable,
                plan_ids: value.plan_ids.into_iter().map(|id| id.as_proto()).collect(),
            })
        }
    }

    pub fn to_server(value: domain::coupons::Coupon) -> server::Coupon {
        CouponWrapper::from(value).0
    }

    pub mod filter {
        use meteroid_grpc::meteroid::api::coupons::v1::list_coupon_request::CouponFilter as ServerCouponFilter;
        use meteroid_store::domain::coupons::CouponFilter;

        pub fn from_server(value: ServerCouponFilter) -> CouponFilter {
            match value {
                ServerCouponFilter::All => CouponFilter::ALL,
                ServerCouponFilter::Active => CouponFilter::ACTIVE,
                ServerCouponFilter::Inactive => CouponFilter::INACTIVE,
                ServerCouponFilter::Archived => CouponFilter::ARCHIVED,
            }
        }
    }

    pub mod discount {
        use crate::api::shared::conversions::ProtoConv;
        use meteroid_grpc::meteroid::api::coupons::v1 as server;
        use meteroid_store::domain;
        use rust_decimal::Decimal;
        use tonic::Status;

        pub fn to_server(value: &domain::coupons::CouponDiscount) -> server::CouponDiscount {
            match value {
                domain::coupons::CouponDiscount::Percentage(value) => server::CouponDiscount {
                    discount_type: Some(server::coupon_discount::DiscountType::Percentage(
                        server::coupon_discount::PercentageDiscount {
                            percentage: value.as_proto(),
                        },
                    )),
                },
                domain::coupons::CouponDiscount::Fixed { currency, amount } => {
                    server::CouponDiscount {
                        discount_type: Some(server::coupon_discount::DiscountType::Fixed(
                            server::coupon_discount::FixedDiscount {
                                currency: currency.clone(),
                                amount: amount.as_proto(),
                            },
                        )),
                    }
                }
            }
        }

        pub fn to_domain(
            value: Option<server::CouponDiscount>,
        ) -> Result<domain::coupons::CouponDiscount, Status> {
            match value.as_ref().and_then(|x| x.discount_type.as_ref()) {
                Some(server::coupon_discount::DiscountType::Percentage(value)) => {
                    Ok(domain::coupons::CouponDiscount::Percentage(
                        Decimal::from_proto_ref(&value.percentage)?,
                    ))
                }
                Some(server::coupon_discount::DiscountType::Fixed(value)) => {
                    Ok(domain::coupons::CouponDiscount::Fixed {
                        currency: value.currency.clone(),
                        amount: Decimal::from_proto_ref(&value.amount)?,
                    })
                }
                None => Err(Status::invalid_argument("discount_type is missing")),
            }
        }
    }
}

pub mod coupons {
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;
    use meteroid_grpc::meteroid::api::coupons::v1 as server;
    use meteroid_store::domain;

    pub struct CouponWrapper(pub server::Coupon);
    impl From<domain::coupons::Coupon> for CouponWrapper {
        fn from(value: domain::coupons::Coupon) -> Self {
            Self(server::Coupon {
                id: value.id.to_string(),
                description: value.description.into(),
                code: value.code.into(),
                discount: Some(discount::to_server(value.discount)),
                expires_at: value.expires_at.map(chrono_to_timestamp),
                redemption_limit: value.redemption_limit,
            })
        }
    }

    pub mod discount {
        use crate::api::shared::conversions::ProtoConv;
        use meteroid_grpc::meteroid::api::coupons::v1 as server;
        use meteroid_store::domain;
        use rust_decimal::Decimal;
        use tonic::Status;

        pub fn to_server(value: domain::coupons::CouponDiscount) -> server::CouponDiscount {
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
                                currency: currency.into(),
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

use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CouponId, TenantId};
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::coupons::CouponStatusPatch;
use meteroid_store::repositories::coupons::CouponInterface;

#[tokio::test]
async fn test_coupons_basic() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let fixed_discount = api::coupons::v1::CouponDiscount {
        discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Fixed(
            api::coupons::v1::coupon_discount::FixedDiscount {
                amount: "20.23".into(),
                currency: "USD".into(),
            },
        )),
    };

    let percentage_discount = api::coupons::v1::CouponDiscount {
        discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Percentage(
            api::coupons::v1::coupon_discount::PercentageDiscount {
                percentage: "14.01".into(),
            },
        )),
    };

    // create coupon
    let created = clients
        .coupons
        .clone()
        .create_coupon(api::coupons::v1::CreateCouponRequest {
            code: "test-code".into(),
            description: "test-desc".into(),
            discount: Some(fixed_discount.clone()),
            expires_at: None,
            redemption_limit: Some(10),
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .coupon
        .unwrap();

    assert_eq!(created.code.as_str(), "test-code");
    assert_eq!(created.description.as_str(), "test-desc");
    assert_eq!(created.discount.as_ref(), Some(&fixed_discount));
    assert_eq!(created.expires_at, None);
    assert_eq!(created.redemption_limit, Some(10));

    // list coupons
    let coupons = clients
        .coupons
        .clone()
        .list_coupons(api::coupons::v1::ListCouponRequest {
            search: None,
            pagination: None,
            filter: api::coupons::v1::list_coupon_request::CouponFilter::All as i32,
            order_by: None,
        })
        .await
        .unwrap()
        .into_inner()
        .coupons;

    assert_eq!(coupons.len(), 1);
    assert_eq!(coupons.first(), Some(&created));

    // edit coupon
    let edited = clients
        .coupons
        .clone()
        .edit_coupon(api::coupons::v1::EditCouponRequest {
            coupon_id: created.id.clone(),
            description: "test-desc-edited".into(),
            discount: Some(percentage_discount.clone()),
            plan_ids: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .coupon
        .unwrap();

    assert_eq!(edited.code.as_str(), "test-code");
    assert_eq!(edited.description.as_str(), "test-desc-edited");
    assert_eq!(edited.discount.as_ref(), Some(&percentage_discount));
    assert_eq!(edited.expires_at, None);
    assert_eq!(edited.redemption_limit, Some(10));

    // delete coupon
    let _ = clients
        .coupons
        .clone()
        .remove_coupon(api::coupons::v1::RemoveCouponRequest {
            coupon_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    let coupons = clients
        .coupons
        .clone()
        .list_coupons(api::coupons::v1::ListCouponRequest {
            search: None,
            pagination: None,
            filter: api::coupons::v1::list_coupon_request::CouponFilter::All as i32,
            order_by: None,
        })
        .await
        .unwrap()
        .into_inner()
        .coupons;

    assert_eq!(coupons.len(), 0);
}

/// A coupon must only be archivable or disable-able by the tenant that owns it.
///
/// `CouponStatusRowPatch` declares `tenant_id` as part of its primary key, so AsChangeset omits
/// it from the statement; without an explicit predicate the UPDATE matched by id alone.
#[tokio::test]
async fn test_coupon_status_update_is_tenant_scoped() {
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let created = clients
        .coupons
        .clone()
        .create_coupon(api::coupons::v1::CreateCouponRequest {
            code: "cross-tenant-code".into(),
            description: "cross-tenant-desc".into(),
            discount: Some(api::coupons::v1::CouponDiscount {
                discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Percentage(
                    api::coupons::v1::coupon_discount::PercentageDiscount {
                        percentage: "10.00".into(),
                    },
                )),
            }),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .coupon
        .unwrap();

    let coupon_id = CouponId::parse_base62(created.id.as_str()).unwrap();

    let result = setup
        .store
        .update_coupon_status(
            Actor::System,
            CouponStatusPatch {
                id: coupon_id,
                tenant_id: TenantId::new(),
                archived_at: None,
                disabled: Some(true),
            },
        )
        .await;

    assert!(
        result.is_err(),
        "a foreign tenant disabled a coupon it does not own"
    );
}

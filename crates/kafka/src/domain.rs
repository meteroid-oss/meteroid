pub mod subscription {
    use common_domain::{CustomerId, PlanId, PricePointId, SubscriptionId, TenantId};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct Subscription {
        pub id: SubscriptionId,
        pub tenant_id: TenantId,
        pub customer_id: CustomerId,
        pub plan_id: PlanId,
        pub price_point_id: PricePointId,
        pub start_date: chrono::NaiveDate,
        pub billing_day: i32,
    }
}

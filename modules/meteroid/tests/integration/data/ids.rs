#![allow(unused)]

use common_domain::ids::*;
use uuid::{Uuid, uuid};

// Basic information
pub const ORGANIZATION_ID: OrganizationId =
    OrganizationId::from_const(uuid!("018c2c82-3def-7fa0-bf6f-a5f8fe341549"));
pub const USER_ID: Uuid = uuid!("ae35bbb9-65da-477d-b856-7dbd87546441");
pub const API_TOKEN_ID: Uuid = uuid!("018ce957-b628-7355-a460-f0d71e01335e");

pub const TENANT_ID: TenantId = TenantId::from_const(uuid!("018c2c82-3df1-7e84-9e05-6e141d0e751a"));
pub const INVOICING_ENTITY_ID: InvoicingEntityId =
    InvoicingEntityId::from_const(uuid!("cf144094-ab72-441c-8c8a-54e18bfba0ef"));
pub const PRODUCT_FAMILY_ID: ProductFamilyId =
    ProductFamilyId::from_const(uuid!("018c2c82-3df2-71a4-b45c-86cb8604b75c"));

// Customers
pub const CUST_SPOTIFY_ID: CustomerId =
    CustomerId::from_const(uuid!("018c345f-7324-7cd2-a692-78e5ab9158e0"));
pub const CUST_UBER_ID: CustomerId =
    CustomerId::from_const(uuid!("018c345f-dff1-7857-b988-6c792ed6fa3f"));
pub const CUST_COMODO_ID: CustomerId =
    CustomerId::from_const(uuid!("018c3463-05f3-7c1f-92b1-ddb1f70905a2"));

// Metrics
pub const METRIC_DATABASE_SIZE: BillableMetricId =
    BillableMetricId::from_const(uuid!("018c3452-129f-702c-93f4-9c15095b0ef4"));
pub const METRIC_BANDWIDTH: BillableMetricId =
    BillableMetricId::from_const(uuid!("018c3453-1f11-76a8-8d69-f74921b2646d"));

// Plans
pub const PLAN_LEETCODE_ID: PlanId =
    PlanId::from_const(uuid!("018c344a-78a8-79bc-aefd-09113eaf5cb3"));
pub const PLAN_NOTION_ID: PlanId =
    PlanId::from_const(uuid!("018c344b-da85-70dc-ae6f-5b919847dbbf"));
pub const PLAN_SUPABASE_ID: PlanId =
    PlanId::from_const(uuid!("018c344d-5957-72cf-816b-938dea2f5c76"));

// Plan versions
pub const PLAN_VERSION_1_LEETCODE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("018c344a-78a9-7e2b-af90-5748672711f8"));
pub const PLAN_VERSION_2_LEETCODE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("018c344a-78a9-7e2b-af90-5748672711f9"));
pub const PLAN_VERSION_NOTION_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("018c344b-da87-7392-bbae-c5c8780adb1b"));
pub const PLAN_VERSION_SUPABASE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("018c35cc-3f41-7551-b7b6-f8bbcd62b784"));

// Components
pub const COMP_LEETCODE_RATE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("018c344b-6050-7ec8-bd8c-d2e9c41ab711"));
pub const COMP_NOTION_SEATS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("018c344c-9ec9-7608-b115-1537b6985e73"));
pub const COMP_SUPABASE_ORG_SLOTS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("3b083801-c77c-4488-848e-a185f0f0a8be"));
pub const COMP_SUPABASE_BANDWIDTH_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("705265c8-6069-4b84-a815-73bc7bd773bd"));
pub const COMP_SUPABASE_DB_SIZE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("331810d4-05b1-4d8e-bf9b-d61cedaec117"));

// Trial-related plans
pub const PLAN_FREE_ID: PlanId = PlanId::from_const(uuid!("019438e0-0001-7000-8000-000000000001"));
pub const PLAN_VERSION_FREE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0002-7000-8000-000000000001"));
pub const PLAN_PRO_WITH_TRIAL_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0003-7000-8000-000000000001"));
pub const PLAN_VERSION_PRO_WITH_TRIAL_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0004-7000-8000-000000000001"));
pub const PLAN_ENTERPRISE_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0005-7000-8000-000000000001"));
pub const PLAN_VERSION_ENTERPRISE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0006-7000-8000-000000000001"));

// Paid plan with free trial (Standard type, trial_is_free = true)
// After trial: TrialExpired if no payment method, or Active with billing
pub const PLAN_PAID_FREE_TRIAL_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0007-7000-8000-000000000001"));
pub const PLAN_VERSION_PAID_FREE_TRIAL_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0008-7000-8000-000000000001"));
pub const COMP_PAID_FREE_TRIAL_RATE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0009-7000-8000-000000000001"));

// Paid plan with paid trial (Standard type, trial_is_free = false)
// Bills immediately but gives trialing_plan features
pub const PLAN_PAID_TRIAL_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-000a-7000-8000-000000000001"));
pub const PLAN_VERSION_PAID_TRIAL_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-000b-7000-8000-000000000001"));
pub const COMP_PAID_TRIAL_RATE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-000c-7000-8000-000000000001"));

// Subscriptions
pub const SUB_SPOTIFY_NOTION_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c3475-bdc5-77dd-9e26-e9a7fdd60426"));
pub const SUB_SPOTIFY_SUPABASE_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c3762-d554-7339-b13d-6fff8c9b76a0"));
pub const SUB_UBER_NOTION_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c3477-2274-7029-9743-b3a4eb779399"));
pub const SUB_UBER_LEETCODE_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c3479-fa9d-713f-b74f-6d9cc22cf110"));
pub const SUB_COMODO_LEETCODE_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c347a-b42b-709f-8e70-b0b63029aa35"));
pub const SUB_COMODO_SUPABASE_ID: SubscriptionId =
    SubscriptionId::from_const(uuid!("018c3763-070e-709d-8413-f42828e71943"));
